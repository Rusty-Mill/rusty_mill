//! `select!`, `join!`/`try_join!` and `task_local!`.
//!
//! ## `select!`: tokio's syntax and semantics, as a proc macro
//!
//! `select!` races any number of futures and runs the handler of
//! whichever completes first (and matches its pattern), dropping the
//! rest. It accepts tokio's full grammar -- refutable patterns,
//! `, if <precondition>` guards, `biased;`, an `else` branch, and up to
//! 64 branches -- with tokio's semantics; see [`select!`] for the
//! details.
//!
//! It started life as a `macro_rules!` limited to 2 to 5 branches with
//! irrefutable patterns and none of the above, because generating a
//! fresh binding per branch declaratively needed either a recursive
//! tt-muncher or a `paste!`-style proc macro, and a proc macro was a new
//! dependency. `rusty_tokio-macros` has since taken on `syn`/`quote`/
//! `proc-macro2` for `#[main]`/`#[test]`, so that reason is gone, and 9
//! of the workspace's 25 `tokio::select!` sites needed something the
//! old macro lacked (gap G6 in `docs/research/ASYNC-RUNTIME-INVENTORY.md`).
//! The work now happens in `rusty_tokio-macros`' `select_internal`;
//! the `macro_rules! select` here only forwards to it with `$crate`,
//! which a proc macro cannot name for itself -- the same split tokio
//! uses.
//!
//! ## `join!`/`try_join!`: still 2 to 5 branches
//!
//! These remain explicit `macro_rules!` arities. A declarative macro has
//! no clean way to generate a fresh, uniquely-named local binding per
//! repetition on stable Rust, so each arity is spelled out directly --
//! mechanical to extend if a real need for more branches comes up, and
//! easy to verify by inspection.

/// Race several futures; run the handler of the first one to complete
/// with a value matching its pattern, and drop the rest. Same syntax and
/// semantics as tokio's `select!`:
///
/// ```text
/// select! {
///     [biased;]
///     <pattern> = <async expression> [, if <precondition>] => <handler>,
///     ...
///     [else => <expression>]
/// }
/// ```
///
/// 1. Every `if <precondition>` is evaluated first, in order. A branch
///    whose precondition is `false` is disabled for this `select!`.
/// 2. Every branch's `<async expression>` is then evaluated (disabled
///    branches included, as in tokio -- their futures are built but
///    never polled) and pinned in place. Anything implementing
///    `IntoFuture` works, so `&mut fut` and `JoinHandle`s do too.
/// 3. The futures are polled concurrently. Each poll starts at a
///    randomly chosen branch, so a branch that is always ready can't
///    starve the others; with `biased;` they are polled in the order
///    written instead (cheaper, and useful when one branch -- usually
///    cancellation -- must win ties).
/// 4. When a future completes, its output is matched against the
///    branch's pattern. On a mismatch (`Some(x) = rx.recv()` seeing
///    `None`) that branch is disabled and the race goes on.
/// 5. On a match, the remaining futures are dropped and the handler
///    runs with the pattern's bindings; its value is the value of
///    `select!`. Because it runs after the futures are gone and in the
///    caller's own context, a handler may borrow what they borrowed and
///    may `.await`, `?`, `return`, `break` or `continue`.
/// 6. If every branch ends up disabled, the `else` expression runs; with
///    no `else`, `select!` panics with "all branches are disabled and
///    there is no else branch".
///
/// Must be used inside an `async` context.
///
/// ```
/// # use rusty_tokio::Runtime;
/// # let rt = Runtime::new().unwrap();
/// # rt.block_on(async {
/// let winner = rusty_tokio::select! {
///     a = async { 1 } => a,
///     b = async { 2 } => b,
/// };
/// assert!(winner == 1 || winner == 2);
///
/// let none: Option<u8> = None;
/// let fell_through = rusty_tokio::select! {
///     biased;
///     Some(v) = async { none } => v,
///     _ = std::future::pending::<()>(), if false => 0,
///     else => 255,
/// };
/// assert_eq!(fell_through, 255);
/// # });
/// ```
#[macro_export]
macro_rules! select {
    ($($tokens:tt)*) => {
        $crate::__select_internal! { ($crate) $($tokens)* }
    };
}

