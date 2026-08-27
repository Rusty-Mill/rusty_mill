//! A single unsafe primitive, isolated in its own crate so the rest of
//! `rusty_serde` (and `rusty_serde_derive`) can stay entirely safe Rust.
//!
//! # Why this exists
//!
//! `rusty_serde::Serializer::Ok` is generic - every format picks its own
//! (this crate's own `json`/`ron` formats both happen to use `()`, but the
//! trait doesn't require that). An object-safe, `dyn`-compatible erasure of
//! `Serializer` (needed so a `with`-routed field can call a serialize
//! function without the derive macro ever having parsed that field's
//! concrete type) can't mention `Ok` in its method signatures at all - `dyn
//! Trait` methods can't be generic over an associated type that varies per
//! implementor. So there's no way to *return* the real `Ok` value through
//! the erased trait's own method signatures.
//!
//! [`Out`] is the standard fix for this (the same trick
//! [`erased_serde`](https://docs.rs/erased-serde) uses upstream): the
//! *caller*, who already knows the concrete `Ok` type statically, hands the
//! erased call a type-erased pointer to a slot it owns. The erased call's
//! own impl - which, despite going through a `dyn Trait` boundary, was
//! itself written against the *same* concrete type (the erasure adapter is
//! a plain generic struct, monomorphized once per concrete serializer, that
//! merely *implements* an object-safe trait) - writes the real value back
//! through that pointer, reinterpreted as its original type. Both ends
//! agree on the type; only the `dyn Trait` boundary in between doesn't know
//! it.
//!
//! # Safety
//!
//! [`Out::new`] and [`Out::set`] must be paired on the exact same `T` -
//! `Out::new::<A>(...)` followed by `Out::set::<B>(...)` for `A != B` is
//! undefined behavior (a type-confused read on the next `dest.take()`).
//! Nothing in this module can check that for you across a `dyn Trait` call;
//! it's the caller's job to construct the erasure adapter so the same `T`
//! is threaded through both ends, the same way `rusty_serde`'s own erasure
//! layer (built on top of this crate) does.

use std::marker::PhantomData;

/// A type-erased handle to an `&'a mut Option<T>` output slot, for handing
/// across a `dyn Trait` boundary that can't itself mention `T`. See the
/// crate-level docs for why this exists and the safety invariant tying
/// [`Out::new`] to [`Out::set`].
pub struct Out<'a> {
    slot: *mut (),
    _marker: PhantomData<&'a mut ()>,
}

impl<'a> Out<'a> {
    /// Erases `dest` into an opaque handle. Constructing one is always
    /// safe on its own - `dest`'s type is only reinterpreted (unsafely) if
    /// and when [`Out::set`] is later called on the handle this returns.
    ///
    /// ```
    /// # use rusty_serde_erased::Out;
    /// let mut dest: Option<i64> = None;
    /// let out = Out::new(&mut dest);
    /// unsafe { out.set(42i64) };
    /// assert_eq!(dest, Some(42));
    /// ```
    pub fn new<T>(dest: &'a mut Option<T>) -> Self {
        Out {
            slot: dest as *mut Option<T> as *mut (),
            _marker: PhantomData,
        }
    }

    /// Writes `value` back into the slot this handle was built from.
    ///
    /// # Safety
    ///
    /// `T` must be the exact type the handle was constructed with via
    /// [`Out::new::<T>`] - calling this with any other `T` is undefined
    /// behavior. This method consumes `self` (rather than taking `&mut
    /// self`) so a handle can only ever be set once, matching how every
    /// `rusty_serde::Serializer` method already consumes `self` by value on
    /// its one, terminal call.
    pub unsafe fn set<T>(self, value: T) {
        let slot = self.slot as *mut Option<T>;
        // SAFETY: forwarded to the caller via this function's own contract.
        unsafe {
            *slot = Some(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Out;

    #[test]
    fn round_trips_a_copy_type() {
        let mut dest: Option<i64> = None;
        let out = Out::new(&mut dest);
        unsafe { out.set(7i64) };
        assert_eq!(dest, Some(7));
    }

    #[test]
    fn round_trips_a_heap_allocated_type() {
        let mut dest: Option<String> = None;
        let out = Out::new(&mut dest);
        unsafe { out.set(String::from("hello")) };
        assert_eq!(dest.as_deref(), Some("hello"));
    }

    #[test]
    fn round_trips_the_unit_type() {
        // The common case in practice: every one of this crate's own
        // Serializer::Ok types is `()`.
        let mut dest: Option<()> = None;
        let out = Out::new(&mut dest);
        unsafe { out.set(()) };
        assert_eq!(dest, Some(()));
    }

    #[derive(Debug, PartialEq)]
    struct Custom {
        a: i32,
        b: String,
    }

    #[test]
    fn round_trips_a_custom_struct() {
        let mut dest: Option<Custom> = None;
        let out = Out::new(&mut dest);
        unsafe {
            out.set(Custom {
                a: 1,
                b: "x".to_string(),
            })
        };
        assert_eq!(
            dest,
            Some(Custom {
                a: 1,
                b: "x".to_string()
            })
        );
    }

    #[test]
    fn unset_slot_stays_none() {
        let mut dest: Option<i64> = None;
        let _out = Out::new(&mut dest);
        // `_out` dropped without ever calling `set` - the slot must be
        // left untouched, not defaulted to some garbage value.
        assert_eq!(dest, None);
    }

    #[test]
    fn overwrites_a_previously_populated_slot() {
        let mut dest: Option<i64> = Some(1);
        let out = Out::new(&mut dest);
        unsafe { out.set(2i64) };
        assert_eq!(dest, Some(2));
    }
}
