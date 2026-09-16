//! Dynamic virtual channel session management, std-only.
//!
//! [`crate::dvc`] is a pure PDU codec: it turns bytes into
//! `CreateRequestPdu`/`DataPdu`/etc. and back, but doesn't track which
//! channels are open or answer the server's requests. [`DvcManager`] is that
//! bookkeeping — a small, I/O-free state machine sitting between the raw
//! `RdpEvent::ChannelData` a caller gets from `net` on the `"DRDYNVC"`
//! channel and the higher-level protocols (RDPGFX, redirection) that open
//! named channels over it:
//!
//! * [`DvcManager::process`] takes one reassembled DVC PDU and returns a
//!   [`DvcStep`]: an optional reply to send back (auto-accepting `Create`
//!   requests and echoing `Capabilities` requests) and an optional
//!   [`DvcEvent`] to hand to the caller (`ChannelOpened`, `Data`,
//!   `ChannelClosed`). It also reassembles a channel's own `DataFirst` +
//!   `Data` fragmentation (MS-RDPEDYC's own, nested *inside* the
//!   MS-RDPBCGR chunking [`crate::vchan`] already handles) into one message.
//! * [`DvcManager::close`] and [`crate::dvc::fragment`] are the outbound
//!   half — build the bytes, then hand each one to
//!   `RdpTransport::send_channel_data`.
//!
//! # Wiring it up
//!
//! Request `"DRDYNVC"` in `EstablishConfig::extra_channels`, note its granted
//! id from `RdpSession::channel_id`, then feed every `ChannelData` on that id
//! to a `DvcManager`:
//!
//! ```no_run
//! use rusty_rdp::dvc::DRDYNVC_CHANNEL_NAME;
//! use rusty_rdp::dvcman::{DvcEvent, DvcManager};
//! use rusty_rdp::net::{RdpEvent, RdpSession, RdpTransport};
//!
//! # fn demo<S: std::io::Read + std::io::Write>(
//! #     mut t: RdpTransport<S>,
//! #     session: RdpSession,
//! # ) -> std::io::Result<()> {
//! let dvc_channel = session.channel_id(DRDYNVC_CHANNEL_NAME).expect("server granted DRDYNVC");
//! let mut manager = DvcManager::new();
//! loop {
//!     let RdpEvent::ChannelData { channel_id, data } = t.recv_event()? else {
//!         continue;
//!     };
//!     if channel_id != dvc_channel {
//!         continue; // traffic on some other requested channel
//!     }
//!     let step = manager
//!         .process(&data)
//!         .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
//!     if let Some(reply) = step.response {
//!         t.send_channel_data(session.user_id, dvc_channel, &reply)?;
//!     }
//!     if let Some(DvcEvent::ChannelOpened { name, .. }) = step.event {
//!         // e.g. name == "Microsoft::Windows::RDS::Graphics": hand off to an
//!         // RDPGFX layer built on top (not yet implemented).
//!         let _ = name;
//!     }
//! }
//! # }
//! ```
//!
//! Every channel the server asks to open is accepted unconditionally
//! (`creation_status = 0`); a caller uninterested in a given channel simply
//! ignores its events.

use std::collections::HashMap;

use crate::dvc::{
    self, CapabilitiesResponsePdu, CapsRequest, ClosePdu, CreateRequestPdu, CreateResponsePdu,
    DataFirstPdu, DataPdu, CMD_CAPABILITY, CMD_CLOSE, CMD_CREATE, CMD_DATA, CMD_DATA_FIRST,
};
use crate::error::{Error, Result};

/// A high-level event surfaced by [`DvcManager::process`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DvcEvent {
    /// The server opened a named dynamic channel (already accepted).
    ChannelOpened {
        /// The channel id to use for [`DvcManager::close`] and outbound data.
        channel_id: u32,
        /// The DVC-based protocol's registered name.
        name: String,
    },
    /// A complete message on an open channel (DVC-level fragments already
    /// reassembled).
    Data {
        /// The channel it arrived on.
        channel_id: u32,
        /// The reassembled message.
        data: Vec<u8>,
    },
    /// The channel was closed (by either side).
    ChannelClosed {
        /// The channel that closed.
        channel_id: u32,
    },
}

/// The outcome of feeding one PDU to [`DvcManager::process`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DvcStep {
    /// A reply to send back on the `"DRDYNVC"` channel, if the manager
    /// needs to acknowledge anything (`Create`/`Capabilities` responses).
    pub response: Option<Vec<u8>>,
    /// An event to surface to the caller, if this PDU produced one.
    pub event: Option<DvcEvent>,
}

/// In-progress reassembly state for one open channel's fragmented message.
struct Reassembly {
    total_len: usize,
    buf: Vec<u8>,
}