/// Runtime support for macro expansions -- public only because a
/// macro expanded in another crate has to name it. Not API.
#[doc(hidden)]
pub mod support {
    use std::cell::Cell;
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;

    thread_local! {
        /// xorshift64* state; 0 means "not seeded yet" (0 is also the
        /// one state xorshift can never leave, so it can't be a real
        /// state anyway).
        static RNG: Cell<u64> = const { Cell::new(0) };
    }

    /// A per-thread seed from std's `RandomState` keys (seeded from the
    /// OS once per thread) mixed with the thread's id, so threads don't
    /// share a sequence. `select!` only needs "not the same branch
    /// first every time", not anything cryptographic, and this keeps
    /// the crate free of a `rand`/`fastrand` dependency.
    fn seed() -> u64 {
        RandomState::new().hash_one(std::thread::current().id()) | 1
    }

    /// A pseudo-random number in `0..n` (`n` must be non-zero; `select!`
    /// always passes its branch count). Lemire's multiply-shift rather
    /// than `%`, which is both cheaper and less biased. During
    /// thread-local teardown, when the state can't be reached, it
    /// returns 0 -- polling in declaration order is a fine fallback.
    pub fn thread_rng_n(n: u32) -> u32 {
        let next = RNG.try_with(|state| {
            let mut x = state.get();
            if x == 0 {
                x = seed();
            }
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            state.set(x);
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        });
        let Ok(random) = next else {
            return 0;
        };
        (((random >> 32) * u64::from(n)) >> 32) as u32
    }
}

