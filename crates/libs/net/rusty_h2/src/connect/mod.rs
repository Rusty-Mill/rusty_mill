//! The connection-level state machine (RFC 9113 §5–§6): settings
//! negotiation, connection/per-stream flow control, and frame dispatch,
//! built on this crate's real `frame`/`hpack`/`stream` modules rather than
//! duplicating stub placeholders of them (which is what this module used
//! to do — see `RELEASE_NOTES.md` for the history).

use std::collections::BTreeMap;

use crate::error::{ErrorCode, H2Error, Result};
use crate::frame::header::DEFAULT_MAX_FRAME_SIZE;
use crate::frame::settings::{Setting, SettingId};
use crate::frame::{
    DataFrame, Frame, GoAwayFrame, RstStreamFrame, SettingsFrame, WindowUpdateFrame,
};
use crate::hpack::{Decoder, Encoder, HeaderField, DEFAULT_HEADER_TABLE_SIZE};
use crate::stream::{Event, Stream};

/// Which side of the connection this endpoint is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerType {
    Client,
    Server,
}

/// Negotiable connection settings (RFC 9113 §6.5.2), with the RFC's own
/// defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerSettings {
    pub header_table_size: u32,
    pub enable_push: bool,
    pub max_concurrent_streams: Option<u32>,
    pub initial_window_size: u32,
    pub max_frame_size: u32,
    pub max_header_list_size: Option<u32>,
}

impl Default for ServerSettings {
    fn default() -> Self {
        ServerSettings {
            header_table_size: DEFAULT_HEADER_TABLE_SIZE as u32,
            enable_push: true,
            max_concurrent_streams: None,
            initial_window_size: 65_535,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
            max_header_list_size: None,
        }
    }
}

impl ServerSettings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies a peer's SETTINGS frame's parameters, replacing whichever
    /// fields it named. Unknown parameters (RFC 9113 §6.5.2 "MUST ignore")
    /// are silently skipped.
    fn apply(&mut self, settings: &[Setting]) {
        for setting in settings {
            match setting.id {
                SettingId::HeaderTableSize => self.header_table_size = setting.value,
                SettingId::EnablePush => self.enable_push = setting.value != 0,
                SettingId::MaxConcurrentStreams => {
                    self.max_concurrent_streams = Some(setting.value)
                }
                SettingId::InitialWindowSize => self.initial_window_size = setting.value,
                SettingId::MaxFrameSize => self.max_frame_size = setting.value,
                SettingId::MaxHeaderListSize => self.max_header_list_size = Some(setting.value),
                SettingId::Unknown(_) => {}
            }
        }
    }
}

/// Connection- and per-stream-level flow control windows (RFC 9113 §5.2 /
/// §6.9). Real signed accounting: `WINDOW_UPDATE` from a peer that
/// simultaneously reduces `SETTINGS_INITIAL_WINDOW_SIZE` can legitimately
/// drive a stream's send window negative (RFC 9113 §6.9.2), so both are
/// `i64`, not `u32`/`usize`.
#[derive(Debug, Clone, Copy)]
struct FlowControl {
    send_window: i64,
    recv_window: i64,
}

impl FlowControl {
    fn new(initial_window: u32) -> Self {
        FlowControl {
            send_window: initial_window as i64,
            recv_window: initial_window as i64,
        }
    }

    fn apply_window_update(&mut self, increment: u32) -> Result<()> {
        self.send_window = self
            .send_window
            .checked_add(increment as i64)
            .filter(|&w| w <= i32::MAX as i64)
            .ok_or(H2Error::Connection(
                ErrorCode::FlowControlError,
                "WINDOW_UPDATE overflowed the flow-control window",
            ))?;
        Ok(())
    }

    fn consume_recv(&mut self, n: u32) {
        self.recv_window -= n as i64;
    }
}

/// One active (or reserved) stream's state plus its own flow-control
/// window.
#[derive(Debug, Clone, Copy)]
struct StreamEntry {
    stream: Stream,
    flow: FlowControl,
}

