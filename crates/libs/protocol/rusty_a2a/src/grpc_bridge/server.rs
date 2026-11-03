//! The server half of the bridge: the engine runs on the application's `tokio` runtime while the
//! handlers that call it run on the transport's.
//!
//! A handler hands the engine call to `tokio` with [`on_app`], and a stream the engine returns to
//! [`feed`]; both are awaited from the transport side. When the client leaves, the transport drops
//! the handler, which drops these, which abort the `tokio` tasks: the engine call stops waiting.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_core::Stream;
use futures_util::StreamExt;
use rusty_hyper::ServeOptions;
use rusty_tonic::server::Router;
use rusty_tonic::Status;

/// Items read ahead of a slow transport. HTTP/2 flow control pushes back on the peer beyond it.
const PREFETCH: usize = 16;

/// A `tokio` task, aborted when dropped.
struct AppTask<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AppTask<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl<T> Future for AppTask<T> {
    type Output = Result<T, Status>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(cx).map(|joined| {
            joined.map_err(|e| {
                // A task cancelled by its runtime shutting down is not the caller's fault.
                if e.is_cancelled() {
                    Status::unavailable("the engine was shut down")
                } else {
                    Status::internal(format!("the engine call failed: {e}"))
                }
            })
        })
    }
}

/// Runs `fut` on the application's runtime and waits for it. Dropping the wait aborts it.
pub(crate) async fn on_app<T, F>(app: &tokio::runtime::Handle, fut: F) -> Result<T, Status>
where
    F: Future<Output = Result<T, Status>> + Send + 'static,
    T: Send + 'static,
{
    AppTask(app.spawn(fut)).await?
}

/// Items produced by a stream that runs on the application's runtime, read from the transport's.
/// Dropping it stops the producer.
pub(crate) struct Fed<T> {
    rx: tokio::sync::mpsc::Receiver<T>,
    task: AppTask<()>,
    ended: bool,
}

impl<T> Fed<T> {
    /// The next item, in order; `Ok(None)` once the stream has ended. A closed channel is an end
    /// only if the producer finished: one that was cancelled (its runtime shut down) or
    /// panicked is an error, so a truncated stream is never reported as complete.
    pub(crate) async fn next(&mut self) -> Result<Option<T>, Status> {
        if self.ended {
            return Ok(None);
        }
        if let Some(item) = self.rx.recv().await {
            return Ok(Some(item));
        }
        self.ended = true;
        (&mut self.task).await.map(|()| None)
    }
}

/// Starts driving `stream` on the application's runtime.
pub(crate) fn feed<S, T>(app: &tokio::runtime::Handle, stream: S) -> Fed<T>
where
    S: Stream<Item = T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel(PREFETCH);
    let task = AppTask(app.spawn(async move {
        let mut stream = std::pin::pin!(stream);
        while let Some(item) = stream.next().await {
            // A closed channel means the transport dropped the call: stop producing.
            if tx.send(item).await.is_err() {
                return;
            }
        }
    }));
    Fed {
        rx,
        task,
        ended: false,
    }
}

/// How long a stopped server may wait for the calls in flight, and then its runtime for its tasks.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// Keeps a server and its runtime running; dropping it stops them.
struct Serving {
    runtime: Option<rusty_tokio::Runtime>,
    server: Option<rusty_hyper::Server>,
}

impl Drop for Serving {
    fn drop(&mut self) {
        let (Some(runtime), Some(server)) = (self.runtime.take(), self.server.take()) else {
            return;
        };
        // The server stops accepting and closes its listener only while its runtime is still
        // running, and a graceful stop waits for the calls in flight. This is dropped on an
        // application thread that must not block, so the wait happens on a thread of its own,
        // bounded so that a stalled call cannot keep it for ever. If that thread cannot be
        // started, the closure and the runtime are dropped here, which stops them (blocking).
        let _ = std::thread::Builder::new()
            .name("rusty_a2a-grpc-stop".into())
            .spawn(move || {
                runtime.block_on(async {
                    let _ = rusty_tokio::time::timeout(STOP_GRACE, server.shutdown()).await;
                });
                runtime.shutdown_timeout(STOP_GRACE);
            });
    }
}

