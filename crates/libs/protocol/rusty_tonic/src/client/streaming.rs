//! The ends of a streaming call.

use std::future::Future;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rusty_h2::driver::{Body, RecvStream, ResponseFuture, SendStream};
use rusty_h2::error::ErrorCode;
use rusty_hyper::HeaderField;
use rusty_prost::Message;
use rusty_tokio::select;
use rusty_tokio::sync::watch;
use rusty_tokio::time::timeout_at;

use super::error::status_from_h2;
use super::Response;
use crate::{
    decode_message, encode_message, FrameDecoder, MetadataMap, ResponseHead, ResponseStart, Status,
    HEADER_LEN,
};

/// Runs `f` until `deadline`; `None` if the deadline came first.
pub(crate) async fn until<F: Future>(deadline: Option<Instant>, f: F) -> Option<F::Output> {
    match deadline {
        None => Some(f.await),
        Some(at) => timeout_at(at, f).await.ok(),
    }
}

fn deadline_exceeded() -> Status {
    Status::deadline_exceeded("the call's deadline passed")
}

/// A header list as the borrowed pairs the core's parsers take.
fn pairs(headers: &[HeaderField]) -> Vec<(&str, &[u8])> {
    headers
        .iter()
        .filter_map(|h| Some((std::str::from_utf8(&h.name).ok()?, h.value.as_slice())))
        .collect()
}

/// The longest message a call may send, checked against the message about to be framed.
pub(crate) fn frame<T: Message>(message: &T, max: Option<usize>) -> Result<Vec<u8>, Status> {
    let frame = encode_message(message)?;
    match max {
        Some(max) if frame.len() - HEADER_LEN > max => Err(Status::resource_exhausted(format!(
            "the message is {} bytes, over the send limit of {max}",
            frame.len() - HEADER_LEN
        ))),
        _ => Ok(frame),
    }
}

/// Where a finished [`Sender`] leaves its stream so the call can still be cancelled: the
/// stream once the requests are all sent, and whether the call was cancelled before that.
#[derive(Default)]
pub(super) struct Handoff {
    cancelled: bool,
    send: Option<SendStream>,
}

pub(super) type Slot = Arc<Mutex<Handoff>>;

/// Locks the slot. Nothing under the lock can panic, so a poisoned lock still holds a valid
/// value and is used as it is.
fn lock(slot: &Slot) -> std::sync::MutexGuard<'_, Handoff> {
    slot.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// How a call is cancelled: a stream whose requests are already sent is reset directly. One
/// still sending is told through a signal, so its [`Sender`] resets it; a [`Sender`] that has
/// finished has left its stream in the slot, and that is reset from here.
pub(super) enum Cancel {
    Reset(SendStream),
    Signal(watch::Sender<bool>, Slot),
}

impl Cancel {
    fn cancel(&mut self) {
        match self {
            Cancel::Reset(send) => send.reset(ErrorCode::Cancel),
            Cancel::Signal(signal, slot) => {
                // A send fails only when the sender is already gone, which is the aim.
                let _ = signal.send(true);
                let mut handoff = lock(slot);
                handoff.cancelled = true;
                if let Some(send) = handoff.send.as_mut() {
                    send.reset(ErrorCode::Cancel);
                }
            }
        }
    }
}

/// The replies of a call, read one message at a time. Dropping it before the end cancels the call.
pub struct Streaming<T> {
    metadata: MetadataMap,
    /// The response, until its headers have arrived.
    head: Option<ResponseFuture>,
    body: Option<RecvStream>,
    decoder: FrameDecoder,
    deadline: Option<Instant>,
    cancel: Cancel,
    /// How the server ended the call, held until the messages before it are delivered.
    ending: Option<Status>,
    /// How the call ended, once everything before has been delivered.
    end: Option<Result<(), Status>>,
    trailers: Option<MetadataMap>,
    _t: PhantomData<fn() -> T>,
}

