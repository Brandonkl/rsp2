/* ************************************************************************ **
** This file is part of rsp2, and is licensed under EITHER the MIT license  **
** or the Apache 2.0 license, at your option.                               **
**                                                                          **
**     http://www.apache.org/licenses/LICENSE-2.0                           **
**     http://opensource.org/licenses/MIT                                   **
**                                                                          **
** Be aware that not all of rsp2 is provided under this permissive license, **
** and that the project as a whole is licensed under the GPL 3.0.           **
** ************************************************************************ */

//! Computes raman intensities of gamma eigenkets using
//! a bond polarizability model.
//!
//! Adapted from the sp2 code.

use crate::FailResult;
use crate::math::basis::Basis3;
use crate::meta::{Element, Mass};
use enum_map::EnumMap;
use rsp2_array_types::{dot, V3, M33};
use rsp2_structure::bonds::{CartBond, CartBonds};

pub struct PolConstant {
    /// `a_par  -   a_perp`
    pub c1: f64,
    /// `a'_par -   a'_perp`
    pub c2: f64,
    /// `a'_par + 2 a'_perp`
    pub c3: f64,
    /// maximum bond length
    pub max_len: f64,
}

// NOTE: there are also constant factors out front based on input light frequency
//       and stuff, so this only gives proportional intensities
fn raman_prefactor(
    mode_frequency: f64,
    temperature: f64,
) -> f64 {
    // (hbar / k_b) in [K] per [cm-1]
    let hk = 0.22898852319;

    let expm1 = f64::exp_m1(hk * mode_frequency / temperature);
    if expm1 == 0.0 {
        // this would happen if the mode_frequency was exactly zero,
        // but acoustic modes are obviously not raman active.
        0.0
    } else {
        let bose_occupation = 1.0 + 1.0 / expm1;
        bose_occupation / mode_frequency
    }
}

#[derive(enum_map::Enum)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BondType { CC, CH, HH }

impl BondType {
    fn from_elements(a: Element, b: Element) -> FailResult<BondType> {
        Ok(match (a, b) {
            (Element::CARBON, Element::CARBON) => BondType::CC,
            (Element::CARBON, Element::HYDROGEN) => BondType::CH,
            (Element::HYDROGEN, Element::CARBON) => BondType::CH,
            (Element::HYDROGEN, Element::HYDROGEN) => BondType::HH,
            _ => bail!{
                "No polarization constants specified for bonds between {} and {}",
                a.symbol(), b.symbol(),
            },
        })
    }
}

pub type PolConstants = EnumMap<BondType, Option<PolConstant>>;
#[allow(bad_style)]
pub fn default_CH_pol_constants() -> PolConstants {
    enum_map!{
        BondType::CC => Some(PolConstant {
            c1: 0.32, c2: 2.60, c3: 7.55,
            max_len: 1.6,
        }),
        BondType::CH => Some(PolConstant {
            c1: 0.32, c2: 2.60, c3: 7.55,
            max_len: 1.3,
        }),
        _ => None,
    }
}

#[allow(bad_style)]
#[allow(unused)] // FIXME
pub fn nanotube_CC_pol_constants() -> PolConstants {
    enum_map!{
        BondType::CC => Some(PolConstant {
            c1: 0.04, c2: 4.0, c3: 4.7,
            max_len: 1.6,
        }),
        _ => None,
    }
}

pub struct RamanTensor {
    prefactor: f64,
    tensor: M33,
}

impl RamanTensor {
    pub fn tensor(&self) -> M33 { self.tensor * self.prefactor }
    pub fn integrate_intensity(
        &self,
        light_polarization: &LightPolarization,
    ) -> f64 {
        let RamanTensor { ref tensor, prefactor } = *self;

        // there was probably an easier way to do this, or a simple proof, given
        // the extremely simple answer
        //
        // random unit vectors in 3D can be generated by generating gaussian x, y, z
        //     v = (x, y, z) / sqrt(x^2 + y^2 + z^2)
        //       = (cos(phi) sin(theta), sin(phi) sin(theta), cos(theta))
        //
        // since we want the average of v^T (polarization tensor) v, we need to
        // find the expectation values for the matrix
        //
        //     1       (x1x2 a + x1y2 b + x1z2 c +
        // --------- *  y1x2 d + y1y2 e + y1z2 f +
        // (r1 r2)^2    z1x2 g + z1y2 h + z1z2 i)^2
        //
        // which ends up looking like the integral of
        //  = (elem)  e^(-(r1^2 + r2^2)/2) / (r1 r2)^2
        //  = (1/sqrt(2 pi))^6 * (elem / (r1 r2)^2) *  e^(-(r1^2 + r2^2)/2)
        //  = (1/sqrt(2 pi))^6 * g(theta1, phi1, theta2, phi2) * f(r1, r2)
        //
        // using [integral 0 to inf of (r^2 e^(-r^2/2) dr) = sqrt(pi / 2)] what
        // we have left is the integral of
        //  = 1 / (16 pi^2) (elem / (r1 r2)^2)
        //
        // so essentially all we need to integrate is
        //    [cos^2(phi) sin^2(theta), xx
        //     sin^2(phi) sin^2(theta), yy
        //     cos(phi) sin(phi) sin^2(theta), xy
        //     cos^2(theta), zz
        //     sin(phi) cos(theta) sin(theta), zy
        //     cos(phi) cos(theta) sin(theta)] zx
        //      * sin(theta) dtheta dphi
        //
        //     xx = 4 pi / 3
        //     yy = 4 pi / 3
        //     zz = 4 pi / 3
        //
        //     yx = 0
        //     yz = 0
        //     xz = 0
        //
        //     = [4/3 pi, 4/3 pi, 4/3 pi]
        //
        // back to the original equation we get
        //  = (4/3 pi)^2 / (16 pi^2)
        //  = 1 / 9
        //
        // for 2D (backscattering) its just:
        //  = 1 / (4 pi^2) (pi^2)
        //  = 1 / 4
        //

        let sq_sum_submatrix = |range: std::ops::Range<usize>| {
            let mut sum = 0.0;
            for i in range.clone() {
                for j in range.clone() {
                    sum += tensor[i][j] * tensor[i][j];
                }
            }
            sum
        };

        let value = match light_polarization {
            LightPolarization::Polarized { incident, scattered } => {
                let sum = dot(incident, &(tensor * scattered));
                sum * sum
            },
            LightPolarization::Average => sq_sum_submatrix(0..3) / 9.0,
            LightPolarization::BackscatterZ => sq_sum_submatrix(0..2) / 4.0,
        };

        prefactor * value
    }
}

