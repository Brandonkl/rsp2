//! Small fixed-size matrix types, compatible with `V2`/`V3`/`V4`
//!
//! This library primarily uses a row-based formalism; matrices are conceptually
//! understood to be containers of row-vectors. This formalism is most useful when
//! most vectors used are row vectors (in which case most matrix-vector multiplication
//! has the matrix on the right)

use ::traits::{Semiring, Ring, Field};
use ::traits::internal::{PrimitiveSemiring, PrimitiveRing, PrimitiveFloat};
use super::types::*;
use super::{Unvee, Envee};

// ---------------------------------------------------------------------------
// ------------------------------ PUBLIC API ---------------------------------

/// Construct a matrix from a function on indices.
#[inline(always)]
pub fn from_fn<M: FromFn<F>, B, F>(f: F) -> M
where F: FnMut(usize, usize) -> B,
{ FromFn::from_fn(f) }

/// Construct a matrix from a 2D array (of rows).
#[inline(always)]
pub fn from_array<A: IntoMatrix>(arr: A) -> A::Matrix
{ arr.into_matrix() }

/// Construct an identity matrix.
#[inline(always)]
pub fn eye<M: Eye>() -> M
{ Eye::eye() }

gen_each!{
    @{Mnn_Mn_Vn_n}
    impl_square_inherent_wrappers!(
        {$Mnn:ident $Mn:ident $Vn:ident $n:tt}
    ) => {
        impl<X> $Mnn<X> {
            /// Matrix inverse.
            #[inline(always)]
            pub fn inv(&self) -> Self
            where Self: Inv,
            { Inv::inv(self) }

            /// Matrix determinant.
            #[inline(always)]
            pub fn det(&self) -> DetT<Self>
            where Self: Det,
            { Det::det(self) }
        }
    }
}

gen_each!{
    @{Mn_n}
    impl_general_inherent_wrappers!(
        {$Mr:ident $r:tt}
    ) => {
        impl<V> $Mr<V> {
            /// Matrix transpose. (does not conjugate)
            #[inline(always)]
            pub fn t(&self) -> TransposeT<Self>
            where Self: Transpose,
            { Transpose::t(self) }

            /// Cast into a plain `[[T; m]; n]`.
            #[inline(always)]
            pub fn into_array(self) -> ArrayT<Self>
            where Self: IntoArray,
            { IntoArray::into_array(self) }

            /// Cast into a plain `&[[T; m]; n]`.
            #[inline(always)]
            pub fn as_array(&self) -> &ArrayT<Self>
            where Self: IntoArray,
            { IntoArray::as_array(self) }

            /// Cast into a plain `&mut [[T; m]; n]`.
            #[inline(always)]
            pub fn as_array_mut(&mut self) -> &mut ArrayT<Self>
            where Self: IntoArray,
            { IntoArray::as_array_mut(self) }
        }
    }
}

gen_each!{
    @{Mn_n}
    @{Vn_n}
    impl_general_inherent_wrappers_with_scalar!(
        {$Mr:ident $r:tt}
        {$Vc:ident $c:tt}
    ) => {
        impl<X> $Mr<$Vc<X>> {
            /// Construct a fixed-size vector from a function on indices.
            #[inline(always)]
            pub fn from_fn<B, F>(f: F) -> Self
            where Self: FromFn<F>, F: FnMut(usize, usize) -> B,
            { FromFn::from_fn(f) }

            /// Map each scalar element of a matrix.
            #[inline(always)]
            pub fn map<B, F>(self, mut f: F) -> $Mr<$Vc<B>>
            where F: FnMut(X) -> B,
            { $Mr(::rsp2_array_utils::map_arr(self.0, |row| row.map(&mut f))) }

            /// Apply a fallible function to each scalar element, with short-circuiting.
            #[inline(always)]
            pub fn try_map<E, B, F>(self, mut f: F) -> Result<$Mr<$Vc<B>>, E>
            where F: FnMut(X) -> Result<B, E>,
            { ::rsp2_array_utils::try_map_arr(self.0, |row| row.try_map(&mut f)).map($Mr) }

            /// Apply a fallible function to each scalar element, with short-circuiting.
            #[inline(always)]
            pub fn opt_map<B, F>(self, mut f: F) -> Option<$Mr<$Vc<B>>>
            where F: FnMut(X) -> Option<B>,
            { ::rsp2_array_utils::opt_map_arr(self.0, |row| row.opt_map(&mut f)).map($Mr) }
        }
    }
}

