//! `RdpTransport`'s type definition and the low-level framing primitives
//! (TPKT/X.224 read+write) shared by both the client ([`super::client`])
//! and server ([`super::server`]) connection sequences.

use super::*;

/// A TPKT-framed RDP transport over a byte stream.
///
/// After [`RdpTransport::establish`] (or a manual `security_exchange`) the
/// transport holds the RC4 session used to encrypt and decrypt I/O-channel
/// traffic.
pub struct RdpTransport<S> {
    pub(super) stream: S,
    pub(super) session: Option<Rc4Session>,
    /// Events decoded from a fast-path PDU that bundled several updates,
    /// waiting to be returned one at a time by [`RdpTransport::recv_event`].
    pub(super) pending: std::collections::VecDeque<RdpEvent>,
    /// `true` when the stream already provides encryption (TLS/CredSSP), so
    /// the RDP security layer is disabled: no Security Exchange, no RC4, and
    /// data PDUs carry no Basic Security Header (MS-RDPBCGR 5.4). Only the
    /// Client Info and licensing PDUs keep a header in this mode.
    pub(super) enhanced: bool,
    /// The MCS I/O channel id, once known (set during channel setup). Slow-path
    /// traffic on any other joined channel is virtual-channel data, not a
    /// Share Control/Data PDU.
    pub(super) io_channel: Option<u16>,
    /// Per-channel reassembly state for static virtual channel traffic
    /// (MS-RDPBCGR 2.2.6.1), keyed by MCS channel id.
    pub(super) channel_reassemblers: HashMap<u16, crate::vchan::Reassembler>,
}

impl<S: Read + Write> RdpTransport<S> {
    /// Wrap a connected stream that speaks standard RDP security (the RDP
    /// security layer encrypts PDUs itself).
    pub fn new(stream: S) -> Self {
        RdpTransport {
            stream,
            session: None,
            pending: std::collections::VecDeque::new(),
            enhanced: false,
            io_channel: None,
            channel_reassemblers: HashMap::new(),
        }
    }

    /// Wrap a stream that already provides encryption (TLS/CredSSP).
    ///
    /// Use this after the X.224 negotiation on the raw TCP connection has
    /// selected an enhanced-security protocol and the stream has been upgraded
    /// (e.g. wrapped in TLS). The RDP security layer is left off:
    /// [`RdpTransport::establish_enhanced`] skips the Security Exchange and no
    /// PDU is RC4-encrypted.
    pub fn new_enhanced(stream: S) -> Self {
        RdpTransport {
            stream,
            session: None,
            pending: std::collections::VecDeque::new(),
            enhanced: true,
            io_channel: None,
            channel_reassemblers: HashMap::new(),
        }
    }

    /// Consume the transport and return the underlying stream.
    pub fn into_inner(self) -> S {
        self.stream
    }

    /// Borrow the underlying stream.
    pub fn get_ref(&self) -> &S {
        &self.stream
    }

    // --- TPKT framing -----------------------------------------------------

    /// Write `payload` as a single TPKT packet.
    pub fn write_tpkt(&mut self, payload: &[u8]) -> io::Result<()> {
        let packet = Tpkt::new(payload).to_vec().map_err(to_io)?;
        self.stream.write_all(&packet)?;
        self.stream.flush()
    }

    /// Read one complete TPKT packet, returning its payload (the X.224 TPDU).
    pub fn read_tpkt(&mut self) -> io::Result<Vec<u8>> {
        let mut header = [0u8; TPKT_HEADER_LEN];
        self.stream.read_exact(&mut header)?;
        let total = Tpkt::peek_total_len(&header)
            .map_err(to_io)?
            .ok_or_else(|| protocol_error("short TPKT header"))?;
        let mut packet = vec![0u8; total];
        packet[..TPKT_HEADER_LEN].copy_from_slice(&header);
        self.stream.read_exact(&mut packet[TPKT_HEADER_LEN..])?;
        let tpkt = Tpkt::decode(&packet).map_err(to_io)?;
        Ok(tpkt.payload.to_vec())
    }

    /// Wrap `payload` in an X.224 Data TPDU and send it.
    pub fn write_x224_data(&mut self, payload: &[u8]) -> io::Result<()> {
        let tpdu = X224::data(payload).to_vec().map_err(to_io)?;
        self.write_tpkt(&tpdu)
    }

    /// Read a TPKT packet and return the inner X.224 Data TPDU payload.
    pub fn read_x224_data(&mut self) -> io::Result<Vec<u8>> {
        let tpdu = self.read_tpkt()?;
        match X224::decode(&tpdu).map_err(to_io)? {
            X224::Data(payload) => Ok(payload.to_vec()),
            other => Err(protocol_error(format!("expected Data TPDU, got {other:?}"))),
        }
    }
}