/// `join!`/`try_join!`: run two or more futures concurrently *within the
/// calling task* (no extra [`spawn`](crate::spawn), no extra `Task`) and
/// resolve once every one of them has, returning a tuple of their
/// outputs.
///
/// Shares [`select!`]'s "poll N sub-futures every wake, sharing one
/// `poll_fn`" shape, but keeps a **2 to 5 branches** scope limit (see
/// this module's docs for why) -- the difference is
/// `join!` waits for *all* branches instead of stopping at the first.
/// Each branch is polled at most once per already-resolved -> not
/// re-polled after it completes, so a branch that finishes early doesn't
/// get spuriously polled again while waiting on its slower siblings.
///
/// `try_join!` is the `Result`-aware sibling: every branch must resolve
/// to a `Result` with the *same* error type (no `From`/`?`-style
/// conversion between differing error types -- keeping every branch's
/// error type identical sidesteps an inference question a first pass
/// doesn't need to take on), and it short-circuits on the first `Err`,
/// returning immediately rather than waiting for the remaining branches
/// to finish pointlessly. As with `select!`, "short-circuits" means the
/// still-pending branches' futures are dropped as soon as the
/// macro-generated future resolves (when their local `pin!` bindings go
/// out of scope), not actively cancelled mid-poll.
///
/// ```
/// # use rusty_tokio::Runtime;
/// # let rt = Runtime::new().unwrap();
/// # rt.block_on(async {
/// let (a, b) = rusty_tokio::join!(async { 1 }, async { 2 });
/// assert_eq!((a, b), (1, 2));
///
/// let ok: Result<(i32, i32), &str> =
///     rusty_tokio::try_join!(async { Ok(1) }, async { Ok(2) });
/// assert_eq!(ok, Ok((1, 2)));
///
/// let err: Result<(i32, i32), &str> =
///     rusty_tokio::try_join!(async { Ok(1) }, async { Err("boom") });
/// assert_eq!(err, Err("boom"));
/// # });
/// ```
#[macro_export]
macro_rules! join {
    ($f0:expr, $f1:expr $(,)?) => {{
        use ::std::future::Future as _;
        let mut __f0 = ::std::pin::pin!($f0);
        let mut __f1 = ::std::pin::pin!($f1);
        let mut __o0 = ::std::option::Option::None;
        let mut __o1 = ::std::option::Option::None;
        ::std::future::poll_fn(move |cx| {
            if __o0.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f0.as_mut().poll(cx) {
                    __o0 = ::std::option::Option::Some(__out);
                }
            }
            if __o1.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f1.as_mut().poll(cx) {
                    __o1 = ::std::option::Option::Some(__out);
                }
            }
            if __o0.is_some() && __o1.is_some() {
                ::std::task::Poll::Ready((__o0.take().unwrap(), __o1.take().unwrap()))
            } else {
                ::std::task::Poll::Pending
            }
        })
        .await
    }};
    ($f0:expr, $f1:expr, $f2:expr $(,)?) => {{
        use ::std::future::Future as _;
        let mut __f0 = ::std::pin::pin!($f0);
        let mut __f1 = ::std::pin::pin!($f1);
        let mut __f2 = ::std::pin::pin!($f2);
        let mut __o0 = ::std::option::Option::None;
        let mut __o1 = ::std::option::Option::None;
        let mut __o2 = ::std::option::Option::None;
        ::std::future::poll_fn(move |cx| {
            if __o0.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f0.as_mut().poll(cx) {
                    __o0 = ::std::option::Option::Some(__out);
                }
            }
            if __o1.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f1.as_mut().poll(cx) {
                    __o1 = ::std::option::Option::Some(__out);
                }
            }
            if __o2.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f2.as_mut().poll(cx) {
                    __o2 = ::std::option::Option::Some(__out);
                }
            }
            if __o0.is_some() && __o1.is_some() && __o2.is_some() {
                ::std::task::Poll::Ready((
                    __o0.take().unwrap(),
                    __o1.take().unwrap(),
                    __o2.take().unwrap(),
                ))
            } else {
                ::std::task::Poll::Pending
            }
        })
        .await
    }};
    ($f0:expr, $f1:expr, $f2:expr, $f3:expr $(,)?) => {{
        use ::std::future::Future as _;
        let mut __f0 = ::std::pin::pin!($f0);
        let mut __f1 = ::std::pin::pin!($f1);
        let mut __f2 = ::std::pin::pin!($f2);
        let mut __f3 = ::std::pin::pin!($f3);
        let mut __o0 = ::std::option::Option::None;
        let mut __o1 = ::std::option::Option::None;
        let mut __o2 = ::std::option::Option::None;
        let mut __o3 = ::std::option::Option::None;
        ::std::future::poll_fn(move |cx| {
            if __o0.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f0.as_mut().poll(cx) {
                    __o0 = ::std::option::Option::Some(__out);
                }
            }
            if __o1.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f1.as_mut().poll(cx) {
                    __o1 = ::std::option::Option::Some(__out);
                }
            }
            if __o2.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f2.as_mut().poll(cx) {
                    __o2 = ::std::option::Option::Some(__out);
                }
            }
            if __o3.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f3.as_mut().poll(cx) {
                    __o3 = ::std::option::Option::Some(__out);
                }
            }
            if __o0.is_some() && __o1.is_some() && __o2.is_some() && __o3.is_some() {
                ::std::task::Poll::Ready((
                    __o0.take().unwrap(),
                    __o1.take().unwrap(),
                    __o2.take().unwrap(),
                    __o3.take().unwrap(),
                ))
            } else {
                ::std::task::Poll::Pending
            }
        })
        .await
    }};
    ($f0:expr, $f1:expr, $f2:expr, $f3:expr, $f4:expr $(,)?) => {{
        use ::std::future::Future as _;
        let mut __f0 = ::std::pin::pin!($f0);
        let mut __f1 = ::std::pin::pin!($f1);
        let mut __f2 = ::std::pin::pin!($f2);
        let mut __f3 = ::std::pin::pin!($f3);
        let mut __f4 = ::std::pin::pin!($f4);
        let mut __o0 = ::std::option::Option::None;
        let mut __o1 = ::std::option::Option::None;
        let mut __o2 = ::std::option::Option::None;
        let mut __o3 = ::std::option::Option::None;
        let mut __o4 = ::std::option::Option::None;
        ::std::future::poll_fn(move |cx| {
            if __o0.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f0.as_mut().poll(cx) {
                    __o0 = ::std::option::Option::Some(__out);
                }
            }
            if __o1.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f1.as_mut().poll(cx) {
                    __o1 = ::std::option::Option::Some(__out);
                }
            }
            if __o2.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f2.as_mut().poll(cx) {
                    __o2 = ::std::option::Option::Some(__out);
                }
            }
            if __o3.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f3.as_mut().poll(cx) {
                    __o3 = ::std::option::Option::Some(__out);
                }
            }
            if __o4.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f4.as_mut().poll(cx) {
                    __o4 = ::std::option::Option::Some(__out);
                }
            }
            if __o0.is_some()
                && __o1.is_some()
                && __o2.is_some()
                && __o3.is_some()
                && __o4.is_some()
            {
                ::std::task::Poll::Ready((
                    __o0.take().unwrap(),
                    __o1.take().unwrap(),
                    __o2.take().unwrap(),
                    __o3.take().unwrap(),
                    __o4.take().unwrap(),
                ))
            } else {
                ::std::task::Poll::Pending
            }
        })
        .await
    }};
}