/// NOTE: Matrix is column-based.
fn raman_tensor(
    eigenvector: &[V3],
    masses: &[Mass],
    bonds: &CartBonds,
    types: &[Element],
    pol_constants: &PolConstants,
) -> FailResult<M33> {
    // kronecker delta value
    let kdelta = <M33>::eye();

    let mut tensor = M33::zero();
    let mut ignored_count = 0;
    let mut ignored_distance = 0.0_f64;
    for CartBond { from, to, cart_vector: bond_vector } in bonds {
        let bond_type = BondType::from_elements(types[from], types[to])?;

        // phonon eigenvector for this atom, need to mass normalize
        let eig: V3 = eigenvector[from] / f64::sqrt(masses[from].0);

        // unit bond vector and length, used later
        let distance: f64 = bond_vector.norm();
        let rhat: V3 = bond_vector / distance;

        let pc = match &pol_constants[bond_type] {
            Some(pc) => pc,
            // ignore bonds which have no corresponding polarization constants
            None => continue,
        };

        // check if bond is actually valid (via length)
        if distance > pc.max_len {
            ignored_count += 1;
            ignored_distance = ignored_distance.max(distance);
        }

        let const_one  = pc.c1; // `a_par  -   a_perp`
        let dconst_one = pc.c2; // `a'_par -   a'_perp`
        let dconst_two = pc.c3; // `a'_par + 2 a'_perp`

        tensor += &M33::from_fn(|r, c| {
            dot(&rhat, &eig) * (
                (dconst_two / 3.0) * kdelta[r][c]
                    + dconst_one * (rhat[r] * rhat[c] - kdelta[r][c] / 3.0)
            ) + (const_one / distance) * (
                (rhat[r] * eig[c] + rhat[c] * eig[r])
                    - 2.0 * rhat[r] * rhat[c] * dot(&rhat, &eig)
            )
        });
    } // for Bond { ... } in bonds

    if ignored_count > 0 {
        warn_once!(
            "{} out of {} bonds were ignored because they were too long! \
            (max length {})",
            ignored_count, bonds.len(),
            ignored_distance,
        );
    }
    Ok(tensor)
}

pub enum LightPolarization {
    // previously:  avg = false, backscatter = (ignored)
    #[allow(unused)]
    Polarized {
        incident: V3,
        scattered: V3,
    },
    // previously:  avg = true, backscatter = false,
    Average,
    // previously:  avg = true, backscatter = true,
    BackscatterZ,
}

/// Quick little struct to simulate named arguments
pub struct Input<'a> {
    pub temperature: f64,
    pub ev_frequencies: &'a [f64],
    pub ev_eigenvectors: &'a Basis3,
    pub site_elements: &'a [Element],
    pub site_masses: &'a [Mass],
    pub bonds: &'a CartBonds,
}

impl<'a> Input<'a> {
    pub fn compute_ev_raman_tensors(self) -> FailResult<Vec<RamanTensor>> {
        let Input {
            ev_frequencies, ev_eigenvectors,
            temperature, site_elements, site_masses, bonds,
        } = self;

        let pol_constants = default_CH_pol_constants();

        zip_eq!(ev_frequencies, &ev_eigenvectors.0)
            .map(|(&frequency, eigs)| {
                let prefactor = raman_prefactor(frequency, temperature);
                let tensor = raman_tensor(
                    eigs.as_real_checked(),
                    site_masses,
                    bonds,
                    site_elements,
                    &pol_constants,
                )?;
                Ok(RamanTensor { prefactor, tensor })
            }).collect()
    }
}
