//! [`StreamingResponse`], returned by
//! [`crate::RequestBuilder::send_streaming`] -- a response whose body
//! hasn't been read yet, pulled incrementally instead of requiring it
//! all in memory at once.

use std::sync::Arc;

use crate::error::Result;
use crate::pool::{ConnectionPool, PoolKey};
use crate::stream::Conn;
use rusty_http::async_tokio::BodyReader;
use rusty_http::{HeaderMap, StatusCode, Url};

/// Where a connection goes back to once its body has been read whole.
pub(crate) struct Reuse {
    pub(crate) pool: Arc<ConnectionPool>,
    pub(crate) key: PoolKey,
}

/// A response whose body hasn't been read yet. Unlike [`crate::Response`]
/// (always fully buffered), the body here is pulled incrementally via
/// [`StreamingResponse::chunk`] -- useful for a large download that
/// shouldn't have to sit fully in memory before the caller can start
/// processing it.
pub struct StreamingResponse {
    status: StatusCode,
    headers: HeaderMap,
    url: Url,
    /// `None` once the body has been read to its end.
    body: Option<BodyReader<Conn>>,
    /// Set when the connection may be pooled after the body ends.
    reuse: Option<Reuse>,
}

impl StreamingResponse {
    pub(crate) fn new(
        status: StatusCode,
        headers: HeaderMap,
        url: Url,
        body: BodyReader<Conn>,
        reuse: Option<Reuse>,
    ) -> Self {
        StreamingResponse {
            status,
            headers,
            url,
            body: Some(body),
            reuse,
        }
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The next chunk of response body data, or `None` once the body is
    /// fully consumed. Chunk boundaries are an implementation detail --
    /// not aligned to `Transfer-Encoding: chunked` framing on the wire
    /// even when the response used it -- so don't rely on chunk size or
    /// count, only on the concatenation of every chunk returned before
    /// `None`.
    ///
    /// Once this returns `None`, a connection the server left open goes
    /// back to the client's pool. Dropping a `StreamingResponse` before
    /// then simply closes the connection, since its framing is mid-body,
    /// rather than leaving anything to clean up.
    pub async fn chunk(&mut self) -> Result<Option<Vec<u8>>> {
        let Some(body) = self.body.as_mut() else {
            return Ok(None);
        };
        let chunk = body.next_chunk().await?;
        if chunk.is_none() {
            self.finish();
        }
        Ok(chunk)
    }

    /// Pools the connection whose body just ended, unless the server
    /// asked to close it or sent bytes past the body.
    fn finish(&mut self) {
        let (Some(body), Some(reuse)) = (self.body.take(), self.reuse.take()) else {
            return;
        };
        let (stream, leftover) = body.into_inner().into_parts();
        if leftover.is_empty() {
            reuse.pool.put(reuse.key, stream);
        }
    }
}