// -------------------------- END PUBLIC API ---------------------------------
// The rest is implementation and boiler boiler boiiiiler boiilerplaaaaate
// ---------------------------------------------------------------------------

/// Implementation detail of the free function `mat::from_fn`.
///
/// > **_Fuggedaboudit._**
pub trait FromFn<F>: Sized {
    fn from_fn(f: F) -> Self;
}

gen_each!{
    @{Mn_n}
    @{Vn_n}
    impl_transpose!(
        {$Mr:ident $r:tt}
        {$Vc:ident $c:tt}
    ) => {
        impl<X, F> FromFn<F> for $Mr<$Vc<X>>
          where F: FnMut(usize, usize) -> X,
        {
            #[inline]
            fn from_fn(mut f: F) -> Self {
                $Mr(<V![$r, _]>::from_fn(|r| {
                    <$Vc<_>>::from_fn(|c| f(r, c))
                }).0)
            }
        }
    }
}

// ---------------------------------------------------------------------------

/// Implementation detail of the free function `mat::from_array`.
///
/// > **_Fuggedaboudit._**
pub trait IntoMatrix: Sized {
    type Matrix;

    fn into_matrix(self) -> Self::Matrix;
}

pub type ArrayT<M> = <M as IntoArray>::Array;
/// Implementation detail of the inherent method `{M2,M3,M4}::into_array`.
///
/// > **_Fuggedaboudit._**
pub trait IntoArray: Sized {
    type Array;

    fn into_array(self) -> Self::Array;
    fn as_array(&self) -> &Self::Array;
    fn as_array_mut(&mut self) -> &mut Self::Array;
}

gen_each!{
    @{Mn_n}
    @{Vn_n}
    impl_transpose!(
        {$Mr:ident $r:tt}
        {$Vc:ident $c:tt}
    ) => {
        impl<X> IntoMatrix for [[X; $c]; $r] {
            type Matrix = $Mr<$Vc<X>>;

            #[inline(always)]
            fn into_matrix(self) -> Self::Matrix
            { $Mr(self.envee()) }
        }

        impl<X> IntoArray for $Mr<$Vc<X>> {
            type Array = [[X; $c]; $r];

            #[inline(always)]
            fn into_array(self) -> Self::Array
            { self.0.unvee() }

            #[inline(always)]
            fn as_array(&self) -> &Self::Array
            { self.0.unvee_ref() }

            #[inline(always)]
            fn as_array_mut(&mut self) -> &mut Self::Array
            { self.0.unvee_mut() }
        }
    }
}

// ---------------------------------------------------------------------------

/// Implementation detail of the free function `mat::eye`.
///
/// > **_Fuggedaboudit._**
pub trait Eye: Sized {
    fn eye() -> Self;
}

impl<X: Semiring> Eye for M22<X>
  where X: PrimitiveSemiring
{
    #[inline(always)]
    fn eye() -> Self { M2([
        V2([ X::one(), X::zero()]),
        V2([X::zero(),  X::one()]),
    ])}
}

impl<X: Semiring> Eye for M33<X>
  where X: PrimitiveSemiring
{
    #[inline(always)]
    fn eye() -> Self { M3([
        V3([ X::one(), X::zero(), X::zero()]),
        V3([X::zero(),  X::one(), X::zero()]),
        V3([X::zero(), X::zero(),  X::one()]),
    ])}
}

impl<X: Semiring> Eye for M44<X>
  where X: PrimitiveSemiring
{
    #[inline(always)]
    fn eye() -> Self { M4([
        V4([ X::one(), X::zero(), X::zero(), X::zero()]),
        V4([X::zero(),  X::one(), X::zero(), X::zero()]),
        V4([X::zero(), X::zero(),  X::one(), X::zero()]),
        V4([X::zero(), X::zero(), X::zero(),  X::one()]),
    ])}
}