/// A named, open dynamic channel.
struct Channel {
    name: String,
    reassembly: Option<Reassembly>,
}

/// Tracks open dynamic channels and answers the server's `Create` and
/// `Capabilities` requests automatically. See the [module docs](self) for how
/// to wire it into [`crate::net::RdpTransport`].
#[derive(Default)]
pub struct DvcManager {
    channels: HashMap<u32, Channel>,
}

impl DvcManager {
    /// Create an empty manager (no channels open).
    pub fn new() -> Self {
        DvcManager::default()
    }

    /// The names of currently open channels, keyed by channel id.
    pub fn open_channels(&self) -> impl Iterator<Item = (u32, &str)> {
        self.channels.iter().map(|(&id, c)| (id, c.name.as_str()))
    }

    /// Feed one fully-reassembled DVC PDU (the payload of an
    /// `RdpEvent::ChannelData` on the `"DRDYNVC"` channel) and get back
    /// what to do next.
    pub fn process(&mut self, pdu: &[u8]) -> Result<DvcStep> {
        match dvc::peek_cmd(pdu)? {
            CMD_CREATE => self.on_create(pdu),
            CMD_DATA_FIRST => self.on_data_first(pdu),
            CMD_DATA => self.on_data(pdu),
            CMD_CLOSE => self.on_close(pdu),
            CMD_CAPABILITY => self.on_capability(pdu),
            other => Err(Error::InvalidValue {
                field: "DVC Cmd",
                value: format!("0x{other:02X}"),
            }),
        }
    }

    /// Build a `DYNVC_CLOSE` for `channel_id` and drop its local state. Send
    /// the result via `RdpTransport::send_channel_data`.
    pub fn close(&mut self, channel_id: u32) -> Vec<u8> {
        self.channels.remove(&channel_id);
        ClosePdu { channel_id }.encode()
    }

    fn on_create(&mut self, pdu: &[u8]) -> Result<DvcStep> {
        let req = CreateRequestPdu::decode(pdu)?;
        self.channels.insert(
            req.channel_id,
            Channel {
                name: req.channel_name.clone(),
                reassembly: None,
            },
        );
        let response = CreateResponsePdu {
            channel_id: req.channel_id,
            creation_status: 0,
        }
        .encode();
        Ok(DvcStep {
            response: Some(response),
            event: Some(DvcEvent::ChannelOpened {
                channel_id: req.channel_id,
                name: req.channel_name,
            }),
        })
    }

    fn on_data_first(&mut self, pdu: &[u8]) -> Result<DvcStep> {
        let first = DataFirstPdu::decode(pdu)?;
        if first.data.len() as u32 >= first.total_length {
            // The "first" fragment is already the whole message.
            return Ok(DvcStep {
                response: None,
                event: Some(DvcEvent::Data {
                    channel_id: first.channel_id,
                    data: first.data,
                }),
            });
        }
        if let Some(channel) = self.channels.get_mut(&first.channel_id) {
            channel.reassembly = Some(Reassembly {
                total_len: first.total_length as usize,
                buf: first.data,
            });
        }
        // No channel known for this id: silently drop rather than erroring,
        // since a Close/Create race is not a protocol violation on our part.
        Ok(DvcStep::default())
    }

    fn on_data(&mut self, pdu: &[u8]) -> Result<DvcStep> {
        let d = DataPdu::decode(pdu)?;
        let Some(channel) = self.channels.get_mut(&d.channel_id) else {
            return Ok(DvcStep::default());
        };
        let Some(reassembly) = channel.reassembly.as_mut() else {
            // No DataFirst preceded this: it is itself a complete message.
            return Ok(DvcStep {
                response: None,
                event: Some(DvcEvent::Data {
                    channel_id: d.channel_id,
                    data: d.data,
                }),
            });
        };
        reassembly.buf.extend_from_slice(&d.data);
        if reassembly.buf.len() >= reassembly.total_len {
            let data = std::mem::take(&mut reassembly.buf);
            channel.reassembly = None;
            return Ok(DvcStep {
                response: None,
                event: Some(DvcEvent::Data {
                    channel_id: d.channel_id,
                    data,
                }),
            });
        }
        Ok(DvcStep::default())
    }

    fn on_close(&mut self, pdu: &[u8]) -> Result<DvcStep> {
        let close = ClosePdu::decode(pdu)?;
        self.channels.remove(&close.channel_id);
        Ok(DvcStep {
            response: None,
            event: Some(DvcEvent::ChannelClosed {
                channel_id: close.channel_id,
            }),
        })
    }

