//! The two ends a streaming handler works with, and the response wire they share.

use std::future::Future;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex as StdMutex};

use rusty_h2::driver::{Body, RecvStream, Respond, SendStream};
use rusty_hyper::HeaderField;
use rusty_prost::Message;
use rusty_tokio::sync::Mutex;

use crate::client::{frame, status_from_h2 as client_status};
use crate::{decode_message, FrameDecoder, MetadataMap, ResponseHead, Status};

/// The status for a failed read or write. A stream that is gone while the handler still uses it
/// is one the client cancelled (`rusty_h2` forgets a reset stream whose request was all read).
fn write_status(e: &rusty_hyper::H2Error) -> Status {
    match e {
        rusty_hyper::H2Error::Closed => Status::cancelled("the client is gone"),
        other => client_status(other),
    }
}

pub(super) fn fields(pairs: Vec<(String, Vec<u8>)>) -> Vec<HeaderField> {
    pairs
        .iter()
        .map(|(name, value)| HeaderField::new(name.as_str(), value.as_slice()))
        .collect()
}

/// The request body as whole messages, undecoded.
pub(super) struct RawIncoming {
    body: RecvStream,
    decoder: FrameDecoder,
    eof: bool,
    /// How reading ended, once it has.
    end: Option<Result<(), Status>>,
}

impl RawIncoming {
    pub(super) fn new(body: RecvStream, max_recv: usize) -> RawIncoming {
        RawIncoming {
            body,
            decoder: FrameDecoder::new(max_recv),
            eof: false,
            end: None,
        }
    }

    /// The next whole message; `Ok(None)` once the client finished sending; the same error
    /// again after a failure.
    pub(super) async fn next(&mut self) -> Result<Option<Vec<u8>>, Status> {
        loop {
            match self.decoder.next_message() {
                Ok(Some(bytes)) => return Ok(Some(bytes)),
                Ok(None) => {}
                Err(status) => return Err(self.fail(status)),
            }
            if let Some(end) = &self.end {
                return end.clone().map(|()| None);
            }
            if self.eof {
                // Everything is delivered; a half message left over is a broken request.
                self.end = Some(self.decoder.finish());
                continue;
            }
            match self.body.next().await {
                Some(Ok(Body::Data(bytes))) => self.decoder.push(&bytes),
                Some(Ok(Body::Trailers(_))) => {}
                Some(Err(e)) => {
                    let status = write_status(&e);
                    return Err(self.fail(status));
                }
                None => self.eof = true,
            }
        }
    }

    fn fail(&mut self, status: Status) -> Status {
        self.end = Some(Err(status.clone()));
        status
    }

    /// The one message of a call that has exactly one.
    pub(super) async fn single<T: Message>(&mut self) -> Result<T, Status> {
        let Some(bytes) = self.next().await? else {
            return Err(Status::internal("the request has no message"));
        };
        if self.next().await?.is_some() {
            return Err(Status::internal("the request has more than one message"));
        }
        decode_message(&bytes)
    }
}

/// The request messages of a call, read one at a time.
pub struct Incoming<T> {
    raw: RawIncoming,
    _t: PhantomData<fn() -> T>,
}

impl<T: Message> Incoming<T> {
    pub(super) fn new(raw: RawIncoming) -> Incoming<T> {
        Incoming {
            raw,
            _t: PhantomData,
        }
    }

    /// The next message; `Ok(None)` once the client has finished sending. `CANCELLED` if the
    /// client cancelled; `RESOURCE_EXHAUSTED` for a message over the receive limit.
    pub async fn message(&mut self) -> Result<Option<T>, Status> {
        match self.raw.next().await? {
            None => Ok(None),
            Some(bytes) => match decode_message(&bytes) {
                Ok(message) => Ok(Some(message)),
                Err(status) => Err(self.raw.fail(status)),
            },
        }
    }
}

/// The response before and after its headers.
pub(super) struct Wire {
    respond: Option<Respond>,
    send: Option<SendStream>,
}

/// What a call's handler and the code around it share.
pub(super) struct Shared {
    /// Initial metadata, then trailers; set from the handler, read once to finish the call.
    meta: StdMutex<(MetadataMap, MetadataMap)>,
    wire: Mutex<Wire>,
    max_send: Option<usize>,
}

