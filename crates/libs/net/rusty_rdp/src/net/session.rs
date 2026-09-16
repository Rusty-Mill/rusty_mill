//! Post-handshake active-session I/O: [`RdpTransport::recv_event`] (slow-path
//! and fast-path server updates, reassembled virtual-channel data) and
//! [`RdpTransport::send_input`] (fast-path client input).

use super::*;

/// A server-to-client event read after the session is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RdpEvent {
    /// A bitmap update: one or more pixel rectangles.
    Bitmap(Vec<BitmapData>),
    /// A palette update (8bpp color table).
    Palette(PaletteUpdate),
    /// A pointer/cursor update (shape, position, or system cursor).
    Pointer(PointerUpdate),
    /// An update-synchronize marker.
    UpdateSynchronize,
    /// Raw drawing orders (not decoded here).
    Orders(Vec<u8>),
    /// A server connection-finalization PDU (synchronize / control / font).
    Finalization(FinalizationPdu),
    /// The server asked to deactivate the share (a reactivation may follow).
    DeactivateAll,
    /// A reassembled message on a static virtual channel other than the I/O
    /// channel (MS-RDPBCGR 2.2.6.1) — e.g. dynamic-channel traffic on
    /// [`crate::dvc::DRDYNVC_CHANNEL_NAME`], decodable with [`crate::dvc`].
    ChannelData {
        /// The MCS channel id the data arrived on (see
        /// [`RdpSession::channel_id`] to map this back to a channel name).
        channel_id: u16,
        /// The reassembled message.
        data: Vec<u8>,
    },
    /// A share PDU this driver does not model.
    Other {
        /// The Share Control `pduType`.
        pdu_type: u16,
        /// The Share Data `pduType2`, when the PDU is a Data PDU.
        pdu_type2: Option<u8>,
    },
}

impl<S: Read + Write> RdpTransport<S> {
    /// Send `data` on virtual channel `channel_id`, chunking it per
    /// MS-RDPBCGR 2.2.6.1 (`crate::vchan::chunk`) and encrypting each chunk
    /// with the stored session when active. Use the id from
    /// [`RdpSession::channel_id`] for a channel requested via
    /// [`EstablishConfig::extra_channels`].
    pub fn send_channel_data(
        &mut self,
        user_id: u16,
        channel_id: u16,
        data: &[u8],
    ) -> io::Result<()> {
        for chunk in crate::vchan::chunk(data, crate::vchan::DEFAULT_CHUNK_SIZE) {
            self.send_share(user_id, channel_id, &chunk)?;
        }
        Ok(())
    }