// ---------------------------------------------------------------------------

/// Output of `det`. Probably a scalar type.
pub type DetT<A> = <A as Det>::Output;

/// Implementation detail of the inherent method `{M22,M33}::det`.
///
/// > **_Fuggedaboudit._**
pub trait Det {
    type Output;

    fn det(&self) -> Self::Output;
}

impl<T: Ring> Det for M22<T>
where T: PrimitiveRing,
{
    type Output = T;

    fn det(&self) -> T {
        self[0][0] * self[1][1] - self[0][1] * self[1][0]
    }
}

impl<T: Ring> Det for M33<T>
where T: PrimitiveRing,
{
    type Output = T;

    fn det(&self) -> T {
        let destructure = |v: &V3<_>| { (v[0], v[1], v[2]) };
        let (a0, a1, a2) = destructure(&self[0]);
        let (b0, b1, b2) = destructure(&self[1]);
        let (c0, c1, c2) = destructure(&self[2]);

        T::zero()
        + a0 * b1 * c2
        + a1 * b2 * c0
        + a2 * b0 * c1
        - a0 * b2 * c1
        - a1 * b0 * c2
        - a2 * b1 * c0
     }
}

// ---------------------------------------------------------------------------

/// Implementation detail of the inherent method `{M22,M33}::inv`.
///
/// > **_Fuggedaboudit._**
pub trait Inv {
    fn inv(&self) -> Self;
}

impl<T: Field> Inv for M22<T>
where T: PrimitiveFloat,
{
    fn inv(&self) -> Self {
        let rdet = T::one() / self.det();
        [ [ self[1][1] * rdet, -self[0][1] * rdet]
        , [-self[1][0] * rdet,  self[0][0] * rdet]
        ].into_matrix()
    }
}

impl<T: Field> Inv for M33<T>
where T: PrimitiveFloat,
{
    fn inv(&self) -> Self {
        let cofactors: M33<T> = from_fn(|r, c|
            T::zero()
            + self[(r+1) % 3][(c+1) % 3] * self[(r+2) % 3][(c+2) % 3]
            - self[(r+1) % 3][(c+2) % 3] * self[(r+2) % 3][(c+1) % 3]
        );
        let det = self[0].dot(&cofactors[0]);
        let rdet = T::one() / det;
        M33::from_fn(|r, c| rdet * cofactors[c][r])
    }
}

// ---------------------------------------------------------------------------

/// Output of `transpose`. Probably a matrix with the dimensions flipped.
pub type TransposeT<A> = <A as Transpose>::Output;

/// Implementation detail of the inherent method `{M2,M3,M4}::t`.
///
/// > **_Fuggedaboudit._**
pub trait Transpose {
    type Output;

    fn t(&self) -> Self::Output;
}

gen_each!{
    @{Mn_n}
    @{Vn_n}
    impl_transpose!(
        {$Mr:ident $r:tt}
        {$Vc:ident $c:tt}
    ) => {
        impl<X: Copy> Transpose for $Mr<$Vc<X>> {
            type Output = M![$c, V![$r, X]];

            #[inline]
            fn t(&self) -> Self::Output
            { from_fn(|r, c| self[c][r]) }
        }
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inverse_2() {
        let actual = from_array([[7., 2.], [-11., 4.]]).inv();
        let expected = from_array([
            [ 2./25., -1./25.],
            [11./50.,  7./50.],
        ]);

        for r in 0..2 {
            for c in 0..2 {
                assert_close!(abs=1e-12, expected[r][c], actual[r][c]);
            }
        }
    }

    #[test]
    fn test_inverse_3() {
        let actual = from_array([
            [1., 2., 4.],
            [5., 2., 1.],
            [3., 6., 3.],
        ]).inv();

        let expected = from_array([
            [ 0./1.,  1./4., -1./12.],
            [-1./6., -1./8., 19./72.],
            [ 1./3.,  0./1., -1./9. ],
        ]);

        for r in 0..3 {
            for c in 0..3 {
                assert_close!(abs=1e-12, expected[r][c], actual[r][c]);
            }
        }
    }
}
