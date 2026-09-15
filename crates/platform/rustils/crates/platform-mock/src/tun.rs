//! `Tun` mock: a scriptable in-memory device, not a routed one.
//!
//! Unlike `MockUdpSocket`/`MockTcpStream` (`crate::net`), there is no
//! peer-to-peer delivery to fake here — a TUN device has no "other side"
//! within this process to connect a channel to; the real "other side" is
//! the kernel's own routing table deciding what to send down the tunnel.
//! So this mock is deliberately simpler: a test queues raw packets it
//! wants `read()` to hand back (standing in for "the kernel routed this
//! into the tunnel"), and every `write()` is recorded for the test to
//! assert against afterward (standing in for "the local stack received
//! this"). Nothing routes between the two.
//!
//! [`platform::tun::TunDevice`] documents `read`/`write` as blocking — the
//! real backend's fd genuinely blocks the calling thread until the kernel
//! has something to deliver. This mock has no kernel to wait on, so it
//! does not block: `read()` on an empty queue returns `Ok(0)` immediately,
//! the same "diverges from real blocking semantics because there is
//! nothing to block on" tradeoff `MockCsprng` makes for randomness.

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::sync::Mutex;

use platform::error::Result;
use platform::tun::{Tun, TunDevice};

use crate::sync::lock;

/// The mock backend's [`Tun`] capability.
pub struct MockTun;

impl Tun for MockTun {
    fn create(
        &self,
        name: &str,
        ipv4: Ipv4Addr,
        prefix_len: u8,
        mtu: u32,
    ) -> Result<Box<dyn TunDevice>> {
        Ok(Box::new(MockTunDevice::new(name, ipv4, prefix_len, mtu)))
    }
}

/// A scriptable in-memory stand-in for a created TUN device. See this
/// module's doc comment for what `read`/`write` actually do here.
pub struct MockTunDevice {
    name: String,
    ipv4: Ipv4Addr,
    prefix_len: u8,
    mtu: u32,
    inbound: Mutex<VecDeque<Vec<u8>>>,
    written: Mutex<Vec<Vec<u8>>>,
}

impl MockTunDevice {
    pub fn new(name: &str, ipv4: Ipv4Addr, prefix_len: u8, mtu: u32) -> Self {
        MockTunDevice {
            name: name.to_string(),
            ipv4,
            prefix_len,
            mtu,
            inbound: Mutex::new(VecDeque::new()),
            written: Mutex::new(Vec::new()),
        }
    }

    /// Test setup: queue a packet for a future `read()` to hand back, as
    /// if the kernel had just routed it into the tunnel.
    pub fn queue_inbound(&self, packet: Vec<u8>) {
        lock(&self.inbound).push_back(packet);
    }

    /// Test assertion: every packet `write()` has recorded so far, in
    /// call order.
    pub fn written_packets(&self) -> Vec<Vec<u8>> {
        lock(&self.written).clone()
    }

    pub fn ipv4(&self) -> Ipv4Addr {
        self.ipv4
    }

    pub fn prefix_len(&self) -> u8 {
        self.prefix_len
    }

    pub fn mtu(&self) -> u32 {
        self.mtu
    }
}

impl TunDevice for MockTunDevice {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        match lock(&self.inbound).pop_front() {
            Some(packet) => {
                let n = packet.len().min(buf.len());
                buf[..n].copy_from_slice(&packet[..n]);
                Ok(n)
            }
            None => Ok(0),
        }
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        lock(&self.written).push(buf.to_vec());
        Ok(buf.len())
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_reports_the_requested_shape() {
        let tun = MockTun;
        let device = tun
            .create("mock0", Ipv4Addr::new(10, 0, 0, 1), 24, 1500)
            .expect("create");
        assert_eq!(device.name(), "mock0");
    }

    #[test]
    fn queued_inbound_packets_are_read_back_in_order() {
        let device = MockTunDevice::new("mock0", Ipv4Addr::new(10, 0, 0, 1), 24, 1500);
        device.queue_inbound(vec![1, 2, 3]);
        device.queue_inbound(vec![4, 5]);

        let mut buf = [0u8; 16];
        let n = device.read(&mut buf).expect("read first");
        assert_eq!(&buf[..n], &[1, 2, 3]);
        let n = device.read(&mut buf).expect("read second");
        assert_eq!(&buf[..n], &[4, 5]);
        let n = device.read(&mut buf).expect("read empty");
        assert_eq!(n, 0);
    }

    #[test]
    fn written_packets_are_recorded_for_assertions() {
        let device = MockTunDevice::new("mock0", Ipv4Addr::new(10, 0, 0, 1), 24, 1500);
        device.write(b"hello").expect("write");
        device.write(b"world").expect("write");
        assert_eq!(
            device.written_packets(),
            vec![b"hello".to_vec(), b"world".to_vec()]
        );
    }
}