impl Shared {
    pub(super) fn new(respond: Respond, max_send: Option<usize>) -> Arc<Shared> {
        Arc::new(Shared {
            meta: StdMutex::new((MetadataMap::new(), MetadataMap::new())),
            wire: Mutex::new(Wire {
                respond: Some(respond),
                send: None,
            }),
            max_send,
        })
    }

    pub(super) fn set_metadata(&self, metadata: MetadataMap) {
        if let Ok(mut meta) = self.meta.lock() {
            meta.0 = metadata;
        }
    }

    pub(super) fn set_trailers(&self, trailers: MetadataMap) {
        if let Ok(mut meta) = self.meta.lock() {
            meta.1 = trailers;
        }
    }

    fn take_trailers(&self) -> MetadataMap {
        self.meta
            .lock()
            .map(|mut meta| std::mem::take(&mut meta.1))
            .unwrap_or_default()
    }

    /// A reply message framed, or `RESOURCE_EXHAUSTED` if over the send limit.
    pub(super) fn encode<T: Message>(&self, message: &T) -> Result<Vec<u8>, Status> {
        frame(message, self.max_send)
    }

    /// Sends one framed reply message, starting the response first if it has not been.
    pub(super) async fn send_frame(&self, data: Vec<u8>) -> Result<(), Status> {
        let mut wire = self.wire.lock().await;
        if wire.send.is_none() {
            let respond = wire
                .respond
                .take()
                .ok_or_else(|| Status::internal("the response has already ended"))?;
            let initial = self.take_initial();
            let head = ResponseHead {
                metadata: initial,
                metadata_dropped: 0,
            };
            let send = respond
                .send_response(fields(head.to_headers()), false)
                .await
                .map_err(|e| write_status(&e))?;
            wire.send = Some(send);
        }
        match wire.send.as_mut() {
            Some(send) => send
                .send_data(data, false)
                .await
                .map_err(|e| write_status(&e)),
            None => Err(Status::internal("the response has not started")),
        }
    }

    fn take_initial(&self) -> MetadataMap {
        self.meta
            .lock()
            .map(|mut meta| std::mem::take(&mut meta.0))
            .unwrap_or_default()
    }

    /// Ends the call with `status`: trailers after the replies, or the whole response as one
    /// header block if nothing was sent. Failures to write mean the client is gone.
    pub(super) async fn finish(&self, status: Status) {
        let mut wire = self.wire.lock().await;
        let trailers = self.take_trailers();
        let status = if status.code() == crate::Code::Ok {
            status.with_metadata(trailers)
        } else {
            status
        };
        if let Some(mut send) = wire.send.take() {
            // A write that fails means the client cancelled; there is no one to tell.
            let _ = send.send_trailers(fields(status.to_trailers())).await;
        } else if let Some(respond) = wire.respond.take() {
            let _ = trailers_only(respond, &status).await;
        }
    }
}

/// A "trailers-only" response: the call is over in its first headers.
pub(super) async fn trailers_only(
    respond: Respond,
    status: &Status,
) -> Result<SendStream, rusty_hyper::H2Error> {
    let mut headers = vec![
        (":status".to_string(), b"200".to_vec()),
        ("content-type".to_string(), b"application/grpc".to_vec()),
    ];
    headers.extend(status.to_trailers());
    respond.send_response(fields(headers), true).await
}

/// Where a streaming handler writes its replies.
pub struct Sink<T> {
    shared: Arc<Shared>,
    _t: PhantomData<fn(T)>,
}

impl<T: Message> Sink<T> {
    pub(super) fn new(shared: Arc<Shared>) -> Sink<T> {
        Sink {
            shared,
            _t: PhantomData,
        }
    }

    /// The initial metadata, sent with the first message (or with the status if there is none).
    /// Ignored once something has been sent.
    pub fn set_metadata(&mut self, metadata: MetadataMap) {
        self.shared.set_metadata(metadata);
    }

    /// The trailing metadata, sent with the final status when the call ends `OK`; an error
    /// status carries its own metadata.
    pub fn set_trailers(&mut self, trailers: MetadataMap) {
        self.shared.set_trailers(trailers);
    }

    /// Sends one reply, waiting for flow-control capacity. `CANCELLED` once the client is gone;
    /// `RESOURCE_EXHAUSTED` for a message over the send limit (nothing is sent).
    pub fn send(&mut self, message: &T) -> impl Future<Output = Result<(), Status>> + Send + '_ {
        // Framed now, so the future does not hold the message.
        let data = self.shared.encode(message);
        async move { self.shared.send_frame(data?).await }
    }
}