/// Serves `router` on `listener`, on a runtime of its own, until the returned future is dropped.
///
/// Fails if the listener cannot be taken over; otherwise it does not return. Dropping it stops
/// the server and then the runtime, on a thread of its own (this is awaited from `tokio`, where
/// blocking to wait for the calls in flight would stall the application).
pub(crate) async fn serve(router: Router, listener: std::net::TcpListener) -> std::io::Result<()> {
    let runtime = rusty_tokio::Runtime::new()?;
    let started = {
        let _enter = runtime.enter();
        rusty_tokio::io::TcpListener::from_std(listener)
            .and_then(|listener| router.serve(listener, None, ServeOptions::default()))
    };
    let server = match started {
        Ok(server) => server,
        Err(e) => {
            runtime.shutdown_background();
            return Err(e);
        }
    };
    let _serving = Serving {
        runtime: Some(runtime),
        server: Some(server),
    };
    std::future::pending::<()>().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use futures_util::StreamExt;
    use rusty_tonic::{Code, Status};

    use super::{feed, on_app};

    /// An application runtime (`tokio`) and a transport runtime (`rusty_tokio`), as in a running
    /// server: handlers are awaited on the second and the engine runs on the first.
    fn runtimes() -> (tokio::runtime::Runtime, rusty_tokio::Runtime) {
        let app = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        (app, rusty_tokio::Runtime::new().unwrap())
    }

    /// Sets a flag when dropped.
    struct Dropped(Arc<AtomicBool>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn set_soon(flag: &AtomicBool) -> bool {
        for _ in 0..200 {
            if flag.load(Ordering::SeqCst) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn the_application_future_has_tokios_spawn_and_timers_though_the_transport_awaits_it() {
        let (app, ours) = runtimes();
        let out = ours.block_on(on_app(app.handle(), async {
            tokio::time::sleep(Duration::from_millis(5)).await;
            Ok(tokio::spawn(async { 7 }).await.unwrap())
        }));
        assert_eq!(out.unwrap(), 7);
    }

    #[test]
    fn an_error_from_the_application_future_comes_back_as_it_was() {
        let (app, ours) = runtimes();
        let out: Result<(), Status> = ours.block_on(on_app(app.handle(), async {
            Err(Status::not_found("no such task"))
        }));
        let status = out.unwrap_err();
        assert_eq!(status.code(), Code::NotFound);
        assert_eq!(status.message(), "no such task");
    }

    #[test]
    fn dropping_the_wait_aborts_the_application_future() {
        let (app, ours) = runtimes();
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Dropped(dropped.clone());
        ours.block_on(async {
            let call = on_app(app.handle(), async move {
                let _guard = guard;
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(())
            });
            // The transport gives up on the call: its client left.
            assert!(rusty_tokio::time::timeout(Duration::from_millis(100), call)
                .await
                .is_err());
        });
        assert!(set_soon(&dropped), "the engine call kept running");
    }

    #[test]
    fn a_panic_in_the_application_future_is_an_internal_error() {
        let (app, ours) = runtimes();
        let out: Result<(), Status> = ours.block_on(on_app(app.handle(), async {
            if true {
                panic!("boom");
            }
            Ok(())
        }));
        assert_eq!(out.unwrap_err().code(), Code::Internal);
    }

    #[test]
    fn a_fed_stream_arrives_in_order_and_ends() {
        let (app, ours) = runtimes();
        let items = (0..50).map(Ok::<u32, Status>);
        let got = ours.block_on(async {
            let mut fed = feed(app.handle(), futures_util::stream::iter(items));
            let mut got = Vec::new();
            while let Some(item) = fed.next().await.unwrap() {
                got.push(item.unwrap());
            }
            got
        });
        assert_eq!(got, (0..50).collect::<Vec<u32>>());
    }

    #[test]
    fn an_error_item_is_delivered_and_the_stream_goes_on_to_end() {
        let (app, ours) = runtimes();
        let items = vec![Ok(1u32), Err(Status::aborted("boom")), Ok(3)];
        let got = ours.block_on(async {
            let mut fed = feed(app.handle(), futures_util::stream::iter(items));
            let mut got = Vec::new();
            while let Some(item) = fed.next().await.unwrap() {
                got.push(item.map_err(|s| s.code()));
            }
            got
        });
        assert_eq!(got, vec![Ok(1), Err(Code::Aborted), Ok(3)]);
    }

    #[test]
    fn dropping_a_fed_stream_stops_its_producer() {
        let (app, ours) = runtimes();
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Dropped(dropped.clone());
        // Produces for ever, one item every 10 ms.
        let producer = futures_util::stream::unfold((guard, 0u32), |(guard, n)| async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            Some((Ok::<u32, Status>(n), (guard, n + 1)))
        })
        .boxed();
        ours.block_on(async {
            let mut fed = feed(app.handle(), producer);
            assert!(fed.next().await.unwrap().is_some());
            drop(fed);
        });
        assert!(set_soon(&dropped), "the producer kept running");
    }

    /// One item, then a producer that never finishes.
    fn one_then_pending() -> impl futures_util::Stream<Item = Result<u32, Status>> + Send {
        futures_util::stream::iter([Ok(1u32)]).chain(futures_util::stream::pending())
    }

    #[test]
    fn a_producer_cancelled_by_its_runtime_shutting_down_is_an_error_not_an_end() {
        let (app, ours) = runtimes();
        let mut fed = feed(app.handle(), one_then_pending());
        let first = ours.block_on(fed.next()).unwrap();
        assert_eq!(first.unwrap().unwrap(), 1);
        // The application stops while the transport stays up.
        drop(app);
        let end = ours.block_on(async {
            rusty_tokio::time::timeout(Duration::from_secs(5), fed.next())
                .await
                .expect("a stopped producer must end the stream, not hang it")
        });
        assert_eq!(end.unwrap_err().code(), Code::Unavailable);
        // Asked again, it has ended.
        assert!(ours.block_on(fed.next()).unwrap().is_none());
    }

    #[test]
    fn a_panicking_producer_is_an_internal_error_after_its_buffered_items() {
        let (app, ours) = runtimes();
        let producer = futures_util::stream::unfold(0u32, |n| async move {
            if n == 2 {
                panic!("boom");
            }
            Some((Ok::<u32, Status>(n), n + 1))
        });
        let (items, end) = ours.block_on(async {
            let mut fed = feed(app.handle(), producer);
            let mut items = Vec::new();
            let end = loop {
                match fed.next().await {
                    Ok(Some(item)) => items.push(item.unwrap()),
                    other => break other,
                }
            };
            (items, end)
        });
        assert_eq!(items, vec![0, 1]);
        assert_eq!(end.unwrap_err().code(), Code::Internal);
    }
}
