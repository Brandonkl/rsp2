
use ::std::ops::{Deref, DerefMut};

// ---------------------------------------------------------------------------

/// A 2-dimensional vector with operations for linear algebra.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[derive(Serialize, Deserialize)]
pub struct V2<X=f64>(pub [X; 2]);

/// A 3-dimensional vector with operations for linear algebra.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[derive(Serialize, Deserialize)]
pub struct V3<X=f64>(pub [X; 3]);

/// A 4-dimensional vector with operations for linear algebra.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[derive(Serialize, Deserialize)]
pub struct V4<X=f64>(pub [X; 4]);

// ---------------------------------------------------------------------------

/// A linear algebra dense matrix with 2 rows and fixed width.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[derive(Serialize, Deserialize)]
pub struct M2<V>(pub [V; 2]);

/// A linear algebra dense matrix with 3 rows and fixed width.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[derive(Serialize, Deserialize)]
pub struct M3<V>(pub [V; 3]);

/// A linear algebra dense matrix with 4 rows and fixed width.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[derive(Serialize, Deserialize)]
pub struct M4<V>(pub [V; 4]);

/// A square dense 2x2 matrix.
pub type M22<X=f64> = M2<V2<X>>;
/// A square dense 3x3 matrix.
pub type M33<X=f64> = M3<V3<X>>;
/// A square dense 4x4 matrix.
pub type M44<X=f64> = M4<V4<X>>;

// ---------------------------------------------------------------------------
// All types behave generally like their backing array type.

pub type Iter<'a, X> = ::std::slice::Iter<'a, X>;
pub type IterMut<'a, X> = ::std::slice::IterMut<'a, X>;

gen_each!{
    [
        {V2 X 2} {V3 X 3} {V4 X 4}
        {M2 V 2} {M3 V 3} {M4 V 4}
    ]
    impl_v_deref!(
        {$Cn:ident $T:ident $n:tt}
    ) => {
        impl<$T> Deref for $Cn<$T> {
            type Target = [$T; $n];

            #[inline(always)]
            fn deref(&self) -> &Self::Target
            { &self.0 }
        }

        impl<$T> DerefMut for $Cn<$T> {
            #[inline(always)]
            fn deref_mut(&mut self) -> &mut Self::Target
            { &mut self.0 }
        }

        // Fix a paper cut not solved by Deref, which is that many methods
        // take `I: IntoIterator`.
        impl<'a, $T> IntoIterator for &'a $Cn<$T> {
            type Item = &'a $T;
            type IntoIter = Iter<'a, $T>;

            #[inline(always)]
            fn into_iter(self) -> Self::IntoIter
            { self.0.iter() }
        }

        impl<'a, $T> IntoIterator for &'a mut $Cn<$T> {
            type Item = &'a mut $T;
            type IntoIter = IterMut<'a, $T>;

            #[inline(always)]
            fn into_iter(self) -> Self::IntoIter
            { self.0.iter_mut() }
        }
    }
}

// ---------------------------------------------------------------------------
