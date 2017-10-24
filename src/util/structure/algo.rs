pub(crate) mod layer {
    use ::Result;
    use ::{Structure, Lattice};

    use ::std::mem;
    use ::itertools::Itertools;
    use ::ordered_float::NotNaN;

    /// Newtype wrapper for a layer number.
    ///
    /// They are numbered from 0.
    #[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Hash, Eq, Ord)]
    pub struct Layer(pub u32);

    // FIXME this is wrong wrong wrong.
    //       Only correct when the other two lattice vectors are
    //         perpendicular to the chosen lattice vector.
    //       May need to rethink the API.
    //
    /// Determine layers in a structure, numbered from zero.
    /// Also returns the count.
    ///
    /// This finds "layers" defined as groups of atoms isolated from
    /// other groups by at least some minimum distance projected along
    /// a normal vector.
    ///
    /// Normal is in fractional coords, and is currently limited such
    /// that it must be one of the lattice vectors.
    pub fn assign_layers<M>(structure: &Structure<M>, normal: &[i32; 3], sep: f64)
    -> Result<(Vec<Layer>, u32)>
    {
        assign_layers_impl(
            &structure.to_fracs(),
            structure.lattice(),
            normal,
            sep,
        )
    }

    // monomorphic for less codegen
    fn assign_layers_impl(fracs: &[[f64; 3]], lattice: &Lattice, normal: &[i32; 3], sep: f64)
    -> Result<(Vec<Layer>, u32)>
    {Ok({
        if fracs.len() == 0 {
            return Ok((vec![], 0));
        }

        let axis = {
            let mut sorted = *normal;
            sorted.sort_unstable();
            ensure!(sorted == [0, 0, 1],
                "unsupported layer normal: {:?}", normal);

            normal.iter().position(|&x| x == 1).unwrap()
        };
        let reduce = |x: f64| (x.fract() + 1.0).fract();

        let sorted: Vec<(usize, f64)> = {
            let mut vec: Vec<_> = fracs.iter()
                .map(|f| reduce(f[axis]))
                .enumerate().collect();

            vec.sort_by_key(|&(_, x)| NotNaN::new(x).unwrap());
            vec
        };

        let frac_sep = sep / lattice.lengths()[axis];

        // FIXME: On second thought I think this is incorrect.
        //        Our requirement should not be that the normal is a
        //        lattice vector; but rather, that two of the lattice
        //        vectors lie within the plane.

        { // Safety HACK!
            use ::rsp2_array_utils::dot;
            let lengths = lattice.lengths();
            let vecs = lattice.matrix();
            for k in 0..3 {
                if k != axis {
                    let cos = dot(&vecs[k], &vecs[axis]) / (lengths[k] * lengths[axis]);
                    ensure!(cos.abs() < 1e-7,
                        "For your safety, assign_layers is currently limited to \
                        lattices where the normal is perpendicular to the other two \
                        lattice vectors.");
                }
            }
        }

        // --(original (incorrect) text)--
        // NOTE: the validity of the following algorithm is
        //       predicated on the normal pointing precisely along
        //       a lattice vector.  This ensures that there's no
        //       funny business where the projected distance along the
        //       axis could suddenly change as a particle crosses a
        //       periodic surface while traveling within a layer.
        //
        //       Some other directions with integer coordinates
        //       could be handled in the future by a unimodular
        //       transform to make that direction become one of the
        //       lattice vectors....In theory.
        // --(end original text)--

        // Split the positions into contiguous segments of atoms
        // where the distance between any two consecutive atoms
        // (projected onto the normal vector) is at most `sep`.
        let mut groups = vec![];
        let mut cur_group = vec![sorted[0].0];
        for (&(_, ax), &(bi, bx)) in sorted.iter().tuple_windows() {
            // Invariant
            assert!(cur_group.len() >= 1);

            if bx - ax > frac_sep {
                let done = mem::replace(&mut cur_group, vec![]);
                groups.push(done);
            }
            cur_group.push(bi);
        }

        // Detect a layer crossing the periodic plane:
        // If the first and last "layers" are close enough,
        // join them together.
        let wrap_distance = sorted[0].1 - (sorted.last().unwrap().1 - 1.0);
        if wrap_distance <= frac_sep && !groups.is_empty() {
            groups[0].extend(cur_group); // join them
        } else {
            groups.push(cur_group); // keep them separate
        }

        // go from vecs of indices to vec of layers
        let n_layers = groups.len();
        let mut out = vec![Layer(0xf00d); fracs.len()];
        for (layer, group) in groups.into_iter().enumerate() {
            for i in group {
                out[i] = Layer(layer as u32);
            }
        }
        (out, n_layers as u32)
    })}

    #[cfg(test)]
    #[deny(unused)]
    mod tests {
        use super::Layer;
        use ::Lattice;
        use ::util::perm::{self, Permute, Perm};

        #[test]
        fn assign_layers_impl() {
            let go = super::assign_layers_impl;

            let fracs = vec![
                // we will be looking along y with frac_tol = 0.11
                [0.0, 0.1, 0.0],
                [0.0, 0.2, 0.0], // obviously same layer
                [0.8, 0.3, 0.4], // laterally displaced, but still same layer
                [0.0, 0.7, 0.0], // a second layer
                [0.0, 0.8, 0.0],
                // (first and last not close enough to meet)
            ];

            // NOTE this does try a non-diagonal lattice and even
            //      goes along an awkwardly oriented vector
            //       but we restrict it to a form that the
            //       (currently broken) algorithm will work on
            //      (1st and 3rd vecs must be orthogonal to 2nd vec)
            let ylen = 4.0;
            let cart_tol    = 0.11 * ylen;  // produces 2 layers
            let smaller_tol = 0.09 * ylen;  // makes all atoms look separate

            const IR2: f64 = ::std::f64::consts::FRAC_1_SQRT_2;
            let lattice = Lattice::new(&[
                [ ylen * IR2, ylen *  IR2,  0.0],
                [ ylen * IR2, ylen * -IR2,  0.0], // (axis we're using)
                [        0.0,         0.0, 13.0],
            ]);

            let layers = |xs: Vec<_>| xs.into_iter().map(Layer).collect();

            assert_eq!(
                go(&fracs, &lattice, &[0, 1, 0], cart_tol).unwrap(),
                (layers(vec![0, 0, 0, 1, 1]), 2));

            // put them out of order
            let (fracs, perm) = perm::shuffle(&fracs);
            assert_eq!(
                go(&fracs, &lattice, &[0, 1, 0], cart_tol).unwrap().0,
                layers(vec![0, 0, 0, 1, 1]).permuted_by(&perm));

            // try a smaller tolerance
            assert_eq!(
                go(&fracs, &lattice, &[0, 1, 0], smaller_tol).unwrap().0,
                layers(vec![0, 1, 2, 3, 4]).permuted_by(&perm));

            // try bridging across the periodic boundary to
            //   join the two layers.
            // also, try a position outside the unit cell.
            let (mut fracs, mut perm) = (fracs, perm);
            fracs.push([0.0, 1.9, 0.0]);
            fracs.push([0.0, 0.0, 0.0]);
            perm.append_mut(&Perm::eye(2));

            assert_eq!(
                go(&fracs, &lattice, &[0, 1, 0], cart_tol).unwrap().0,
                layers(vec![0, 0, 0, 0, 0, 0, 0]).permuted_by(&perm));

            // try joining the end regions when there is more than one layer
            // (this might use a different codepath for some implementations)
            fracs.push([0.0, 0.5, 0.0]);
            perm.append_mut(&Perm::eye(1));

            assert_eq!(
                go(&fracs, &lattice, &[0, 1, 0], cart_tol).unwrap().0,
                layers(vec![0, 0, 0, 0, 0, 0, 0, 1]).permuted_by(&perm));
        }
    }
}