    /// Receive and classify one server-to-client event once the session is
    /// active (after [`establish`](Self::establish)).
    ///
    /// Reads the next frame — slow-path (TPKT / X.224 / MCS / Share) or
    /// fast-path — decrypts it with the stored session, and returns a typed
    /// [`RdpEvent`]. A fast-path PDU may bundle several updates; the extras are
    /// buffered and returned by later calls. Anything not modelled comes back
    /// as [`RdpEvent::Other`] rather than an error. Slow-path traffic on a
    /// virtual channel other than the I/O channel is reassembled
    /// (MS-RDPBCGR 2.2.6.1) and, once complete, returned as
    /// [`RdpEvent::ChannelData`].
    pub fn recv_event(&mut self) -> io::Result<RdpEvent> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Ok(event);
            }
            let mut first = [0u8; 1];
            self.stream.read_exact(&mut first)?;
            if crate::fastpath::is_fastpath(first[0]) {
                let events = self.read_fastpath_output(first[0])?;
                self.pending.extend(events);
            } else if let Some((channel_id, body)) = self.read_slowpath_share(first[0])? {
                // Route by channel once join_all_channels has told us which one
                // is the I/O channel; if that hasn't happened (e.g. a caller
                // driving recv_event without the full establish() sequence),
                // there is nothing to route by, so treat it as Share data.
                if self.io_channel.is_none() || Some(channel_id) == self.io_channel {
                    self.pending.push_back(classify_share(&body)?);
                } else if let Some(data) = self
                    .channel_reassemblers
                    .entry(channel_id)
                    .or_default()
                    .feed(&body)
                    .map_err(to_io)?
                {
                    self.pending
                        .push_back(RdpEvent::ChannelData { channel_id, data });
                }
                // Otherwise a partial chunk was buffered; keep reading.
            }
        }
    }

    /// Read the remainder of a TPKT packet whose first byte is `first`,
    /// returning the inner X.224 TPDU payload.
    fn read_tpkt_rest(&mut self, first: u8) -> io::Result<Vec<u8>> {
        let mut rest = [0u8; 3];
        self.stream.read_exact(&mut rest)?;
        let total = u16::from_be_bytes([rest[1], rest[2]]) as usize;
        if total < TPKT_HEADER_LEN {
            return Err(protocol_error("short TPKT packet"));
        }
        let mut packet = vec![0u8; total];
        packet[..TPKT_HEADER_LEN].copy_from_slice(&[first, rest[0], rest[1], rest[2]]);
        self.stream.read_exact(&mut packet[TPKT_HEADER_LEN..])?;
        let tpkt = Tpkt::decode(&packet).map_err(to_io)?;
        Ok(tpkt.payload.to_vec())
    }

    /// Read a slow-path frame and return `(channel_id, decrypted body)`, or
    /// `None` if the frame is not a Send Data Indication.
    fn read_slowpath_share(&mut self, first: u8) -> io::Result<Option<(u16, Vec<u8>)>> {
        let tpdu = self.read_tpkt_rest(first)?;
        let inner = match X224::decode(&tpdu).map_err(to_io)? {
            X224::Data(payload) => payload.to_vec(),
            _ => return Ok(None),
        };
        let (channel_id, user_data) = match DomainPdu::decode(&inner).map_err(to_io)? {
            DomainPdu::SendDataIndication {
                channel_id,
                user_data,
                ..
            } => (channel_id, user_data.to_vec()),
            _ => return Ok(None),
        };
        if self.enhanced {
            // Under TLS, data PDUs carry no Basic Security Header.
            return Ok(Some((channel_id, user_data)));
        }
        let mut session = self.session.take();
        let result = security::unwrap_pdu(session.as_mut(), &user_data)
            .map(|(_flags, body)| body)
            .map_err(to_io);
        self.session = session;
        Ok(Some((channel_id, result?)))
    }

    /// Read a fast-path output frame whose header byte is `header` and decode
    /// its updates into events.
    fn read_fastpath_output(&mut self, header: u8) -> io::Result<Vec<RdpEvent>> {
        let l1 = {
            let mut b = [0u8; 1];
            self.stream.read_exact(&mut b)?;
            b[0]
        };
        let (total, len_field) = if l1 & 0x80 != 0 {
            let mut b = [0u8; 1];
            self.stream.read_exact(&mut b)?;
            ((((l1 & 0x7F) as usize) << 8) | b[0] as usize, 2usize)
        } else {
            (l1 as usize, 1usize)
        };
        let header_len = 1 + len_field;
        if total < header_len {
            return Err(protocol_error("short fast-path PDU"));
        }
        let mut rest = vec![0u8; total - header_len];
        self.stream.read_exact(&mut rest)?;

        let encryption_flags = (header >> 6) & 0x03;
        let update_bytes = if encryption_flags & crate::fastpath::FASTPATH_ENCRYPTED != 0 {
            if rest.len() < 8 {
                return Err(protocol_error("fast-path PDU missing signature"));
            }
            let signature = rest[..8].to_vec();
            let ciphertext = &rest[8..];
            let mut session = self.session.take();
            let result = match session.as_mut() {
                Some(s) => s.decrypt(&signature, ciphertext).map_err(to_io),
                None => Err(protocol_error("encrypted fast-path PDU but no session")),
            };
            self.session = session;
            result?
        } else {
            rest
        };

        let updates = crate::fastpath::parse_output_updates(&update_bytes).map_err(to_io)?;
        Ok(updates.into_iter().map(fastpath_update_to_event).collect())
    }

    /// Send client input as a fast-path Input PDU, encrypting with the stored
    /// session when one is active.
    pub fn send_input(&mut self, events: &[InputEvent]) -> io::Result<()> {
        let (count, event_bytes) = crate::fastpath::encode_input_events(events);
        if count > u8::MAX as usize {
            return Err(protocol_error("too many input events for one PDU"));
        }

        // numberEvents rides in the header when it fits in 4 bits, else in a
        // separate byte prefixed to the (possibly encrypted) event data.
        let (num_field, mut plaintext) = if count <= 0x0F {
            (count as u8, Vec::new())
        } else {
            (0u8, vec![count as u8])
        };
        plaintext.extend_from_slice(&event_bytes);

        let mut session = self.session.take();
        let (enc_flags, body) = match session.as_mut() {
            Some(s) => {
                let (signature, ciphertext) = s.encrypt(&plaintext);
                let mut body = signature.to_vec();
                body.extend_from_slice(&ciphertext);
                (
                    crate::fastpath::FASTPATH_ENCRYPTED | crate::fastpath::FASTPATH_SECURE_CHECKSUM,
                    body,
                )
            }
            None => (0u8, plaintext),
        };
        self.session = session;

        let header = crate::fastpath::FASTPATH_ACTION | (num_field << 2) | (enc_flags << 6);
        // Total length includes the header byte, the length field, and the body.
        let base = 1 + body.len();
        let total = if base < 0x7F { base + 1 } else { base + 2 };

        let mut w = crate::cursor::Writer::new();
        w.write_u8(header);
        crate::fastpath::write_length(&mut w, total).map_err(to_io)?;
        w.write_bytes(&body);
        self.stream.write_all(w.as_slice())?;
        self.stream.flush()
    }
}

