//! Handlers: async functions whose arguments are extractors.

use std::future::Future;
use std::sync::Arc;

use crate::extract::FromRequest;
use crate::request::Request;
use crate::response::{IntoResponse, Response};
use crate::BoxFuture;

/// An async function usable as a route: up to eight [`FromRequest`]
/// arguments, returning anything [`IntoResponse`]. `T` is the argument
/// tuple, only there to tell the implementations apart.
pub trait Handler<T, S>: Clone + Send + Sync + 'static {
    /// Extracts the arguments and calls the function. A failed extraction
    /// answers with its rejection without calling it.
    fn call(self, req: Request, state: &S) -> BoxFuture<'static, Response>;
}

/// A handler with its argument types erased, as the router stores it.
pub(crate) type BoxHandler<S> =
    Arc<dyn Fn(Request, &S) -> BoxFuture<'static, Response> + Send + Sync>;

pub(crate) fn erase<H, T, S>(handler: H) -> BoxHandler<S>
where
    H: Handler<T, S>,
    T: 'static,
{
    Arc::new(move |req, state| handler.clone().call(req, state))
}

macro_rules! impl_handler {
    ($($arg:ident),*) => {
        #[allow(non_snake_case, unused_mut, unused_variables)]
        impl<F, Fut, R, S, $($arg,)*> Handler<($($arg,)*), S> for F
        where
            F: FnOnce($($arg),*) -> Fut + Clone + Send + Sync + 'static,
            Fut: Future<Output = R> + Send + 'static,
            R: IntoResponse,
            $($arg: FromRequest<S> + Send + 'static,)*
        {
            fn call(self, mut req: Request, state: &S) -> BoxFuture<'static, Response> {
                $(
                    let $arg = match $arg::from_request(&mut req, state) {
                        Ok(value) => value,
                        Err(rejection) => {
                            let response = rejection.into_response();
                            return Box::pin(async move { response });
                        }
                    };
                )*
                Box::pin(async move { self($($arg),*).await.into_response() })
            }
        }
    };
}

impl_handler!();
impl_handler!(T1);
impl_handler!(T1, T2);
impl_handler!(T1, T2, T3);
impl_handler!(T1, T2, T3, T4);
impl_handler!(T1, T2, T3, T4, T5);
impl_handler!(T1, T2, T3, T4, T5, T6);
impl_handler!(T1, T2, T3, T4, T5, T6, T7);
impl_handler!(T1, T2, T3, T4, T5, T6, T7, T8);
