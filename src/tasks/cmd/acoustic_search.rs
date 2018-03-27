
use super::lammps::{LammpsBuilder, LammpsExt};
use super::SupercellSpecExt;

use ::errors::{Result, ok};
use ::rsp2_tasks_config as cfg;

use ::math::basis::Basis3;

use ::rsp2_slice_math::{v, V, vdot, vnormalize, BadNorm};

use ::slice_of_array::prelude::*;
use ::rsp2_structure::supercell;
use ::rsp2_structure::{ElementStructure};
use ::util::tup3;

use std::fmt;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ModeKind {
    /// Uniform translations of the entire structure.
    ///
    /// Any given structure has three.
    /// (*technically*, fewer may be found if there are multiple
    ///  non-interacting parts in the structure; but currently,
    ///  the acoustic searcher explicitly does not support such
    ///  structures, because it has no strategy for identifying
    ///  piecewise translational modes)
    Translational,

    /// A mode where the force is not only at a zero, but also
    /// at an inflection point.
    ///
    /// There are at most three, depending on the dimensionality of
    /// the structure.
    Rotational,

    /// An imaginary mode that is not acoustic! Bad!
    Imaginary,

    Vibrational,
}

pub struct Colorful(pub ModeKind);

impl fmt::Display for ModeKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match *self {
            ModeKind::Translational => "T",
            ModeKind::Rotational    => "R",
            ModeKind::Imaginary     => "‼",
            ModeKind::Vibrational   => "-",
        })
    }
}

impl fmt::Display for Colorful {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let color = match self.0 {
            ModeKind::Translational => ::ansi_term::Colour::Yellow.bold(),
            ModeKind::Rotational    => ::ansi_term::Colour::Purple.bold(),
            ModeKind::Imaginary     => ::ansi_term::Colour::Red.bold(),
            ModeKind::Vibrational   => ::ansi_term::Colour::White.normal(),
        };
        write!(f, "{}", ::ui::color::gpaint(color, self.0))
    }
}

pub(crate) fn perform_acoustic_search(
    lmp: &LammpsBuilder,
    eigenvalues: &[f64],
    eigenvectors: &Basis3,
    structure: &ElementStructure,
    settings: &cfg::Settings,
) -> Result<Vec<ModeKind>>
{ok({
    let &cfg::Settings {
        acoustic_search: cfg::AcousticSearch {
            displacement_distance,
            rotational_fdot_threshold,
            imaginary_fdot_threshold,
        },
        potential: cfg::Potential {
            // I feel like this function shouldn't have to care about this...
            supercell: ref supercell_spec,
            ..
        },
        ..
    } = settings;


    let zero_index = eigenvalues.iter().position(|&x| x >=  0.0).unwrap_or(eigenvalues.len());

    let mut kinds = vec![None; eigenvalues.len()];

    { // quickly spot translational modes

        // We want to search a little bit past the negative freqs, but not *too* far.
        // Surely, the frequencies of the acoustic modes must be less than this:
        let stop_index = eigenvalues.iter().position(|&x| x >= 10.0).unwrap_or(eigenvalues.len());

        let mut t_end = zero_index;
        for (i, ket) in eigenvectors.0.iter().take(stop_index).enumerate() {
            if ket.acousticness() >= 0.95 {
                t_end = i + 1;
                kinds[i] = Some(ModeKind::Translational);
            }
        }

        // if there's more than three then the eigenbasis clearly isn't even orthonormal
        ensure!(
            kinds.iter().filter(|&x| x == &Some(ModeKind::Translational)).count() <= 3,
            "More than 3 pure translational modes! These eigenvectors are garbage!");

        // Everything after the last translational or negative mode is vibrational.
        kinds.truncate(t_end);
        kinds.resize(eigenvalues.len(), Some(ModeKind::Vibrational));
    }

    { // look at the negative eigenvectors for rotations and true imaginary modes
        let sc_dims = tup3(supercell_spec.dim_for_unitcell(structure.lattice()));
        let (superstructure, sc_token) = supercell::diagonal(sc_dims, structure.clone());
        let mut lmp = lmp.with_modified_inner(|b| b.threaded(true)).build(superstructure.clone())?;
        let mut diff_at_pos = lmp.flat_diff_fn();

        let pos_0 = superstructure.to_carts();
        let grad_0 = diff_at_pos(pos_0.flat())?.1;

        for (i, ket) in eigenvectors.0.iter().take(zero_index).enumerate() {
            if kinds[i].is_some() {
                continue;
            }

            let direction = sc_token.replicate(ket.as_real_checked());
            let V(pos_l) = v(pos_0.flat()) - displacement_distance * v(direction.flat());
            let V(pos_r) = v(pos_0.flat()) + displacement_distance * v(direction.flat());
            let grad_l = diff_at_pos(&pos_l[..])?.1;
            let grad_r = diff_at_pos(&pos_r[..])?.1;

            let mut d_grad_l = v(&grad_0) - v(grad_l);
            let mut d_grad_r = v(grad_r) - v(&grad_0);

            // for rotational modes, the two d_grads should be anti-parallel.
            // for true imaginary modes, the two d_grads are.... uh, "pro-parallel".
            // for non-pure translational modes, the two d_grads could be just noise
            //   (they should be zero, but we're about to normalize them)
            //   which means they could also masquerade as one of the other types.
            for d_grad in vec![&mut d_grad_l, &mut d_grad_r] {
                *d_grad = match vnormalize(&*d_grad) {
                    Err(BadNorm(_)) => {
                        // use a zero vector; it'll be classified as suspicious
                        d_grad.clone()
                    },
                    Ok(v) => v,
                };
            }

            kinds[i] = Some({
                match vdot(&d_grad_l, &d_grad_r) {
                    dot if dot < -1.001 || 1.001 < dot => panic!("bad unit vector dot"),
                    dot if dot <= -rotational_fdot_threshold => ModeKind::Rotational,
                    dot if imaginary_fdot_threshold <= dot => ModeKind::Imaginary,
                    dot => {
                        // This mode *could* be piecewise translational, which we don't support.
                        warn!(
                            "Could not classify mode at frequency {} (fdot = {:.6})! \
                            Assuming it is imaginary.", eigenvalues[i], dot,
                        );
                        ModeKind::Imaginary
                    },
                }
            });
        }
    }

    kinds.into_iter()
        .map(|opt| opt.expect("bug! every index should have been accounted for"))
        .collect()
})}