/// Classify a decrypted Share Control / Share Data PDU body into an event.
fn classify_share(body: &[u8]) -> io::Result<RdpEvent> {
    let (control, _payload) = ShareControlHeader::decode(body).map_err(to_io)?;
    match control.pdu_type {
        PDUTYPE_DEACTIVATEALLPDU => Ok(RdpEvent::DeactivateAll),
        crate::pdu::PDUTYPE_DATAPDU => {
            let (_source, header, _data) = ShareDataHeader::decode(body).map_err(to_io)?;
            match header.pdu_type2 {
                PDUTYPE2_UPDATE => {
                    let (_s, _sid, update) = UpdatePdu::decode(body).map_err(to_io)?;
                    Ok(match update {
                        UpdatePdu::Bitmap(rects) => RdpEvent::Bitmap(rects),
                        UpdatePdu::Palette(palette) => RdpEvent::Palette(palette),
                        UpdatePdu::Synchronize => RdpEvent::UpdateSynchronize,
                        UpdatePdu::Orders(data) => RdpEvent::Orders(data),
                    })
                }
                PDUTYPE2_POINTER => {
                    let (_s, _sid, pointer) = PointerUpdate::decode(body).map_err(to_io)?;
                    Ok(RdpEvent::Pointer(pointer))
                }
                PDUTYPE2_SYNCHRONIZE | PDUTYPE2_CONTROL | PDUTYPE2_FONTMAP => {
                    let (_s, _sid, fin) = FinalizationPdu::decode(body).map_err(to_io)?;
                    Ok(RdpEvent::Finalization(fin))
                }
                other => Ok(RdpEvent::Other {
                    pdu_type: control.pdu_type,
                    pdu_type2: Some(other),
                }),
            }
        }
        other => Ok(RdpEvent::Other {
            pdu_type: other,
            pdu_type2: None,
        }),
    }
}

/// Map a fast-path update to the shared [`RdpEvent`] type.
fn fastpath_update_to_event(update: crate::fastpath::FastPathUpdate) -> RdpEvent {
    use crate::fastpath::FastPathUpdate as F;
    use crate::pointer::{PointerUpdate, SYSPTR_DEFAULT, SYSPTR_NULL};
    match update {
        F::Bitmap(rects) => RdpEvent::Bitmap(rects),
        F::Palette(palette) => RdpEvent::Palette(palette),
        F::Synchronize => RdpEvent::UpdateSynchronize,
        F::PointerHidden => RdpEvent::Pointer(PointerUpdate::System(SYSPTR_NULL)),
        F::PointerDefault => RdpEvent::Pointer(PointerUpdate::System(SYSPTR_DEFAULT)),
        F::PointerPosition { x, y } => RdpEvent::Pointer(PointerUpdate::Position { x, y }),
        F::PointerColor(pointer) => RdpEvent::Pointer(PointerUpdate::Color(pointer)),
        F::PointerNew { xor_bpp, pointer } => {
            RdpEvent::Pointer(PointerUpdate::New { xor_bpp, pointer })
        }
        F::PointerCached(index) => RdpEvent::Pointer(PointerUpdate::Cached(index)),
        F::Raw { update_code, .. } => RdpEvent::Other {
            pdu_type: 0,
            pdu_type2: Some(update_code),
        },
    }
}