/// See [`join!`]'s docs -- this is its `Result`-aware, short-circuiting
/// sibling.
#[macro_export]
macro_rules! try_join {
    ($f0:expr, $f1:expr $(,)?) => {{
        use ::std::future::Future as _;
        let mut __f0 = ::std::pin::pin!($f0);
        let mut __f1 = ::std::pin::pin!($f1);
        let mut __o0 = ::std::option::Option::None;
        let mut __o1 = ::std::option::Option::None;
        ::std::future::poll_fn(move |cx| {
            if __o0.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f0.as_mut().poll(cx) {
                    match __out {
                        ::std::result::Result::Ok(__v) => __o0 = ::std::option::Option::Some(__v),
                        ::std::result::Result::Err(__e) => {
                            return ::std::task::Poll::Ready(::std::result::Result::Err(__e))
                        }
                    }
                }
            }
            if __o1.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f1.as_mut().poll(cx) {
                    match __out {
                        ::std::result::Result::Ok(__v) => __o1 = ::std::option::Option::Some(__v),
                        ::std::result::Result::Err(__e) => {
                            return ::std::task::Poll::Ready(::std::result::Result::Err(__e))
                        }
                    }
                }
            }
            if __o0.is_some() && __o1.is_some() {
                ::std::task::Poll::Ready(::std::result::Result::Ok((
                    __o0.take().unwrap(),
                    __o1.take().unwrap(),
                )))
            } else {
                ::std::task::Poll::Pending
            }
        })
        .await
    }};
    ($f0:expr, $f1:expr, $f2:expr $(,)?) => {{
        use ::std::future::Future as _;
        let mut __f0 = ::std::pin::pin!($f0);
        let mut __f1 = ::std::pin::pin!($f1);
        let mut __f2 = ::std::pin::pin!($f2);
        let mut __o0 = ::std::option::Option::None;
        let mut __o1 = ::std::option::Option::None;
        let mut __o2 = ::std::option::Option::None;
        ::std::future::poll_fn(move |cx| {
            if __o0.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f0.as_mut().poll(cx) {
                    match __out {
                        ::std::result::Result::Ok(__v) => __o0 = ::std::option::Option::Some(__v),
                        ::std::result::Result::Err(__e) => {
                            return ::std::task::Poll::Ready(::std::result::Result::Err(__e))
                        }
                    }
                }
            }
            if __o1.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f1.as_mut().poll(cx) {
                    match __out {
                        ::std::result::Result::Ok(__v) => __o1 = ::std::option::Option::Some(__v),
                        ::std::result::Result::Err(__e) => {
                            return ::std::task::Poll::Ready(::std::result::Result::Err(__e))
                        }
                    }
                }
            }
            if __o2.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f2.as_mut().poll(cx) {
                    match __out {
                        ::std::result::Result::Ok(__v) => __o2 = ::std::option::Option::Some(__v),
                        ::std::result::Result::Err(__e) => {
                            return ::std::task::Poll::Ready(::std::result::Result::Err(__e))
                        }
                    }
                }
            }
            if __o0.is_some() && __o1.is_some() && __o2.is_some() {
                ::std::task::Poll::Ready(::std::result::Result::Ok((
                    __o0.take().unwrap(),
                    __o1.take().unwrap(),
                    __o2.take().unwrap(),
                )))
            } else {
                ::std::task::Poll::Pending
            }
        })
        .await
    }};
    ($f0:expr, $f1:expr, $f2:expr, $f3:expr $(,)?) => {{
        use ::std::future::Future as _;
        let mut __f0 = ::std::pin::pin!($f0);
        let mut __f1 = ::std::pin::pin!($f1);
        let mut __f2 = ::std::pin::pin!($f2);
        let mut __f3 = ::std::pin::pin!($f3);
        let mut __o0 = ::std::option::Option::None;
        let mut __o1 = ::std::option::Option::None;
        let mut __o2 = ::std::option::Option::None;
        let mut __o3 = ::std::option::Option::None;
        ::std::future::poll_fn(move |cx| {
            if __o0.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f0.as_mut().poll(cx) {
                    match __out {
                        ::std::result::Result::Ok(__v) => __o0 = ::std::option::Option::Some(__v),
                        ::std::result::Result::Err(__e) => {
                            return ::std::task::Poll::Ready(::std::result::Result::Err(__e))
                        }
                    }
                }
            }
            if __o1.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f1.as_mut().poll(cx) {
                    match __out {
                        ::std::result::Result::Ok(__v) => __o1 = ::std::option::Option::Some(__v),
                        ::std::result::Result::Err(__e) => {
                            return ::std::task::Poll::Ready(::std::result::Result::Err(__e))
                        }
                    }
                }
            }
            if __o2.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f2.as_mut().poll(cx) {
                    match __out {
                        ::std::result::Result::Ok(__v) => __o2 = ::std::option::Option::Some(__v),
                        ::std::result::Result::Err(__e) => {
                            return ::std::task::Poll::Ready(::std::result::Result::Err(__e))
                        }
                    }
                }
            }
            if __o3.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f3.as_mut().poll(cx) {
                    match __out {
                        ::std::result::Result::Ok(__v) => __o3 = ::std::option::Option::Some(__v),
                        ::std::result::Result::Err(__e) => {
                            return ::std::task::Poll::Ready(::std::result::Result::Err(__e))
                        }
                    }
                }
            }
            if __o0.is_some() && __o1.is_some() && __o2.is_some() && __o3.is_some() {
                ::std::task::Poll::Ready(::std::result::Result::Ok((
                    __o0.take().unwrap(),
                    __o1.take().unwrap(),
                    __o2.take().unwrap(),
                    __o3.take().unwrap(),
                )))
            } else {
                ::std::task::Poll::Pending
            }
        })
        .await
    }};
    ($f0:expr, $f1:expr, $f2:expr, $f3:expr, $f4:expr $(,)?) => {{
        use ::std::future::Future as _;
        let mut __f0 = ::std::pin::pin!($f0);
        let mut __f1 = ::std::pin::pin!($f1);
        let mut __f2 = ::std::pin::pin!($f2);
        let mut __f3 = ::std::pin::pin!($f3);
        let mut __f4 = ::std::pin::pin!($f4);
        let mut __o0 = ::std::option::Option::None;
        let mut __o1 = ::std::option::Option::None;
        let mut __o2 = ::std::option::Option::None;
        let mut __o3 = ::std::option::Option::None;
        let mut __o4 = ::std::option::Option::None;
        ::std::future::poll_fn(move |cx| {
            if __o0.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f0.as_mut().poll(cx) {
                    match __out {
                        ::std::result::Result::Ok(__v) => __o0 = ::std::option::Option::Some(__v),
                        ::std::result::Result::Err(__e) => {
                            return ::std::task::Poll::Ready(::std::result::Result::Err(__e))
                        }
                    }
                }
            }
            if __o1.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f1.as_mut().poll(cx) {
                    match __out {
                        ::std::result::Result::Ok(__v) => __o1 = ::std::option::Option::Some(__v),
                        ::std::result::Result::Err(__e) => {
                            return ::std::task::Poll::Ready(::std::result::Result::Err(__e))
                        }
                    }
                }
            }
            if __o2.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f2.as_mut().poll(cx) {
                    match __out {
                        ::std::result::Result::Ok(__v) => __o2 = ::std::option::Option::Some(__v),
                        ::std::result::Result::Err(__e) => {
                            return ::std::task::Poll::Ready(::std::result::Result::Err(__e))
                        }
                    }
                }
            }
            if __o3.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f3.as_mut().poll(cx) {
                    match __out {
                        ::std::result::Result::Ok(__v) => __o3 = ::std::option::Option::Some(__v),
                        ::std::result::Result::Err(__e) => {
                            return ::std::task::Poll::Ready(::std::result::Result::Err(__e))
                        }
                    }
                }
            }
            if __o4.is_none() {
                if let ::std::task::Poll::Ready(__out) = __f4.as_mut().poll(cx) {
                    match __out {
                        ::std::result::Result::Ok(__v) => __o4 = ::std::option::Option::Some(__v),
                        ::std::result::Result::Err(__e) => {
                            return ::std::task::Poll::Ready(::std::result::Result::Err(__e))
                        }
                    }
                }
            }
            if __o0.is_some()
                && __o1.is_some()
                && __o2.is_some()
                && __o3.is_some()
                && __o4.is_some()
            {
                ::std::task::Poll::Ready(::std::result::Result::Ok((
                    __o0.take().unwrap(),
                    __o1.take().unwrap(),
                    __o2.take().unwrap(),
                    __o3.take().unwrap(),
                    __o4.take().unwrap(),
                )))
            } else {
                ::std::task::Poll::Pending
            }
        })
        .await
    }};
}

