//! Reading the server name out of a TLS ClientHello.
//!
//! # Why this exists rather than a certificate resolver
//!
//! `rustls` selects a certificate through `ResolvesServerCert`, and that is how
//! this would normally be done. [`rusty_tls`]'s `TlsAcceptor` holds its
//! `ServerConfig` privately and offers no way to supply one, so there is no
//! resolver to install — and reaching around the crate into `rustls` to build a
//! config by hand would give up the one thing importing it buys: that this
//! gateway is not the consumer in the ecosystem that rolls its own TLS.
//!
//! So the name is read off the wire instead. The first bytes a client sends are
//! a ClientHello carrying the SNI extension in plaintext — it has to be, since
//! the server cannot decrypt anything before it knows which certificate to
//! present. Peeking those bytes chooses the acceptor, and the acceptor
//! `rusty_tls` does expose then does the handshake on a stream nothing has
//! consumed.
//!
//! # It only ever reads
//!
//! Nothing here decides whether a handshake succeeds; `rustls` still does all
//! of that, on the same bytes, immediately afterwards. A ClientHello this
//! cannot parse — truncated, malformed, or a version whose layout changed —
//! returns `None`, the default certificate is served, and the handshake either
//! works or fails on its own merits. That is the same outcome as before SNI
//! selection existed, which is what makes a permissive parser the safe choice
//! here rather than a lax one.
//!
//! Every length in the message is attacker-controlled, so every read is bounds
//! checked against the slice rather than against the length that preceded it.

/// A cursor over the ClientHello that cannot read past its slice.
///
/// The whole parser is written against this rather than indexing directly:
/// every field in the message is a length written by whoever connected, and
/// one unchecked `&bytes[a..b]` is a panic in the accept loop.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Reader { bytes, at: 0 }
    }

    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(count)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|slice| slice[0])
    }

    fn u16(&mut self) -> Option<usize> {
        let slice = self.take(2)?;
        Some(usize::from(u16::from_be_bytes([slice[0], slice[1]])))
    }

    /// Skip a block introduced by a one-byte length.
    fn skip_u8_vec(&mut self) -> Option<()> {
        let length = usize::from(self.u8()?);
        self.take(length).map(|_| ())
    }

    /// Skip a block introduced by a two-byte length.
    fn skip_u16_vec(&mut self) -> Option<()> {
        let length = self.u16()?;
        self.take(length).map(|_| ())
    }
}

/// How many bytes are worth peeking to find the name.
///
/// A ClientHello is normally well under a kilobyte, but a client offering many
/// cipher suites, extensions or a large session ticket can be several. The cap
/// exists so a peer that never sends a complete one cannot make the accept loop
/// wait forever; past it, the default certificate is served.
pub const MAX_HELLO: usize = 8 * 1024;

/// The server name a ClientHello asks for, if it carries one.
///
/// `None` for anything this cannot read: a message still arriving, a client
/// that sent no SNI extension — which is every client addressing the gateway by
/// IP — or bytes that are not a ClientHello at all.
pub fn server_name(bytes: &[u8]) -> Option<String> {
    let mut reader = Reader::new(bytes);

    // Record layer: a handshake record (22) and its two-byte version, then the
    // length of what it carries. The record's own length is not trusted as a
    // bound; the slice is.
    if reader.u8()? != 0x16 {
        return None;
    }
    reader.take(2)?;
    reader.u16()?;

    // Handshake header: ClientHello (1) and a three-byte length.
    if reader.u8()? != 0x01 {
        return None;
    }
    reader.take(3)?;

    // ClientHello body: the legacy version and 32 bytes of random, then three
    // variable-length blocks before the extensions begin.
    reader.take(2)?;
    reader.take(32)?;
    reader.skip_u8_vec()?; // session id
    reader.skip_u16_vec()?; // cipher suites
    reader.skip_u8_vec()?; // compression methods

    let extensions_length = reader.u16()?;
    let extensions = reader.take(extensions_length)?;
    read_server_name(extensions)
}

/// Find the SNI extension among the ClientHello's extensions.
fn read_server_name(extensions: &[u8]) -> Option<String> {
    let mut reader = Reader::new(extensions);

    while let Some(kind) = reader.u16() {
        let length = reader.u16()?;
        let body = reader.take(length)?;

        // `server_name` is extension 0. Everything else is skipped by length,
        // which is why an unknown extension does not stop the search.
        if kind != 0 {
            continue;
        }

        let mut names = Reader::new(body);
        names.u16()?; // the list's own length
        while let Some(name_kind) = names.u8() {
            let name_length = names.u16()?;
            let name = names.take(name_length)?;

            // Type 0 is `host_name`, and it is the only type ever defined.
            if name_kind != 0 {
                continue;
            }
            // A name is ASCII by the spec. Rejecting anything else rather than
            // lossily converting keeps a hostname match from succeeding
            // against something nobody wrote down.
            let name = std::str::from_utf8(name).ok()?;
            return Some(name.to_ascii_lowercase());
        }
        return None;
    }

    None
}

#[cfg(test)]
mod tests;