impl<T> Streaming<T> {
    pub(super) fn new(
        head: ResponseFuture,
        max_recv: usize,
        deadline: Option<Instant>,
        cancel: Cancel,
    ) -> Streaming<T> {
        Streaming {
            metadata: MetadataMap::new(),
            head: Some(head),
            body: None,
            decoder: FrameDecoder::new(max_recv),
            deadline,
            cancel,
            ending: None,
            end: None,
            trailers: None,
            _t: PhantomData,
        }
    }

    /// Ends the call with `status` and cancels the stream.
    fn fail(&mut self, status: Status) -> Status {
        self.cancel.cancel();
        self.head = None;
        self.body = None;
        self.end = Some(Err(status.clone()));
        status
    }

    /// Waits for the response's headers. After it, [`Streaming::metadata`] is the server's.
    pub(super) async fn open(&mut self) -> Result<(), Status> {
        let Some(pending) = self.head.as_mut() else {
            return Ok(());
        };
        // Awaited in place, not taken out: if this future is dropped mid-wait, `Drop` still
        // finds the call open and cancels it.
        let waited = until(self.deadline, pending).await;
        self.head = None;
        let response = match waited {
            None => return Err(self.fail(deadline_exceeded())),
            Some(Err(e)) => return Err(self.fail(status_from_h2(&e))),
            Some(Ok(response)) => response,
        };
        let http_status = pairs(&response.headers)
            .iter()
            .find(|(name, _)| *name == ":status")
            .and_then(|(_, v)| std::str::from_utf8(v).ok()?.parse().ok())
            .unwrap_or(0);
        match ResponseHead::from_headers(http_status, pairs(&response.headers)) {
            Err(status) => Err(self.fail(status)),
            Ok(ResponseStart::TrailersOnly(status)) => {
                self.trailers = Some(status.metadata().clone());
                self.ending = Some(status);
                Ok(())
            }
            Ok(ResponseStart::Headers(head)) => {
                self.metadata = head.metadata;
                self.body = Some(response.body);
                Ok(())
            }
        }
    }

    /// The status of a call whose very first headers ended it unsuccessfully.
    pub(super) fn failed_at_once(&self) -> Option<Status> {
        self.ending.clone().filter(|s| s.code() != crate::Code::Ok)
    }

    /// Reads one more piece of the response into the decoder, or notes how it ended.
    async fn pull(&mut self) {
        let Some(body) = self.body.as_mut() else {
            self.fail(Status::internal("the response has no body"));
            return;
        };
        match until(self.deadline, body.next()).await {
            None => {
                self.fail(deadline_exceeded());
            }
            Some(Some(Ok(Body::Data(bytes)))) => self.decoder.push(&bytes),
            Some(Some(Ok(Body::Trailers(fields)))) => {
                self.body = None;
                let status = Status::from_headers(pairs(&fields)).unwrap_or_else(|| {
                    Status::unknown("the server ended the call without a grpc-status")
                });
                self.trailers = Some(status.metadata().clone());
                self.ending = Some(status);
            }
            Some(Some(Err(e))) => {
                self.fail(status_from_h2(&e));
            }
            Some(None) => {
                self.body = None;
                self.ending = Some(Status::unknown(
                    "the server ended the call without a grpc-status",
                ));
            }
        }
    }
}

impl<T: Message> Streaming<T> {
    /// The initial metadata; empty until the server's headers have arrived, which is before the
    /// first [`message`](Streaming::message) returns.
    pub fn metadata(&self) -> &MetadataMap {
        &self.metadata
    }

    /// The next message; `Ok(None)` once the call ended with status OK; `Err` with the call's
    /// status if it ended any other way, and again on every later call.
    pub async fn message(&mut self) -> Result<Option<T>, Status> {
        self.open().await?;
        loop {
            match self.decoder.next_message() {
                Ok(Some(bytes)) => {
                    return match decode_message::<T>(&bytes) {
                        Ok(message) => Ok(Some(message)),
                        Err(status) => Err(self.fail(status)),
                    }
                }
                Ok(None) => {}
                Err(status) => return Err(self.fail(status)),
            }
            if let Some(end) = &self.end {
                return end.clone().map(|()| None);
            }
            if let Some(status) = self.ending.take() {
                // Everything the server sent is delivered; a half message left over is a
                // broken call even if the server called it a success.
                let result = match (status.code(), self.decoder.finish()) {
                    (crate::Code::Ok, Err(cut)) => Err(cut),
                    (crate::Code::Ok, Ok(())) => Ok(()),
                    _ => Err(status),
                };
                self.end = Some(result);
                continue;
            }
            self.pull().await;
        }
    }