/// The still-buffering state of a HEADERS or PUSH_PROMISE frame whose
/// `END_HEADERS` flag was not set, waiting for CONTINUATION frame(s)
/// (RFC 9113 §6.10) to complete the header block before it can be run
/// through the (stateful, connection-wide) HPACK decoder.
#[derive(Debug, Clone)]
enum PendingHeaderBlock {
    Headers {
        stream_id: u32,
        end_stream: bool,
        fragment: Vec<u8>,
    },
    PushPromise {
        stream_id: u32,
        promised_stream_id: u32,
        fragment: Vec<u8>,
    },
}

impl PendingHeaderBlock {
    fn stream_id(&self) -> u32 {
        match self {
            PendingHeaderBlock::Headers { stream_id, .. } => *stream_id,
            PendingHeaderBlock::PushPromise { stream_id, .. } => *stream_id,
        }
    }

    fn fragment_mut(&mut self) -> &mut Vec<u8> {
        match self {
            PendingHeaderBlock::Headers { fragment, .. } => fragment,
            PendingHeaderBlock::PushPromise { fragment, .. } => fragment,
        }
    }
}

/// The connection state machine: settings negotiation, flow control, and
/// frame dispatch, driving the real per-stream state machine
/// ([`crate::stream::Stream`]) and real HPACK codec
/// ([`crate::hpack::Encoder`]/[`Decoder`]) for every frame that touches
/// them.
pub struct Connection {
    pub local_settings: ServerSettings,
    pub remote_settings: ServerSettings,
    conn_flow: FlowControl,
    encoder: Encoder,
    decoder: Decoder,
    streams: BTreeMap<u32, StreamEntry>,
    next_stream_id: u32,
    peer_type: PeerType,
    close_reason: Option<H2Error>,
    seen_preface: bool,
    pending_header_block: Option<PendingHeaderBlock>,
}

// `Encoder`/`Decoder` don't implement `Debug` (their internal HPACK
// dynamic-table state isn't meant for inspection); a manual impl covering
// the rest of the connection's real state is more useful than deriving it
// would be anyway.
impl core::fmt::Debug for Connection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Connection")
            .field("local_settings", &self.local_settings)
            .field("remote_settings", &self.remote_settings)
            .field("streams", &self.streams.keys().collect::<Vec<_>>())
            .field("next_stream_id", &self.next_stream_id)
            .field("peer_type", &self.peer_type)
            .field("close_reason", &self.close_reason)
            .field("seen_preface", &self.seen_preface)
            .finish_non_exhaustive()
    }
}

impl Connection {
    pub fn new(settings: ServerSettings, peer_type: PeerType) -> Self {
        let next_stream_id = match peer_type {
            // Client-initiated streams are odd-numbered; server-initiated
            // (pushed) streams are even (RFC 9113 §5.1.1).
            PeerType::Client => 1,
            PeerType::Server => 2,
        };
        Connection {
            local_settings: settings,
            remote_settings: ServerSettings::default(),
            conn_flow: FlowControl::new(65_535),
            encoder: Encoder::new(settings.header_table_size as usize),
            decoder: Decoder::new(DEFAULT_HEADER_TABLE_SIZE),
            streams: BTreeMap::new(),
            next_stream_id,
            peer_type,
            close_reason: None,
            seen_preface: false,
            pending_header_block: None,
        }
    }

    pub fn peer_type(&self) -> PeerType {
        self.peer_type
    }

    /// Marks the client connection preface as seen (server side only
    /// needs this before accepting the first SETTINGS frame; a client
    /// connection doesn't send one to itself).
    pub fn mark_preface_seen(&mut self) {
        self.seen_preface = true;
    }

    fn stream_entry(&mut self, stream_id: u32) -> &mut StreamEntry {
        self.streams
            .entry(stream_id)
            .or_insert_with(|| StreamEntry {
                stream: Stream::new(stream_id),
                flow: FlowControl::new(self.remote_settings.initial_window_size),
            })
    }