#[allow(unused)]
pub(crate) mod group {
    use ::errors::*;
    use ::std::hash::Hash;
    use ::std::result::Result as StdResult;

    /// Tree representation of a finite group, with generators as leaves.
    pub(crate) struct GroupTree<G> {
        members: Vec<G>,
        decomps: Vec<Option<(usize, usize)>>,
    }

    impl<G> GroupTree<G>
    {
        /// Constructs a `GroupTree<G>` given a sequence that contains
        /// each member of a finite group exactly once.
        ///
        /// A `GroupTree<G>` constructed in this manner is guaranteed to
        /// order its elements in the same order as the input `Vec`.
        ///
        /// Arguments of the closure should follow the convention of function
        /// composition; `of(a, b)` should compute "`a` of `b`".
        pub fn from_all_members<GFn>(members: Vec<G>, mut of: GFn) -> Self
        where
            G: Hash + Eq + Clone,
            GFn: FnMut(&G, &G) -> G,
        {
            assert!(members.len() > 0, "empty groups do not exist!");

            let indices: ::std::collections::HashMap<G, usize> =
                members.iter().cloned()
                .enumerate().map(|(i, x)| (x, i))
                .collect();

            // Brute force O(G)^2 attempt to fill the tree.
            // I'm fairly certain this can be improved in some way by using
            // known element-inverse pairs to quickly find new members,
            // but I don't think it's worth it since this will probably only ever
            // be used on spacegroups, which are not terribly large.
            let mut decomps = vec![None; members.len()];
            for a in 0..members.len() {
                for b in 0..a + 1 {
                    let c = indices[&of(&members[a], &members[b])];
                    if c > a {
                        decomps[c] = Some((a, b));
                    }
                }
            }
            GroupTree { members, decomps }
        }