/// Declares one or more task-local values -- see
/// [`crate::task::LocalKey`]'s module docs for exactly what "task-local"
/// means here and when a scoped value is actually visible.
///
/// ```
/// # use rusty_tokio::Runtime;
/// # let rt = Runtime::new().unwrap();
/// rusty_tokio::task_local! {
///     static REQUEST_ID: u32;
/// }
///
/// # rt.block_on(async {
/// REQUEST_ID
///     .scope(42, async {
///         assert_eq!(REQUEST_ID.with(|id| *id), 42);
///     })
///     .await;
///
/// // Outside of that scope, the value is gone again.
/// assert!(REQUEST_ID.try_with(|id| *id).is_err());
/// # });
/// ```
#[macro_export]
macro_rules! task_local {
    () => {};

    ($(#[$attr:meta])* $vis:vis static $name:ident: $ty:ty; $($rest:tt)*) => {
        $(#[$attr])*
        $vis static $name: $crate::task::LocalKey<$ty> = {
            ::std::thread_local! {
                static __KEY: ::std::cell::RefCell<::std::option::Option<$ty>> =
                    const { ::std::cell::RefCell::new(::std::option::Option::None) };
            }
            $crate::task::LocalKey { __inner: __KEY }
        };
        $crate::task_local! { $($rest)* }
    };
}
