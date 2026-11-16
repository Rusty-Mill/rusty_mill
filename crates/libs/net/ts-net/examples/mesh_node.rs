//! A probe for the two-NAT harness (`interop/two_nat_test.sh`): one `ts-net`
//! node that receives or sends a paced byte stream and reports which path the
//! peer is on.
//!
//! `mesh_node serve <hub url> <stun host:port> <authkey> <name> <port> <total bytes>`
//! `mesh_node send  <hub url> <stun host:port> <authkey> <name> <peer ip> <port> <total bytes>`
//!
//! The sender writes 32 KiB every 50 ms and prints `path=` each second:
//! `direct <addr>` once the peer's current endpoint is a direct address,
//! `relay` before that. It ends with `DONE sent N first_direct=<secs|never>`.
//! The receiver checks every byte against the sender's pattern and ends with
//! `DONE recv N ok`; it exits non-zero on a short, wrong or late stream.

#[cfg(target_os = "linux")]
mod probe {
    use std::net::{IpAddr, SocketAddr};
    use std::process::ExitCode;
    use std::time::{Duration, Instant};

    use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt};
    use ts_net::{Node, NodeConfig, Trust};

    const CHUNK: usize = 32 << 10;
    const PACE: Duration = Duration::from_millis(50);
    /// The receiver gives up when nothing arrives for this long.
    const STALL: Duration = Duration::from_secs(30);

    fn pattern(offset: usize, len: usize) -> Vec<u8> {
        (offset..offset + len)
            .map(|i| (i as u32).wrapping_mul(2654435761).to_be_bytes()[0])
            .collect()
    }

    async fn start(url: &str, stun: &str, key: &str, name: &str) -> Result<Node, String> {
        let state_dir =
            std::env::temp_dir().join(format!("mesh-node-{name}-{}", std::process::id()));
        let node = Node::new(NodeConfig {
            control_url: url.to_string(),
            derp_url: None,
            authkey: key.to_string(),
            hostname: name.to_string(),
            state_dir,
            enable_direct: true,
            stun_server: Some(stun.to_string()),
            trust: Trust::System,
        })
        .await
        .map_err(|e| format!("register with the hub: {e}"))?;
        node.wait_ip().await.ok_or("no tailnet address")?;
        Ok(node)
    }

    /// `Some(addr)` once the peer's current endpoint is direct.
    async fn direct(node: &Node) -> Option<String> {
        let status = node.status().await?;
        let peer = status.peer.values().next()?;
        (!peer.cur_addr.is_empty()).then(|| peer.cur_addr.clone())
    }

    async fn serve(a: &[&str]) -> Result<(), String> {
        let [url, stun, key, name, port, total] = a else {
            return Err("serve: wrong number of arguments".into());
        };
        let port: u16 = port.parse().map_err(|_| "bad port")?;
        let total: usize = total.parse().map_err(|_| "bad total")?;
        let node = start(url, stun, key, name).await?;
        let mut listener = node.bind(port).await.map_err(|e| e.to_string())?;
        println!("READY {}", node.tailnet_ip().await.ok_or("no address")?);
        let mut stream = listener.accept().await.ok_or("no connection")?;
        let (mut got, mut buf, mut last) = (0usize, vec![0u8; CHUNK], Instant::now());
        while got < total {
            let n = rusty_tokio::time::timeout(STALL, stream.read(&mut buf))
                .await
                .map_err(|_| format!("stalled after {got} bytes"))?
                .map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            if buf[..n] != pattern(got, n)[..] {
                return Err(format!("bytes differ in the {n} bytes after {got}"));
            }
            got += n;
            if last.elapsed() >= Duration::from_secs(1) {
                println!("recv {got}");
                last = Instant::now();
            }
        }
        if got != total {
            return Err(format!("stream ended at {got} of {total} bytes"));
        }
        println!("DONE recv {got} ok");
        Ok(())
    }

    async fn send(a: &[&str]) -> Result<(), String> {
        let [url, stun, key, name, peer, port, total] = a else {
            return Err("send: wrong number of arguments".into());
        };
        let peer: IpAddr = peer.parse().map_err(|_| "bad peer ip")?;
        let port: u16 = port.parse().map_err(|_| "bad port")?;
        let total: usize = total.parse().map_err(|_| "bad total")?;
        let node = start(url, stun, key, name).await?;
        let mut stream = node
            .connect(SocketAddr::new(peer, port))
            .await
            .map_err(|e| format!("dial: {e}"))?;
        let (begun, mut sent, mut last) = (Instant::now(), 0usize, Instant::now());
        let mut first_direct: Option<u64> = None;
        while sent < total {
            let chunk = pattern(sent, CHUNK.min(total - sent));
            stream.write_all(&chunk).await.map_err(|e| e.to_string())?;
            sent += chunk.len();
            if last.elapsed() >= Duration::from_secs(1) {
                let path = direct(&node).await;
                first_direct = first_direct.or(path.as_ref().map(|_| begun.elapsed().as_secs()));
                println!(
                    "sent {sent} path={}",
                    path.map_or("relay".into(), |p| format!("direct {p}"))
                );
                last = Instant::now();
            }
            rusty_tokio::time::sleep(PACE).await;
        }
        let late = direct(&node).await.map(|_| begun.elapsed().as_secs());
        let first = first_direct
            .or(late)
            .map_or("never".into(), |s| s.to_string());
        println!("DONE sent {sent} first_direct={first}");
        // The stack closes only after its queue drains; give it time.
        rusty_tokio::time::sleep(Duration::from_secs(10)).await;
        Ok(())
    }

    #[rusty_tokio::main(flavor = "current_thread")]
    pub async fn main() -> ExitCode {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        let result = match a.split_first() {
            Some((&"serve", rest)) => serve(rest).await,
            Some((&"send", rest)) => send(rest).await,
            _ => Err("usage: see the file header".into()),
        };
        match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("mesh_node: {e}");
                ExitCode::FAILURE
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    probe::main()
}

#[cfg(not(target_os = "linux"))]
fn main() {}
