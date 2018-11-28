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

use crate::traits::{IsArray, WithElement};

use std::mem::{ManuallyDrop, uninitialized};
use std::ptr;

/// Map an array by value.
///
/// `V` should be an array type, like `[T; n]`.
pub fn map_arr<B, V, F>(v: V, f: F) -> Brother!{V, B}
  where
    V: ArrayMapExt<B>,
    F: FnMut(V::Element) -> B,
{ v.map_the_thing(f) }

/// Map an array fallibly, short-circuiting on the first Error.
pub fn try_map_arr<B, V, E, F>(v: V, f: F) -> Result<Brother!{V, B}, E>
  where
    V: ArrayMapExt<B>,
    F: FnMut(V::Element) -> Result<B, E>,
{ v.try_map_the_thing(f) }

/// Map an array fallibly, short-circuiting on the first None.
pub fn opt_map_arr<B, V, F>(v: V, f: F) -> Option<Brother!{V, B}>
  where
    V: ArrayMapExt<B>,
    F: FnMut(V::Element) -> Option<B>,
{ v.opt_map_the_thing(f) }

/// Construct an array from a function on indices.
///
/// `V` should be an array type, like `[T; n]`.
pub fn arr_from_fn<V, F>(f: F) -> V
  where
    V: ArrayFromFunctionExt,
    F: FnMut(usize) -> V::Element,
{ V::from_fn(f) }

/// Construct an array fallibly, short-circuiting on the first Error.
pub fn try_arr_from_fn<V, E, F>(f: F) -> Result<V, E>
  where
    V: ArrayFromFunctionExt,
    F: FnMut(usize) -> Result<V::Element, E>,
{ V::try_from_fn(f) }

/// Construct an array fallibly, short-circuiting on the first None.
///
/// (you hear the author mumble something incomprehensible about monads)
pub fn opt_arr_from_fn<V, F>(f: F) -> Option<V>
  where
    V: ArrayFromFunctionExt,
    F: FnMut(usize) -> Option<V::Element>,
{ V::opt_from_fn(f) }

/// Implementation detail of `map_arr` and `map_mat`.
///
/// If you could just ignore this, that'd be swell.
/// Please prefer the freestanding functions instead.
pub trait ArrayMapExt<B>: IsArray + WithElement<B>
{
    // https://github.com/rust-lang/rust/issues/45781
    // let's stick to awkward and obscure method names for now
    #[inline]
    fn map_the_thing<F>(self, mut f: F) -> Brother!{Self, B}
    where F: FnMut(Self::Element) -> B,
    { self.try_map_the_thing(|x| Ok::<_, ()>(f(x))).ok().unwrap() }

    #[inline]
    fn try_map_the_thing<E, F>(self, mut f: F) -> Result<Brother!{Self, B}, E>
    where F: FnMut(Self::Element) -> Result<B, E>,
    {
        // !!! FIXME: use of uninitialized is unsound if B is uninhabited !!!

        // SAFETY:
        //  - uninitialized() data must never be read; beware of drops!
        //  - ptr::{write, read} argument must be aligned
        //    - [T; n] is aligned to T
        //  - ptr::write leaks the old value
        //  - ptr::read creates the potential for double-drops
        let me = ManuallyDrop::new(self);
        let mut out = ManuallyDrop::new(unsafe { uninitialized::<Brother!{Self, B}>() });
        for i in 0..Self::array_len() {
            let x = unsafe { ptr::read(&me.array_as_slice()[i]) };

            // If the function panics, uninit data remaining in `self` and
            // `out` are both safely forgotten thanks to ManuallyDrop.
            // Any written or unread values are leaked.
            match f(x) {
                Ok(x) => unsafe { ptr::write(&mut out.array_as_mut_slice()[i], x) },
                Err(e) => {
                    // Drop each unread element, and each element that was written.
                    // These drops could also panic; but the prior justifications
                    // for panic-safety still hold.
                    // NOTE: the element at index `i` does not need to be dropped from
                    //       anywhere, because we gave ours away and got nothing back.
                    for p in out.array_as_slice()[..i].iter().rev() {
                        unsafe { ptr::read(p) }; // drop!
                    }
                    for p in &me.array_as_slice()[i + 1..] {
                        unsafe { ptr::read(p) }; // drop!
                    }
                    return Err(e);
                }
            }
        }
        // `me` can now be leaked, as we have given away ownership of all elements.
        // `out` can be safely returned because it is now fully initialized.
        Ok(ManuallyDrop::into_inner(out))
    }

    #[inline]
    fn opt_map_the_thing<F>(self, mut f: F) -> Option<Brother!{Self, B}>
    where F: FnMut(Self::Element) -> Option<B>
    {
        // hand the problem off to our "sufficiently smart compiler"
        self.try_map_the_thing(|x| f(x).ok_or(Err::<B, _>(()))).ok()
    }
}

