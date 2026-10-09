//! An async face for the blocking [`Client`], with no async runtime of its
//! own: the client lives on a worker thread, each call is a message to it, and
//! the future resolves when the worker answers. Works on any executor.
//!
//! - **Order.** Calls run one at a time, in the order they were issued. Many
//!   tasks may share an [`AsyncClient`] (it is cheap to clone via `Arc`); their
//!   calls queue.
//! - **Dropping a future** does not stop its call (the worker is blocked in
//!   it); the answer is discarded. The client's own call timeout bounds it.
//! - **Notifications** reach the [`Handler`] on the worker thread, also while
//!   no call is running: the worker polls the transport when idle.
//! - **Closing.** Dropping the `AsyncClient` ends the worker, which drops the
//!   client and so closes the transport (child stopped, HTTP session ended).

use crate::client::{Client, Handler};
use crate::error::ClientError;
use crate::session::ClientConfig;
use crate::transport::Transport;
use rusty_json::Value;
use rusty_mcp_proto::{
    CallToolResponse, CallToolResult, CompleteParams, CompleteResult, GetPromptResult,
    Implementation, Prompt, ProtocolVersion, ReadResourceResult, Resource, ResourceTemplate,
    ServerCapabilities, Tool,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};
use std::thread::JoinHandle;
use std::time::Duration;

/// How long an idle worker waits for a command before polling the transport
/// for notifications.
const IDLE: Duration = Duration::from_millis(50);
/// How long that poll waits for a message.
const IDLE_POLL: Duration = Duration::from_millis(5);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Slot<R> {
    value: Option<Result<R, ClientError>>,
    waker: Option<Waker>,
}

/// The eventual answer to one call.
pub struct Reply<R>(Arc<Mutex<Slot<R>>>);

/// The worker's end of a [`Reply`]. If it is dropped unanswered (the worker
/// ended with the call still queued), the call fails with
/// [`ClientError::Closed`] instead of hanging.
struct Fill<R>(Arc<Mutex<Slot<R>>>, bool);

impl<R> Fill<R> {
    fn set(mut self, value: Result<R, ClientError>) {
        self.1 = true;
        self.deliver(value);
    }

    fn deliver(&self, value: Result<R, ClientError>) {
        let mut slot = lock(&self.0);
        slot.value = Some(value);
        if let Some(waker) = slot.waker.take() {
            waker.wake();
        }
    }
}

impl<R> Drop for Fill<R> {
    fn drop(&mut self) {
        if !self.1 {
            self.deliver(Err(ClientError::Closed));
        }
    }
}

fn reply<R>() -> (Fill<R>, Reply<R>) {
    let slot = Arc::new(Mutex::new(Slot {
        value: None,
        waker: None,
    }));
    (Fill(Arc::clone(&slot), false), Reply(slot))
}