        /// Compute a homomorphism of a group using the tree
        /// to elide expensive computations.
        ///
        /// Ideally, `F` should be a function that is very expensive to
        /// compute, while `HFn` should be comparatively cheaper.
        pub fn try_compute_homomorphism<E, H, F, HFn>(
            &self,
            mut compute: F,
            mut compose: HFn,
        ) -> StdResult<Vec<H>, E>
        where
            F: FnMut(&G) -> StdResult<H, E>,
            HFn: FnMut(&H, &H) -> StdResult<H, E>,
        {Ok({
            let mut out = Vec::with_capacity(self.members.len());

            for (g, decomp) in izip!(&self.members, &self.decomps) {
                let value = match *decomp {
                    None => compute(g)?,
                    Some((a, b)) => compose(&out[a], &out[b])?,
                };
                out.push(value);
            }
            out
        })}
    }

    /// Generates a finite group from a non-empty set of generators.
    ///
    /// The generators may contain duplicates or extraneous elements.
    ///
    /// The order of the output is arbitrary, but consistent for
    /// inputs that are related by a group isomorphism.
    pub fn generate_finite_group<G, GFn>(generators: &[G], mut g_fn: GFn) -> Vec<G>
    where
        G: Hash + Eq + Clone,
        GFn: FnMut(&G, &G) -> G,
    {
        use ::std::collections::{HashSet, VecDeque};
        assert!(generators.len() > 0, "empty groups do not exist!");

        let mut seen = HashSet::new();
        let mut out = vec![];

        let mut queue: VecDeque<_> = generators.iter().cloned().collect();

        while let Some(g) = queue.pop_front() {
            if seen.insert(g.clone()) {
                for h in generators {
                    queue.push_back(g_fn(&g, h));
                }
                out.push(g);
            }
        }
        out
    }
}

#[allow(dead_code)]
pub(crate) mod perm {
    use ::slice_of_array::prelude::*;
    use ::{Lattice, CoordStructure};
    use ::{FracRot, FracOp};
    use super::group::GroupTree;

    use ::Result;
    use ::util::perm::{Perm, argsort, Permute};

    pub fn dumb_symmetry_test(
        structure: &CoordStructure,
        ops: &[FracOp],
        tol: f64,
    ) -> Result<()>
    {Ok({
        let lattice = structure.lattice();
        let from_fracs = structure.to_fracs();
        let perms = of_spacegroup(structure, ops, tol)?;

        for (op, perm) in izip!(ops, perms) {
            dumb_validate_equivalent(
                lattice,
                &op.transform_prim(&from_fracs),
                &from_fracs.to_vec().permuted_by(&perm),
                tol,
            )
        }
    })}