impl<B, V: WithElement<B>> ArrayMapExt<B> for V { }

/// Implementation detail of `vec_from_fn` and `mat_from_fn`.
///
/// If you could just ignore this, that'd be swell.
/// Please prefer the freestanding functions instead.
pub trait ArrayFromFunctionExt: IsArray + WithElement<usize>
{
    /// If you haven't guessed by now,
    /// we are banking quite heavily on compiler optimizations.
    fn array_of_indices() -> Brother!{Self, usize};

    fn from_fn<F>(f: F) -> Self
    where F: FnMut(usize) -> Self::Element;

    fn try_from_fn<E, F>(f: F) -> Result<Self, E>
    where F: FnMut(usize) -> Result<Self::Element, E>;

    fn opt_from_fn<F>(f: F) -> Option<Self>
    where F: FnMut(usize) -> Option<Self::Element>;
}

impl<V: WithElement<usize>> ArrayFromFunctionExt for V
where Brother!{Self, usize}: ArrayMapExt<Self::Element>,
{
    #[inline(always)]
    fn array_of_indices() -> Brother!{Self, usize}
    {
        let p = &INDICES[0..Self::array_len()];
        let p = p as *const [usize] as *const Brother!{Self, usize};
        unsafe { std::ptr::read(p) }
    }

    #[inline(always)]
    fn from_fn<F>(f: F) -> Self
    where F: FnMut(usize) -> Self::Element
    { Self::array_of_indices().map_the_thing(f) }

    #[inline(always)]
    fn try_from_fn<E, F>(f: F) -> Result<Self, E>
    where F: FnMut(usize) -> Result<Self::Element, E>
    { Self::array_of_indices().try_map_the_thing(f) }

    #[inline(always)]
    fn opt_from_fn<F>(f: F) -> Option<Self>
    where F: FnMut(usize) -> Option<Self::Element>
    { Self::array_of_indices().opt_map_the_thing(f) }
}

const INDICES: [usize; 65] = [
     0,  1,  2,  3,  4,  5,  6,  7,  8,  9,
    10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
    20, 21, 22, 23, 24, 25, 26, 27, 28, 29,
    30, 31, 32, 33, 34, 35, 36, 37, 38, 39,
    40, 41, 42, 43, 44, 45, 46, 47, 48, 49,
    50, 51, 52, 53, 54, 55, 56, 57, 58, 59,
    60, 61, 62, 63, 64,
];

#[cfg(test)]
#[deny(dead_code)]
mod tests {
    use crate::test_util::PushDrop;
    use std::cell::RefCell;

    #[test]
    fn try_vec_from_fn_drop() {
        use super::try_arr_from_fn;

        let vec = RefCell::new(vec![3, 4, 2]);

        // Completely construct something;
        // nothing should get dropped.
        let arr: Result<[PushDrop<'_, i32>; 5], ()> = try_arr_from_fn(
            |i| Ok(PushDrop::new(i as i32, &vec))
        );
        assert_eq!(*vec.borrow(), vec![3, 4, 2]);
        std::mem::forget(arr);

        // Interrupt construction with an Err.
        // The successfully added elements should be dropped in reverse.
        let ret: Result<[PushDrop<'_, i32>; 5], _> = try_arr_from_fn(
            |i| match i {
                3 => Err("lol!"),
                i => Ok(PushDrop::new(i as i32, &vec)),
            }
        );
        assert_eq!(ret, Err("lol!"));
        assert_eq!(*vec.borrow(), vec![3, 4, 2, 2, 1, 0]);
    }

    #[test]
    fn try_map_arr_drop() {
        use super::{try_map_arr, arr_from_fn};

        let vec = RefCell::new(vec![]);
        let make_arr = || -> [PushDrop<'_, i32>; 5] {
            arr_from_fn(|i| PushDrop::new(i as i32, &vec))
        };

        // Completely map something;
        // nothing should get dropped.
        let arr = make_arr();
        let _arr: Result<[PushDrop<'_, i32>; 5], ()> =
            try_map_arr(arr, |x| Ok(PushDrop::new(x.into_inner() + 10, &vec)));

        assert_eq!(*vec.borrow(), vec![]);
        std::mem::forget(_arr);

        // Interrupt construction with an Err.
        // Both the unmapped elements and the successfully mapped
        //   elements should be dropped.
        let arr = make_arr();
        let ret: Result<[PushDrop<'_, i32>; 5], _> =
            try_map_arr(arr, |x| match x.into_inner() {
                2 => Err("lol!"),
                x => Ok(PushDrop::new(x + 10, &vec)),
            });
        assert_eq!(ret, Err("lol!"));
        vec.borrow_mut().sort();
        assert_eq!(*vec.borrow(), vec![3, 4, 10, 11]);
    }
}