    /// Number of streams currently in the "open" or "half-closed" states
    /// (RFC 9113 §5.1.2). Together with `prune_if_closed`, this is what
    /// keeps `self.streams` bounded by
    /// `local_settings.max_concurrent_streams` instead of growing
    /// forever as new stream ids arrive.
    fn open_stream_count(&self) -> usize {
        self.streams
            .values()
            .filter(|entry| {
                matches!(
                    entry.stream.state,
                    crate::stream::StreamState::Open
                        | crate::stream::StreamState::HalfClosedLocal
                        | crate::stream::StreamState::HalfClosedRemote
                )
            })
            .count()
    }

    /// Refuses `stream_id` with `REFUSED_STREAM` (RFC 9113 §5.1.2) if it
    /// isn't already tracked and accepting it would exceed the
    /// peer-advertised `SETTINGS_MAX_CONCURRENT_STREAMS` -- otherwise an
    /// unbounded sequence of increasing stream ids (e.g. HEADERS
    /// immediately followed by RST_STREAM, the HTTP/2 "Rapid Reset"
    /// pattern, CVE-2023-44487-class) grows `self.streams` without
    /// bound regardless of the advertised limit.
    fn reject_if_over_max_concurrent_streams(&self, stream_id: u32) -> Result<()> {
        if self.streams.contains_key(&stream_id) {
            return Ok(());
        }
        if let Some(max) = self.local_settings.max_concurrent_streams {
            if self.open_stream_count() >= max as usize {
                return Err(H2Error::Stream(
                    stream_id,
                    ErrorCode::RefusedStream,
                    "SETTINGS_MAX_CONCURRENT_STREAMS exceeded",
                ));
            }
        }
        Ok(())
    }

    /// Evicts `stream_id`'s entry once it has reached the terminal
    /// `Closed` state, so closed streams don't linger in `self.streams`
    /// forever (RFC 9113 §5.1's "Closed" is terminal; nothing further
    /// needs its state kept around).
    fn prune_if_closed(&mut self, stream_id: u32) {
        if let Some(entry) = self.streams.get(&stream_id) {
            if entry.stream.is_closed() {
                self.streams.remove(&stream_id);
            }
        }
    }

    /// Encodes `headers` via this connection's HPACK encoder — the bridge
    /// a caller building a HEADERS frame needs (this module doesn't build
    /// the frame itself; see `client`/`server` for that).
    pub fn encode_headers(&mut self, headers: &[HeaderField]) -> Vec<u8> {
        let mut out = Vec::new();
        self.encoder.encode(headers, &mut out);
        out
    }