    // Slow, and not even always correct
    fn dumb_nearest_distance(
        lattice: &Lattice,
        frac_a: &[f64; 3],
        frac_b: &[f64; 3],
    ) -> f64
    {
        use ::rsp2_array_utils::{vec_from_fn, dot};
        use ::Coords;
        let diff: [_; 3] = vec_from_fn(|k| frac_a[k] - frac_b[k]);
        let diff: [_; 3] = vec_from_fn(|k| diff[k] - diff[k].round());

        let mut diffs = vec![];
        for &a in &[-1., 0., 1.] {
            for &b in &[-1., 0., 1.] {
                for &c in &[-1., 0., 1.] {
                    diffs.push([diff[0] + a, diff[1] + b, diff[2] + c]);
                }
            }
        }
        let carts = Coords::Fracs(diffs).to_carts(lattice);
        carts.into_iter().map(|v| dot(&v, &v).sqrt())
            .min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap()
    }

    // Slow, and not even always correct
    fn dumb_validate_equivalent(
        lattice: &Lattice,
        frac_a: &[[f64; 3]],
        frac_b: &[[f64; 3]],
        tol: f64,
    )
    {
        for i in 0..frac_a.len() {
            let d = dumb_nearest_distance(lattice, &frac_a[i], &frac_b[i]);
            assert!(d < tol * (1.0 + 1e-7));
        }
    }

    // NOTE: Takes CoordStructure to communicate that the algorithm only cares
    //       about positions.  There is a small use-case for an <M: Eq> variant
    //       which could possibly allow two identical positions to be distinguished
    //       (maybe e.g. representing a defect as some superposition with a ghost)
    //       but I wouldn't want it to be the default.
    #[allow(unused)] // FIXME
    pub(crate) fn of_spacegroup(
        prim_structure: &CoordStructure,
        ops: &[FracOp],
        tol: f64,
    ) -> Result<Vec<Perm>>
    {Ok({
        use ::errors::*;
        let lattice = prim_structure.lattice();
        let from_fracs = prim_structure.to_fracs();

        let tree = GroupTree::from_all_members(
            ops.to_vec(),
            |second, first| second * first,
        );

        assert_eq!(
            ops,
            &tree.try_compute_homomorphism(
                |op| Ok::<_, Error>(op.clone()),
                |second, first| Ok::<_, Error>(second * first),
            )?[..]
        );

        tree.try_compute_homomorphism(
            |op| {
                let to_fracs = op.transform_prim(&from_fracs);
                let perm = of_rotation_impl(lattice, &from_fracs, &to_fracs[..], tol)?;
                dumb_validate_equivalent(
                    lattice,
                    &to_fracs[..],
                    &from_fracs.to_vec().permuted_by(&perm)[..],
                    tol
                );
                Ok::<_, Error>(perm)
            },
            // FIXME this works with second.permuted_by(first) but that's clearly wrong,
            //         the error is somewhere else
            |second, first| Ok(first.clone().permuted_by(second)),
        )?
    })}

    // NOTE: Takes CoordStructure to communicate that the algorithm only cares
    //       about positions.  There is a small use-case for an <M: Eq> variant
    //       which could possibly allow two identical positions to be distinguished
    //       (maybe e.g. representing a defect as some superposition with a ghost)
    //       but I wouldn't want it to be the default.
    #[allow(unused)] // FIXME
    pub(crate) fn of_rotation(
        structure: &CoordStructure,
        rotation: &FracRot,
        tol: f64,
    ) -> Result<Perm>
    {Ok({
        let lattice = structure.lattice();
        let from_fracs = structure.to_fracs();
        let to_fracs = rotation.transform_prim(&from_fracs);

        of_rotation_impl(lattice, &from_fracs, &to_fracs, tol)?
    })}

    fn of_rotation_impl(
        lattice: &Lattice,
        from_fracs: &[[f64; 3]],
        to_fracs: &[[f64; 3]],
        tol: f64,
    ) -> Result<Perm>
    {Ok({
        use ::rsp2_array_utils::dot;
        use ::ordered_float::NotNaN;
        use ::Coords::Fracs;

        // Sort both sides by some measure which is likely to produce a small
        // maximum value of (sorted_rotated_index - sorted_original_index).
        // The C code is optimized for this case, reducing an O(n^2)
        // search down to ~O(n). (for O(n log n) work overall, including the sort)
        //
        // We choose distance from the nearest bravais lattice point as our measure.
        let sort_by_lattice_distance = |fracs: &[[f64; 3]]| {
            let mut fracs = fracs.to_vec();
            for x in fracs.flat_mut() {
                *x -= x.round();
            }

            let distances = Fracs(fracs.clone())
                    .to_carts(lattice)
                    .into_iter()
                    .map(|x| NotNaN::new(dot(&x, &x).sqrt()).unwrap())
                    .collect::<Vec<_>>();
            let perm = argsort(&distances);
            (perm.clone(), fracs.permuted_by(&perm))
        };

        let (perm_from, sorted_from) = sort_by_lattice_distance(&from_fracs);
        let (perm_to, sorted_to) = sort_by_lattice_distance(&to_fracs);

        let perm_between = brute_force_near_identity(
            lattice,
            &sorted_from[..],
            &sorted_to[..],
            tol,
        )?;

        // Compose all of the permutations for the full permutation.
        //
        // Note that permutations are associative; that is,
        //     x.permute(p).permute(q) == x.permute(p.permute(q))
        perm_from
            .permuted_by(&perm_between)
            .permuted_by(&perm_to.inverted())
    })}


