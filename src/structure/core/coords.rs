use ::Lattice;
use ::oper::{Perm, Permute};
use ::oper::{Part, Partition};
use ::oper::part::Unlabeled;

use ::rsp2_array_types::{V3, M33};

/// Wrapper type for coordinates used as input to some APIs.
///
/// This allows a function to support either cartesian coordinates,
/// or fractional coordinates with respect to some lattice.
#[derive(Debug, Clone, PartialEq)]
pub enum CoordsKind {
    Carts(Vec<V3>),
    Fracs(Vec<V3>),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Tag { Cart, Frac }

impl CoordsKind {
    pub fn len(&self) -> usize
    { self.as_slice().1.len() }

    pub(crate) fn as_slice(&self) -> (Tag, &[V3])
    { match *self {
        CoordsKind::Carts(ref c) => (Tag::Cart, c),
        CoordsKind::Fracs(ref c) => (Tag::Frac, c),
    }}

    pub(crate) fn as_mut_vec(&mut self) -> (Tag, &mut Vec<V3>)
    { match *self {
        CoordsKind::Carts(ref mut c) => (Tag::Cart, c),
        CoordsKind::Fracs(ref mut c) => (Tag::Frac, c),
    }}

    pub(crate) fn into_vec(self) -> (Tag, Vec<V3>)
    { match self {
        CoordsKind::Carts(c) => (Tag::Cart, c),
        CoordsKind::Fracs(c) => (Tag::Frac, c),
    }}

    pub(crate) fn from_vec(tag: Tag, c: Vec<V3>) -> Self
    { match tag {
        Tag::Cart => CoordsKind::Carts(c),
        Tag::Frac => CoordsKind::Fracs(c),
    }}
}

// projections
impl CoordsKind {
    pub(crate) fn as_carts_opt(&self) -> Option<&[V3]>
    { match *self {
        CoordsKind::Carts(ref x) => Some(x),
        CoordsKind::Fracs(_) => None,
    }}

    pub(crate) fn as_fracs_opt(&self) -> Option<&[V3]>
    { match *self {
        CoordsKind::Carts(_) => None,
        CoordsKind::Fracs(ref x) => Some(x),
    }}
}

// conversions
impl CoordsKind {
    pub fn into_carts(self, lattice: &Lattice) -> Vec<V3>
    { match self {
        CoordsKind::Carts(c) => c,
        CoordsKind::Fracs(c) => dot_n3_33(&c, lattice.matrix()),
    }}

    pub fn into_fracs(self, lattice: &Lattice) -> Vec<V3>
    { match self {
        CoordsKind::Carts(c) => dot_n3_33(&c, lattice.inverse_matrix()),
        CoordsKind::Fracs(c) => c,
    }}

    pub fn to_carts(&self, lattice: &Lattice) -> Vec<V3>
    { match *self {
        CoordsKind::Carts(ref c) => c.clone(),
        CoordsKind::Fracs(ref c) => dot_n3_33(c, lattice.matrix()),
    }}

    pub fn to_fracs(&self, lattice: &Lattice) -> Vec<V3>
    { match *self {
        CoordsKind::Carts(ref c) => dot_n3_33(c, lattice.inverse_matrix()),
        CoordsKind::Fracs(ref c) => c.clone(),
    }}

    pub(crate) fn into_tag(self, tag: Tag, lattice: &Lattice) -> Vec<V3>
    { match tag {
        Tag::Cart => self.into_carts(lattice),
        Tag::Frac => self.into_fracs(lattice),
    }}

    #[allow(unused)]
    pub(crate) fn to_tag(&self, tag: Tag, lattice: &Lattice) -> Vec<V3>
    { match tag {
        Tag::Cart => self.to_carts(lattice),
        Tag::Frac => self.to_fracs(lattice),
    }}
}

fn dot_n3_33(c: &[V3], m: &M33) -> Vec<V3>
{ c.iter().map(|v| v * m).collect() }

impl Permute for CoordsKind {
    fn permuted_by(self, perm: &Perm) -> CoordsKind
    { match self {
        CoordsKind::Carts(c) => CoordsKind::Carts(c.permuted_by(perm)),
        CoordsKind::Fracs(c) => CoordsKind::Fracs(c.permuted_by(perm)),
    }}
}

impl Partition for CoordsKind {
    fn into_unlabeled_partitions<L>(self, part: &Part<L>) -> Unlabeled<Self>
    {
        let (tag, coords) = self.into_vec();
        Box::new(coords.into_unlabeled_partitions(part).map(move |c| Self::from_vec(tag, c)))
    }
}

#[cfg(test)]
#[deny(unused)]
mod tests {
    use ::Lattice;
    use ::CoordsKind::{Fracs, Carts};

    use ::rsp2_array_types::Envee;

    // make sure the library correctly chooses whether to use the
    // regular matrix, the inverse matrix, or no matrix
    #[test]
    fn div_vs_mul() {

        let x = |mag| vec![[mag, 0.0, 0.0]].envee();
        let lattice = Lattice::cubic(2.0);

        assert_eq!(x(1.0), Fracs(x(1.0)).to_fracs(&lattice));
        assert_eq!(x(1.0), Fracs(x(1.0)).into_fracs(&lattice));
        assert_eq!(x(2.0), Fracs(x(1.0)).to_carts(&lattice));
        assert_eq!(x(2.0), Fracs(x(1.0)).into_carts(&lattice));

        assert_eq!(x(0.5), Carts(x(1.0)).to_fracs(&lattice));
        assert_eq!(x(0.5), Carts(x(1.0)).into_fracs(&lattice));
        assert_eq!(x(1.0), Carts(x(1.0)).to_carts(&lattice));
        assert_eq!(x(1.0), Carts(x(1.0)).into_carts(&lattice));
    }

    // make sure matrix multiplication is done in the correct order
    #[test]
    fn multiplication_order() {

        // a matrix not equal to its transpose
        let lattice = Lattice::from(&[
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
        ]);

        // what happens to [1,0,0] when we interpret it in one coord system
        //  and then convert to the other system
        let input = vec![[1.0, 0.0, 0.0]].envee();
        let frac_to_cart = vec![[0.0, 1.0, 0.0]].envee();
        let cart_to_frac = vec![[0.0, 0.0, 1.0]].envee();

        assert_eq!(&frac_to_cart, &Fracs(input.clone()).to_carts(&lattice));
        assert_eq!(&frac_to_cart, &Fracs(input.clone()).into_carts(&lattice));
        assert_eq!(&cart_to_frac, &Carts(input.clone()).to_fracs(&lattice));
        assert_eq!(&cart_to_frac, &Carts(input.clone()).into_fracs(&lattice));
    }
}