    fn on_capability(&mut self, pdu: &[u8]) -> Result<DvcStep> {
        let req = CapsRequest::decode(pdu)?;
        let response = CapabilitiesResponsePdu {
            version: req.version(),
        }
        .encode();
        Ok(DvcStep {
            response: Some(response),
            event: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dvc::{fragment, CapsRequest, CAPS_VERSION2};

    #[test]
    fn create_request_is_accepted_and_opens_channel() {
        let mut mgr = DvcManager::new();
        let req = CreateRequestPdu {
            channel_id: 3,
            channel_name: "Microsoft::Windows::RDS::Graphics".to_string(),
        };
        let step = mgr.process(&req.encode()).unwrap();

        let resp = CreateResponsePdu::decode(step.response.as_ref().unwrap()).unwrap();
        assert_eq!(resp.channel_id, 3);
        assert!(resp.succeeded());
        assert_eq!(
            step.event,
            Some(DvcEvent::ChannelOpened {
                channel_id: 3,
                name: "Microsoft::Windows::RDS::Graphics".to_string(),
            })
        );
        assert_eq!(
            mgr.open_channels().collect::<Vec<_>>(),
            vec![(3, "Microsoft::Windows::RDS::Graphics")]
        );
    }

    #[test]
    fn capability_request_is_echoed() {
        let mut mgr = DvcManager::new();
        let req = CapsRequest::V2 {
            priority_charges: [936, 3276, 9362, 21845],
        };
        let step = mgr.process(&req.encode()).unwrap();
        let resp = CapabilitiesResponsePdu::decode(step.response.as_ref().unwrap()).unwrap();
        assert_eq!(resp.version, CAPS_VERSION2);
        assert_eq!(step.event, None);
    }

    #[test]
    fn single_pdu_message_is_delivered_immediately() {
        let mut mgr = DvcManager::new();
        mgr.process(
            &CreateRequestPdu {
                channel_id: 7,
                channel_name: "test".to_string(),
            }
            .encode(),
        )
        .unwrap();

        let msg = b"a short message";
        let pdus = fragment(7, msg);
        assert_eq!(pdus.len(), 1); // fits in one PDU, so no DataFirst.
        let step = mgr.process(&pdus[0]).unwrap();
        assert_eq!(
            step.event,
            Some(DvcEvent::Data {
                channel_id: 7,
                data: msg.to_vec(),
            })
        );
        assert!(step.response.is_none());
    }

    #[test]
    fn fragmented_message_reassembles_and_reports_once() {
        let mut mgr = DvcManager::new();
        mgr.process(
            &CreateRequestPdu {
                channel_id: 9,
                channel_name: "test".to_string(),
            }
            .encode(),
        )
        .unwrap();

        let msg: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let pdus = fragment(9, &msg);
        assert!(pdus.len() > 2, "test needs a multi-chunk message");

        // Every PDU but the last produces no event...
        for p in &pdus[..pdus.len() - 1] {
            let step = mgr.process(p).unwrap();
            assert_eq!(step.event, None);
            assert!(step.response.is_none());
        }
        // ...and the last one delivers the whole reassembled message.
        let last = mgr.process(&pdus[pdus.len() - 1]).unwrap();
        assert_eq!(
            last.event,
            Some(DvcEvent::Data {
                channel_id: 9,
                data: msg,
            })
        );
    }

    #[test]
    fn close_removes_channel_and_reports_event() {
        let mut mgr = DvcManager::new();
        mgr.process(
            &CreateRequestPdu {
                channel_id: 11,
                channel_name: "test".to_string(),
            }
            .encode(),
        )
        .unwrap();
        assert_eq!(mgr.open_channels().count(), 1);

        let step = mgr.process(&ClosePdu { channel_id: 11 }.encode()).unwrap();
        assert_eq!(step.event, Some(DvcEvent::ChannelClosed { channel_id: 11 }));
        assert_eq!(mgr.open_channels().count(), 0);
    }

    #[test]
    fn client_initiated_close_matches_server_close_shape() {
        let mut mgr = DvcManager::new();
        mgr.process(
            &CreateRequestPdu {
                channel_id: 4,
                channel_name: "test".to_string(),
            }
            .encode(),
        )
        .unwrap();

        let close_bytes = mgr.close(4);
        assert_eq!(ClosePdu::decode(&close_bytes).unwrap().channel_id, 4);
        assert_eq!(mgr.open_channels().count(), 0);
    }

    #[test]
    fn data_on_unknown_channel_is_ignored_not_erroring() {
        let mut mgr = DvcManager::new();
        // No CreateRequest was ever processed for channel 99.
        let step = mgr
            .process(
                &DataPdu {
                    channel_id: 99,
                    data: vec![1, 2, 3],
                }
                .encode(),
            )
            .unwrap();
        assert_eq!(step.event, None);
        assert!(step.response.is_none());
    }
}