    // Optimized for permutations near the identity.
    fn brute_force_near_identity(
        lattice: &Lattice,
        from_fracs: &[[f64; 3]],
        to_fracs: &[[f64; 3]],
        tol: f64,
    ) -> Result<Perm>
    {Ok({

        assert_eq!(from_fracs.len(), to_fracs.len());
        let n = from_fracs.len();

        const UNSET: u32 = ::std::u32::MAX;
        assert!(n < UNSET as usize);

        let mut perm = vec![UNSET; from_fracs.len()];

        // optimization: Rather than filling the out vector in order,
        // we find where each index belongs (e.g. we place the 0, then
        // we place the 1, etc.).
        // Then we can track the first unassigned index.
        //
        // This works best if the permutation is close to the identity.
        // (more specifically, if the max value of 'out[i] - i' is small)
        //
        // This optimization does create some data dependencies which block
        // opportunities for parallelization; but for reducing O(n^2)
        // computations down to O(n), it is worth it.
        let mut search_start = 0;

        'from: for from in 0..n {

            // Skip through things filled out of order.
            while perm[search_start] != UNSET {
                search_start += 1;
            }

            for to in search_start..n {
                if perm[to] != UNSET {
                    continue;
                }

                // FIXME use utils
                let mut diff = [0f64; 3];
                for k in 0..3 {
                    diff[k] = from_fracs[from][k] - to_fracs[to][k];
                    diff[k] -= diff[k].round();
                }
                let mut distance2 = 0.0;
                for k in 0..3 {
                    let mut diff_cart = 0.0;
                    for l in 0..3 {
                        diff_cart += lattice.matrix()[k][l] * diff[l];
                    }
                    distance2 += diff_cart * diff_cart;
                }

                if distance2 < tol * tol {
                    perm[to] = from as u32;
                    continue 'from;
                }
            }
            bail!("positions are too dissimilar");
        }

        ensure!(
            perm.iter().all(|&x| x != UNSET),
            "multiple positions mapped to the same index");

        Perm::from_vec(perm)?
    })}

    #[cfg(test)]
    #[deny(unused)]
    mod tests {
        use ::Lattice;
        use super::{Perm, Permute};

        use ::slice_of_array::*;
        use ::rand::Rand;

        fn random_vec<T: Rand>(n: u32) -> Vec<T>
        { (0..n).map(|_| ::rand::random()).collect() }

        fn random_problem(n: u32) -> (Vec<[f64; 3]>, Perm, Vec<[f64; 3]>)
        {
            let original: Vec<[f64; 3]> = random_vec(n);
            let perm = Perm::random(n);
            let permuted = original.clone().permuted_by(&perm);
            (original, perm, permuted)
        }

        #[test]
        fn brute_force_works() {
            let (original, perm, permuted) = random_problem(20);
            let lattice = Lattice::new(random_vec(3).as_array());

            let output = super::brute_force_near_identity(
                &lattice, &original, &permuted, 1e-5,
            ).unwrap();

            assert_eq!(output, perm);
        }

        #[test]
        fn of_rotation_impl_works() {
            let (original, perm, permuted) = random_problem(20);
            let lattice = Lattice::new(random_vec(3).as_array());

            let output = super::of_rotation_impl(
                &lattice, &original, &permuted, 1e-5,
            ).unwrap();

            assert_eq!(output, perm);
        }
    }
}