impl<R> Future for Reply<R> {
    type Output = Result<R, ClientError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut slot = lock(&self.0);
        match slot.value.take() {
            Some(v) => Poll::Ready(v),
            None => {
                slot.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// What the worker keeps in view of the handshake.
struct Facts {
    negotiated: Option<ProtocolVersion>,
    server_info: Option<Implementation>,
    server_capabilities: Option<ServerCapabilities>,
    instructions: Option<String>,
}

type Job<T, H> = Box<dyn FnOnce(&mut Client<T, H>) + Send>;

/// A connected client, usable from async code.
pub struct AsyncClient<T: Transport + Send + 'static, H: Handler + Send + 'static> {
    jobs: Option<Sender<Job<T, H>>>,
    worker: Option<JoinHandle<()>>,
    facts: Facts,
}

impl<T: Transport + Send + 'static, H: Handler + Send + 'static> AsyncClient<T, H> {
    /// Shake hands with the server on a new worker thread.
    ///
    /// # Errors
    /// As [`Client::connect`]; or the worker thread could not start.
    pub async fn connect(
        transport: T,
        config: ClientConfig,
        handler: H,
        timeout: Duration,
    ) -> Result<Self, ClientError> {
        let (jobs, inbox) = channel::<Job<T, H>>();
        let (fill, answer) = reply::<Facts>();
        let worker = std::thread::Builder::new()
            .name("mcp-client".to_owned())
            .spawn(move || {
                let mut client = match Client::connect(transport, config, handler, timeout) {
                    Ok(c) => c,
                    Err(e) => return fill.set(Err(e)),
                };
                let s = client.session();
                fill.set(Ok(Facts {
                    negotiated: s.negotiated().cloned(),
                    server_info: s.server_info().cloned(),
                    server_capabilities: s.server_capabilities().cloned(),
                    instructions: s.instructions().map(str::to_owned),
                }));
                loop {
                    match inbox.recv_timeout(IDLE) {
                        Ok(job) => job(&mut client),
                        Err(RecvTimeoutError::Timeout) => {
                            // Between calls, let notifications through.
                            if client.pump(IDLE_POLL).is_err() {
                                return;
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .map_err(ClientError::Io)?;
        let facts = answer.await?;
        Ok(Self {
            jobs: Some(jobs),
            worker: Some(worker),
            facts,
        })
    }

    /// The revision in force.
    pub fn negotiated(&self) -> Option<&ProtocolVersion> {
        self.facts.negotiated.as_ref()
    }

    /// Who the server says it is.
    pub fn server_info(&self) -> Option<&Implementation> {
        self.facts.server_info.as_ref()
    }

    /// What the server says it can do.
    pub fn server_capabilities(&self) -> Option<&ServerCapabilities> {
        self.facts.server_capabilities.as_ref()
    }

    /// The server's usage hints.
    pub fn instructions(&self) -> Option<&str> {
        self.facts.instructions.as_deref()
    }

    /// Run `f` on the worker with the client and await its result.
    fn run<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Client<T, H>) -> Result<R, ClientError> + Send + 'static,
    ) -> Reply<R> {
        let (fill, answer) = reply();
        let job: Job<T, H> = Box::new(move |client| fill.set(f(client)));
        // If the worker is gone the job is dropped here, and with it `fill`,
        // which fails the call with `Closed`.
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(job);
        }
        answer
    }

    /// Any request, answered with its raw result.
    ///
    /// # Errors
    /// As [`Client::call`].
    pub fn call(&self, method: &str, params: Option<Value>) -> Reply<Value> {
        let method = method.to_owned();
        self.run(move |c| c.call(&method, params))
    }

    /// Any notification.
    ///
    /// # Errors
    /// As [`Client::notify`].
    pub fn notify(&self, method: &str, params: Option<Value>) -> Reply<()> {
        let method = method.to_owned();
        self.run(move |c| c.notify(&method, params))
    }

    /// `ping`.
    pub fn ping(&self) -> Reply<()> {
        self.run(Client::ping)
    }

    /// Every tool, following the pages.
    pub fn list_tools(&self) -> Reply<Vec<Tool>> {
        self.run(Client::list_tools)
    }

    /// Every prompt.
    pub fn list_prompts(&self) -> Reply<Vec<Prompt>> {
        self.run(Client::list_prompts)
    }

    /// Every resource.
    pub fn list_resources(&self) -> Reply<Vec<Resource>> {
        self.run(Client::list_resources)
    }

    /// Every resource template.
    pub fn list_resource_templates(&self) -> Reply<Vec<ResourceTemplate>> {
        self.run(Client::list_resource_templates)
    }

    /// `prompts/get`.
    pub fn get_prompt(&self, name: &str, arguments: Option<Value>) -> Reply<GetPromptResult> {
        let name = name.to_owned();
        self.run(move |c| c.get_prompt(&name, arguments))
    }

    /// `resources/read`.
    pub fn read_resource(&self, uri: &str) -> Reply<ReadResourceResult> {
        let uri = uri.to_owned();
        self.run(move |c| c.read_resource(&uri))
    }

    /// `completion/complete`.
    pub fn complete(&self, params: CompleteParams) -> Reply<CompleteResult> {
        self.run(move |c| c.complete(&params))
    }

    /// One `tools/call`, answered as the server answered.
    pub fn call_tool_once(&self, name: &str, arguments: Option<Value>) -> Reply<CallToolResponse> {
        let name = name.to_owned();
        self.run(move |c| c.call_tool_once(&name, arguments))
    }

    /// `tools/call` driven to its end: questions answered by the handler,
    /// tasks polled.
    pub fn call_tool(&self, name: &str, arguments: Option<Value>) -> Reply<CallToolResult> {
        let name = name.to_owned();
        self.run(move |c| c.call_tool(&name, arguments))
    }
}

impl<T: Transport + Send + 'static, H: Handler + Send + 'static> Drop for AsyncClient<T, H> {
    /// End the worker (after the call it is in, if any) and close the
    /// transport.
    fn drop(&mut self) {
        self.jobs = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
