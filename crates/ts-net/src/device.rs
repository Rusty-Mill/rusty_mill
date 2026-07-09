//! A smoltcp `Device` whose "wire" is the tailscale-rs engine: received IP
//! packets come from the engine (decrypted WireGuard payloads) and transmitted
//! packets go back to the engine to be encapsulated and sent onto the tailnet.

use std::collections::VecDeque;

use smoltcp::phy::{
    Checksum, ChecksumCapabilities, Device, DeviceCapabilities, Medium, RxToken, TxToken,
};
use smoltcp::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

/// The tailnet MTU (matches `ts_tun::DEFAULT_MTU`); WireGuard + DERP framing
/// fits comfortably under the physical path MTU.
pub const MTU: usize = 1280;

/// A packet-queue device bridging smoltcp to the engine.
pub struct ChannelDevice {
    /// Inbound IP packets from the engine, awaiting smoltcp processing.
    rx: VecDeque<Vec<u8>>,
    /// Outbound IP packets smoltcp produced, sent to the engine.
    tx: UnboundedSender<Vec<u8>>,
}

impl ChannelDevice {
    pub fn new(tx: UnboundedSender<Vec<u8>>) -> Self {
        Self {
            rx: VecDeque::new(),
            tx,
        }
    }

    /// Queues an inbound packet for smoltcp to consume on the next poll.
    pub fn push_rx(&mut self, packet: Vec<u8>) {
        self.rx.push_back(packet);
    }
}

impl Device for ChannelDevice {
    type RxToken<'a> = QueueRxToken;
    type TxToken<'a> = QueueTxToken<'a>;

    fn receive(&mut self, _now: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let packet = self.rx.pop_front()?;
        Some((QueueRxToken { packet }, QueueTxToken { tx: &self.tx }))
    }

    fn transmit(&mut self, _now: Instant) -> Option<Self::TxToken<'_>> {
        Some(QueueTxToken { tx: &self.tx })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = MTU;
        // The engine sends our packets straight onto the tailnet with no
        // hardware offload, so smoltcp must compute/verify checksums itself.
        let mut checksum = ChecksumCapabilities::default();
        checksum.ipv4 = Checksum::Both;
        checksum.tcp = Checksum::Both;
        checksum.udp = Checksum::Both;
        checksum.icmpv4 = Checksum::Both;
        caps.checksum = checksum;
        caps
    }
}

pub struct QueueRxToken {
    packet: Vec<u8>,
}

impl RxToken for QueueRxToken {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.packet)
    }
}

pub struct QueueTxToken<'a> {
    tx: &'a UnboundedSender<Vec<u8>>,
}

impl TxToken for QueueTxToken<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut buf = vec![0u8; len];
        let r = f(&mut buf);
        // A closed channel just means the engine stopped; drop the packet.
        let _ = self.tx.send(buf);
        r
    }
}