    /// The trailing metadata, once the call has ended.
    pub fn trailers(&self) -> Option<&MetadataMap> {
        self.trailers.as_ref()
    }

    /// The one message of a call that has exactly one, with its metadata and trailers.
    pub(super) async fn single(mut self) -> Result<Response<T>, Status> {
        let message = match self.message().await? {
            Some(message) => message,
            None => return Err(Status::internal("the server sent no message")),
        };
        if self.message().await?.is_some() {
            return Err(self.fail(Status::internal("the server sent more than one message")));
        }
        Ok(Response {
            message,
            metadata: std::mem::take(&mut self.metadata),
            trailers: self.trailers.take().unwrap_or_default(),
        })
    }
}

impl<T> Drop for Streaming<T> {
    fn drop(&mut self) {
        // Nothing to cancel once the server has finished the call.
        if self.end.is_none() && (self.head.is_some() || self.body.is_some()) {
            self.cancel.cancel();
        }
    }
}

/// The sending end of a call that streams requests.
pub struct Sender<T> {
    send: SendStream,
    cancelled: watch::Receiver<bool>,
    slot: Slot,
    max_send: Option<usize>,
    deadline: Option<Instant>,
    _t: PhantomData<fn(T)>,
}

enum Sent {
    Done(Result<(), rusty_hyper::H2Error>),
    Cancelled,
}

impl<T: Message> Sender<T> {
    pub(super) fn new(
        send: SendStream,
        cancelled: watch::Receiver<bool>,
        slot: Slot,
        max_send: Option<usize>,
        deadline: Option<Instant>,
    ) -> Sender<T> {
        Sender {
            send,
            cancelled,
            slot,
            max_send,
            deadline,
            _t: PhantomData,
        }
    }

    async fn write(&mut self, data: Vec<u8>, end: bool) -> Result<(), Status> {
        let outcome = until(self.deadline, async {
            select! {
                biased;
                r = self.send.send_data(data, end) => Sent::Done(r),
                _ = self.cancelled.changed() => Sent::Cancelled,
            }
        })
        .await;
        match outcome {
            Some(Sent::Done(r)) => r.map_err(|e| status_from_h2(&e)),
            Some(Sent::Cancelled) => {
                self.send.reset(ErrorCode::Cancel);
                Err(Status::cancelled("the call has ended"))
            }
            None => {
                self.send.reset(ErrorCode::Cancel);
                Err(deadline_exceeded())
            }
        }
    }

    /// Sends one message, waiting for flow-control capacity. A message over the send limit
    /// is `RESOURCE_EXHAUSTED` and nothing is sent; the call goes on. Once the call has ended
    /// (or been cancelled) it is `CANCELLED`; the call's own status is on the reading end.
    pub async fn send(&mut self, message: &T) -> Result<(), Status> {
        let frame = frame(message, self.max_send)?;
        self.write(frame, false).await
    }

    /// Ends the requests. Without it (the sender dropped) the call is cancelled. After it, the
    /// call can still be cancelled from the reply: dropping it, or its deadline, resets the stream.
    pub async fn finish(mut self) -> Result<(), Status> {
        self.write(Vec::new(), true).await?;
        let Sender { mut send, slot, .. } = self;
        let mut handoff = lock(&slot);
        if handoff.cancelled {
            // Cancelled while the last bytes were going out.
            send.reset(ErrorCode::Cancel);
        } else {
            handoff.send = Some(send);
        }
        Ok(())
    }
}

/// The single reply of a call that streams requests.
pub struct Reply<T> {
    pub(super) streaming: Streaming<T>,
}

impl<T: Message> Reply<T> {
    /// Waits for the reply, its metadata and its trailers.
    pub async fn response(self) -> Result<Response<T>, Status> {
        self.streaming.single().await
    }
}