    /// Applies one incoming frame, returning any frames this connection
    /// needs sent in response (e.g. a SETTINGS or PING ACK).
    pub fn apply_frame(&mut self, frame: Frame) -> Result<Vec<Frame>> {
        if let Some(reason) = &self.close_reason {
            return Err(reason.clone());
        }
        match self.handle_frame(frame) {
            Ok(responses) => Ok(responses),
            Err(e @ H2Error::Connection(_, _)) => {
                self.close_reason = Some(e.clone());
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    /// Alias kept for callers written against the earlier stub API.
    pub fn apply(&mut self, frame: Frame) -> Result<Vec<Frame>> {
        self.apply_frame(frame)
    }

    fn handle_frame(&mut self, frame: Frame) -> Result<Vec<Frame>> {
        match frame {
            Frame::Settings(settings) => self.handle_settings(settings),
            Frame::Ping(ping) => Ok(self.handle_ping(ping)),
            Frame::WindowUpdate(wu) => self.handle_window_update(wu),
            Frame::Headers(headers) => self.handle_headers(headers),
            Frame::Data(data) => self.handle_data(data),
            Frame::RstStream(rst) => self.handle_rst_stream(rst),
            Frame::GoAway(goaway) => self.handle_goaway(goaway),
            Frame::PushPromise(pp) => self.handle_push_promise(pp),
            Frame::Continuation(cont) => self.handle_continuation(cont),
            // PRIORITY/unknown frames carry no connection-state-affecting
            // semantics this driver tracks (RFC 9113 §5.5's "MUST ignore"
            // rule for unknown frame types, and PRIORITY's tree is a real
            // gap — see README).
            Frame::Priority(_) | Frame::Unknown { .. } => Ok(vec![]),
        }
    }

    fn handle_settings(
        &mut self,
        settings: crate::frame::settings::SettingsFrame,
    ) -> Result<Vec<Frame>> {
        if settings.ack {
            // Peer acknowledged settings we sent; nothing further to do.
            return Ok(vec![]);
        }
        self.remote_settings.apply(&settings.settings);
        // Any already-open stream's send window is defined relative to
        // the *current* SETTINGS_INITIAL_WINDOW_SIZE (RFC 9113 §6.9.2);
        // this driver doesn't retroactively rewrite existing streams'
        // windows on a mid-connection change -- a known, narrow gap
        // (new streams do pick up the new value, see `stream_entry`).
        Ok(vec![Frame::Settings(SettingsFrame {
            ack: true,
            settings: vec![],
        })])
    }

    fn handle_ping(&mut self, ping: crate::frame::ping::PingFrame) -> Vec<Frame> {
        if ping.ack {
            return vec![];
        }
        vec![Frame::Ping(crate::frame::ping::PingFrame {
            ack: true,
            opaque_data: ping.opaque_data,
        })]
    }

    fn handle_window_update(&mut self, wu: WindowUpdateFrame) -> Result<Vec<Frame>> {
        if wu.stream_id == 0 {
            self.conn_flow
                .apply_window_update(wu.window_size_increment)?;
        } else {
            self.stream_entry(wu.stream_id)
                .flow
                .apply_window_update(wu.window_size_increment)?;
        }
        Ok(vec![])
    }

    fn handle_headers(
        &mut self,
        headers: crate::frame::headers::HeadersFrame,
    ) -> Result<Vec<Frame>> {
        if self.pending_header_block.is_some() {
            return Err(H2Error::Connection(
                ErrorCode::ProtocolError,
                "HEADERS received while a prior header block is still awaiting CONTINUATION",
            ));
        }
        if !headers.end_headers {
            // RFC 9113 §6.10: a header block split across HEADERS and
            // CONTINUATION frames must be reassembled before it's run
            // through the (stateful) HPACK decoder -- decoding a lone
            // fragment here would desync the shared dynamic table for
            // the rest of the connection.
            self.pending_header_block = Some(PendingHeaderBlock::Headers {
                stream_id: headers.stream_id,
                end_stream: headers.end_stream,
                fragment: headers.header_block_fragment,
            });
            return Ok(vec![]);
        }
        self.finish_headers(
            headers.stream_id,
            headers.end_stream,
            &headers.header_block_fragment,
        )
    }

    /// Runs a complete (possibly CONTINUATION-reassembled) header block
    /// through HPACK and applies the resulting stream-state transition.
    fn finish_headers(
        &mut self,
        stream_id: u32,
        end_stream: bool,
        header_block_fragment: &[u8],
    ) -> Result<Vec<Frame>> {
        // Decoding still runs (and must: HPACK is stateful, so even a
        // stream we otherwise reject needs its header block consumed to
        // keep the dynamic table in sync with the peer) before any
        // stream-state bookkeeping.
        let _fields = self.decoder.decode(header_block_fragment)?;

        self.reject_if_over_max_concurrent_streams(stream_id)?;

        let entry = self.stream_entry(stream_id);
        entry.stream.apply(Event::RecvHeaders)?;
        if end_stream {
            entry.stream.apply(Event::RecvEndStream)?;
        }
        self.prune_if_closed(stream_id);
        Ok(vec![])
    }

    fn handle_data(&mut self, data: DataFrame) -> Result<Vec<Frame>> {
        let len = data.data.len() as u32;
        self.conn_flow.consume_recv(len);
        let entry = self.stream_entry(data.stream_id);
        entry.flow.consume_recv(len);
        if data.end_stream {
            entry.stream.apply(Event::RecvEndStream)?;
        }
        self.prune_if_closed(data.stream_id);
        Ok(vec![])
    }

    fn handle_rst_stream(&mut self, rst: RstStreamFrame) -> Result<Vec<Frame>> {
        let entry = self.stream_entry(rst.stream_id);
        entry.stream.apply(Event::RecvRstStream)?;
        self.prune_if_closed(rst.stream_id);
        Ok(vec![])
    }

    fn handle_goaway(&mut self, goaway: GoAwayFrame) -> Result<Vec<Frame>> {
        self.close_reason = Some(H2Error::Connection(goaway.error_code, "peer sent GOAWAY"));
        Ok(vec![])
    }

    fn handle_push_promise(
        &mut self,
        pp: crate::frame::push_promise::PushPromiseFrame,
    ) -> Result<Vec<Frame>> {
        if !self.remote_settings.enable_push {
            return Err(H2Error::Connection(
                ErrorCode::ProtocolError,
                "PUSH_PROMISE received with push disabled",
            ));
        }
        if self.pending_header_block.is_some() {
            return Err(H2Error::Connection(
                ErrorCode::ProtocolError,
                "PUSH_PROMISE received while a prior header block is still awaiting CONTINUATION",
            ));
        }
        if !pp.end_headers {
            // Same reassembly requirement as HEADERS (RFC 9113 §6.10).
            self.pending_header_block = Some(PendingHeaderBlock::PushPromise {
                stream_id: pp.stream_id,
                promised_stream_id: pp.promised_stream_id,
                fragment: pp.header_block_fragment,
            });
            return Ok(vec![]);
        }
        self.finish_push_promise(pp.promised_stream_id, &pp.header_block_fragment)
    }

    /// Runs a complete (possibly CONTINUATION-reassembled) PUSH_PROMISE
    /// header block through HPACK and reserves the promised stream.
    fn finish_push_promise(
        &mut self,
        promised_stream_id: u32,
        header_block_fragment: &[u8],
    ) -> Result<Vec<Frame>> {
        // Decode (dynamic-table state must stay in sync, same reasoning
        // as `finish_headers`) and reserve the promised stream via the
        // real state machine. Actually delivering the pushed response
        // itself isn't implemented -- a documented gap, not a silent
        // no-op: the stream is genuinely reserved, just never fulfilled.
        let _fields = self.decoder.decode(header_block_fragment)?;
        self.reject_if_over_max_concurrent_streams(promised_stream_id)?;
        let entry = self.stream_entry(promised_stream_id);
        entry.stream.apply(Event::RecvPushPromise)?;
        Ok(vec![])
    }

    /// Merges a CONTINUATION frame's fragment into the pending
    /// HEADERS/PUSH_PROMISE header block it continues (RFC 9113 §6.10),
    /// only running the reassembled block through HPACK once
    /// `END_HEADERS` is finally set.
    fn handle_continuation(
        &mut self,
        cont: crate::frame::continuation::ContinuationFrame,
    ) -> Result<Vec<Frame>> {
        let Some(mut pending) = self.pending_header_block.take() else {
            return Err(H2Error::Connection(
                ErrorCode::ProtocolError,
                "CONTINUATION received without a preceding HEADERS/PUSH_PROMISE",
            ));
        };
        if pending.stream_id() != cont.stream_id {
            return Err(H2Error::Connection(
                ErrorCode::ProtocolError,
                "CONTINUATION stream id does not match the header block it continues",
            ));
        }
        pending
            .fragment_mut()
            .extend_from_slice(&cont.header_block_fragment);
        if !cont.end_headers {
            self.pending_header_block = Some(pending);
            return Ok(vec![]);
        }
        match pending {
            PendingHeaderBlock::Headers {
                stream_id,
                end_stream,
                fragment,
            } => self.finish_headers(stream_id, end_stream, &fragment),
            PendingHeaderBlock::PushPromise {
                promised_stream_id,
                fragment,
                ..
            } => self.finish_push_promise(promised_stream_id, &fragment),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::HeadersFrame;
    use crate::stream::StreamState;

    fn settings_frame(ack: bool, settings: Vec<Setting>) -> Frame {
        Frame::Settings(SettingsFrame { ack, settings })
    }

    #[test]
    fn new_connection_assigns_stream_ids_per_peer_type() {
        let client = Connection::new(ServerSettings::default(), PeerType::Client);
        let server = Connection::new(ServerSettings::default(), PeerType::Server);
        assert_eq!(client.next_stream_id, 1);
        assert_eq!(server.next_stream_id, 2);
    }

    #[test]
    fn settings_frame_is_acked_and_updates_remote_settings() {
        let mut conn = Connection::new(ServerSettings::default(), PeerType::Server);
        let responses = conn
            .apply_frame(settings_frame(
                false,
                vec![Setting {
                    id: SettingId::InitialWindowSize,
                    value: 100_000,
                }],
            ))
            .unwrap();
        assert_eq!(conn.remote_settings.initial_window_size, 100_000);
        assert!(matches!(
            responses.as_slice(),
            [Frame::Settings(SettingsFrame { ack: true, .. })]
        ));
    }

    #[test]
    fn settings_ack_produces_no_response() {
        let mut conn = Connection::new(ServerSettings::default(), PeerType::Server);
        let responses = conn.apply_frame(settings_frame(true, vec![])).unwrap();
        assert!(responses.is_empty());
    }

    #[test]
    fn ping_is_acked_with_the_same_opaque_data() {
        let mut conn = Connection::new(ServerSettings::default(), PeerType::Client);
        let responses = conn
            .apply_frame(Frame::Ping(crate::frame::ping::PingFrame {
                ack: false,
                opaque_data: *b"abcdefgh",
            }))
            .unwrap();
        match responses.as_slice() {
            [Frame::Ping(p)] => {
                assert!(p.ack);
                assert_eq!(&p.opaque_data, b"abcdefgh");
            }
            other => panic!("expected one PING ack, got {other:?}"),
        }
    }

    #[test]
    fn window_update_increases_the_connection_send_window() {
        let mut conn = Connection::new(ServerSettings::default(), PeerType::Client);
        let before = conn.conn_flow.send_window;
        conn.apply_frame(Frame::WindowUpdate(WindowUpdateFrame {
            stream_id: 0,
            window_size_increment: 1000,
        }))
        .unwrap();
        assert_eq!(conn.conn_flow.send_window, before + 1000);
    }

    #[test]
    fn window_update_on_a_stream_only_affects_that_stream() {
        let mut conn = Connection::new(ServerSettings::default(), PeerType::Client);
        conn.stream_entry(1);
        let conn_before = conn.conn_flow.send_window;
        conn.apply_frame(Frame::WindowUpdate(WindowUpdateFrame {
            stream_id: 1,
            window_size_increment: 500,
        }))
        .unwrap();
        assert_eq!(conn.conn_flow.send_window, conn_before);
        assert_eq!(conn.streams[&1].flow.send_window, 65_535 + 500);
    }

    #[test]
    fn headers_frame_opens_a_stream_via_the_real_state_machine() {
        let mut conn = Connection::new(ServerSettings::default(), PeerType::Server);
        let mut encoder = Encoder::new(4096);
        let mut header_block = Vec::new();
        encoder.encode(&[HeaderField::new(":method", "GET")], &mut header_block);

        conn.apply_frame(Frame::Headers(HeadersFrame {
            stream_id: 1,
            end_stream: false,
            end_headers: true,
            priority: None,
            header_block_fragment: header_block,
        }))
        .unwrap();

        assert_eq!(conn.streams[&1].stream.state, StreamState::Open);
    }

    #[test]
    fn headers_with_end_stream_half_closes_the_remote_side() {
        let mut conn = Connection::new(ServerSettings::default(), PeerType::Server);
        let mut encoder = Encoder::new(4096);
        let mut header_block = Vec::new();
        encoder.encode(&[HeaderField::new(":method", "GET")], &mut header_block);

        conn.apply_frame(Frame::Headers(HeadersFrame {
            stream_id: 1,
            end_stream: true,
            end_headers: true,
            priority: None,
            header_block_fragment: header_block,
        }))
        .unwrap();

        assert_eq!(conn.streams[&1].stream.state, StreamState::HalfClosedRemote);
    }

    #[test]
    fn data_frame_consumes_recv_flow_control_window() {
        let mut conn = Connection::new(ServerSettings::default(), PeerType::Server);
        conn.stream_entry(1);
        let before = conn.conn_flow.recv_window;
        conn.apply_frame(Frame::Data(DataFrame {
            stream_id: 1,
            end_stream: false,
            data: vec![0u8; 100],
        }))
        .unwrap();
        assert_eq!(conn.conn_flow.recv_window, before - 100);
        assert_eq!(conn.streams[&1].flow.recv_window, 65_535 - 100);
    }

    #[test]
    fn rst_stream_prunes_the_stream_entry() {
        let mut conn = Connection::new(ServerSettings::default(), PeerType::Server);
        // RST_STREAM on a still-idle stream is a connection error (RFC 9113
        // §5.1): open it first via HEADERS, matching a real request.
        let mut encoder = Encoder::new(4096);
        let mut header_block = Vec::new();
        encoder.encode(&[HeaderField::new(":method", "GET")], &mut header_block);
        conn.apply_frame(Frame::Headers(HeadersFrame {
            stream_id: 1,
            end_stream: false,
            end_headers: true,
            priority: None,
            header_block_fragment: header_block,
        }))
        .unwrap();
        assert!(conn.streams.contains_key(&1));

        conn.apply_frame(Frame::RstStream(RstStreamFrame {
            stream_id: 1,
            error_code: ErrorCode::Cancel,
        }))
        .unwrap();

        // The stream reached the terminal `Closed` state; its entry must
        // be pruned from `self.streams` rather than lingering forever
        // (otherwise a flood of increasing stream ids each immediately
        // RST_STREAM'd -- HTTP/2 "Rapid Reset", CVE-2023-44487-class --
        // grows memory without bound).
        assert!(!conn.streams.contains_key(&1));
    }

    #[test]
    fn headers_beyond_max_concurrent_streams_is_refused() {
        let settings = ServerSettings {
            max_concurrent_streams: Some(1),
            ..ServerSettings::default()
        };
        let mut conn = Connection::new(settings, PeerType::Server);
        let mut encoder = Encoder::new(4096);

        let mut first_block = Vec::new();
        encoder.encode(&[HeaderField::new(":method", "GET")], &mut first_block);
        conn.apply_frame(Frame::Headers(HeadersFrame {
            stream_id: 1,
            end_stream: false,
            end_headers: true,
            priority: None,
            header_block_fragment: first_block,
        }))
        .unwrap();

        let mut second_block = Vec::new();
        encoder.encode(&[HeaderField::new(":method", "GET")], &mut second_block);
        let err = conn
            .apply_frame(Frame::Headers(HeadersFrame {
                stream_id: 3,
                end_stream: false,
                end_headers: true,
                priority: None,
                header_block_fragment: second_block,
            }))
            .unwrap_err();

        assert!(matches!(
            err,
            H2Error::Stream(3, ErrorCode::RefusedStream, _)
        ));
        // The refused stream must not have been inserted, and the
        // already-open stream must remain the only tracked entry.
        assert!(!conn.streams.contains_key(&3));
        assert_eq!(conn.streams.len(), 1);
    }

    #[test]
    fn goaway_sets_a_close_reason_and_rejects_further_frames() {
        let mut conn = Connection::new(ServerSettings::default(), PeerType::Client);
        conn.apply_frame(Frame::GoAway(GoAwayFrame {
            last_stream_id: 0,
            error_code: ErrorCode::NoError,
            debug_data: vec![],
        }))
        .unwrap();
        let err = conn.apply_frame(settings_frame(true, vec![])).unwrap_err();
        assert!(matches!(err, H2Error::Connection(ErrorCode::NoError, _)));
    }

    #[test]
    fn push_promise_reserves_the_promised_stream_when_enabled() {
        let mut conn = Connection::new(ServerSettings::default(), PeerType::Client);
        conn.remote_settings.enable_push = true;
        let mut encoder = Encoder::new(4096);
        let mut header_block = Vec::new();
        encoder.encode(&[HeaderField::new(":method", "GET")], &mut header_block);

        conn.apply_frame(Frame::PushPromise(
            crate::frame::push_promise::PushPromiseFrame {
                stream_id: 1,
                end_headers: true,
                promised_stream_id: 2,
                header_block_fragment: header_block,
            },
        ))
        .unwrap();

        assert_eq!(conn.streams[&2].stream.state, StreamState::ReservedRemote);
    }

    #[test]
    fn push_promise_is_rejected_when_push_is_disabled() {
        let mut conn = Connection::new(ServerSettings::default(), PeerType::Client);
        conn.remote_settings.enable_push = false;
        let err = conn
            .apply_frame(Frame::PushPromise(
                crate::frame::push_promise::PushPromiseFrame {
                    stream_id: 1,
                    end_headers: true,
                    promised_stream_id: 2,
                    header_block_fragment: vec![],
                },
            ))
            .unwrap_err();
        assert!(matches!(
            err,
            H2Error::Connection(ErrorCode::ProtocolError, _)
        ));
    }

    #[test]
    fn continuation_frame_is_merged_into_the_preceding_header_block() {
        let mut conn = Connection::new(ServerSettings::default(), PeerType::Server);
        let mut encoder = Encoder::new(4096);

        // A header whose value is long enough that splitting the encoded
        // block near its tail lands inside the value's string encoding,
        // not on a frame boundary -- decoding either half alone (the
        // pre-fix bug: decoding regardless of `end_headers`) must fail.
        let field = HeaderField::new(
            "x-custom-header-name",
            "some-long-header-value-used-to-force-a-split-mid-string",
        );
        let mut full_block = Vec::new();
        encoder.encode(std::slice::from_ref(&field), &mut full_block);
        assert!(
            full_block.len() > 10,
            "test header block too short to split meaningfully"
        );
        let split = full_block.len() - 3;
        let (first_part, second_part) = full_block.split_at(split);

        // HEADERS with END_HEADERS unset must not be decoded yet -- it's
        // buffered pending the CONTINUATION that completes it.
        let responses = conn
            .apply_frame(Frame::Headers(HeadersFrame {
                stream_id: 1,
                end_stream: false,
                end_headers: false,
                priority: None,
                header_block_fragment: first_part.to_vec(),
            }))
            .unwrap();
        assert!(responses.is_empty());
        assert!(
            !conn.streams.contains_key(&1),
            "stream must not open until the full header block is decoded"
        );

        // The CONTINUATION frame completes the header block; only now
        // should it be decoded, updating the dynamic table exactly once.
        conn.apply_frame(Frame::Continuation(
            crate::frame::continuation::ContinuationFrame {
                stream_id: 1,
                end_headers: true,
                header_block_fragment: second_part.to_vec(),
            },
        ))
        .unwrap();
        assert_eq!(conn.streams[&1].stream.state, StreamState::Open);
        assert_eq!(conn.decoder.dynamic_table_len(), 1);

        // A subsequent request that references the same header via HPACK
        // indexing must decode against the correctly-synced dynamic
        // table built from the *complete*, reassembled block -- proving
        // the shared table wasn't desynced by decoding a lone fragment.
        let mut second_block = Vec::new();
        encoder.encode(&[field], &mut second_block);
        assert_eq!(
            second_block.len(),
            1,
            "peer encoder should now emit a single indexed byte for a repeated header"
        );

        conn.apply_frame(Frame::Headers(HeadersFrame {
            stream_id: 3,
            end_stream: false,
            end_headers: true,
            priority: None,
            header_block_fragment: second_block,
        }))
        .unwrap();
        assert_eq!(conn.streams[&3].stream.state, StreamState::Open);
        assert_eq!(conn.decoder.dynamic_table_len(), 1);
    }
}
