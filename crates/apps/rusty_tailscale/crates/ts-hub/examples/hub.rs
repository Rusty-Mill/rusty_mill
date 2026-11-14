//! Runs a hub for interop checks: `hub <listen addr:port> <advertised host> <preauth key>`.
//! The relay shares the TCP port; STUN answers on the same port over UDP.

use rusty_tokio::net::{TcpListener, UdpSocket};
use ts_hub::{Hub, stun};

#[rusty_tokio::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [listen, host, key] = args.as_slice() else {
        eprintln!("usage: hub <listen addr:port> <advertised host> <preauth key>");
        std::process::exit(2);
    };
    let tcp = TcpListener::bind(listen.as_str()).await?;
    let port = tcp.local_addr()?.port();
    let udp = UdpSocket::bind(tcp.local_addr()?).await?;

    let hub = Hub::new();
    hub.add_preauth(key, "mesh");
    hub.advertise(host, port, port);
    println!("hub on http://{host}:{port} (STUN udp/{port})");
    rusty_tokio::spawn(stun::serve(udp));
    hub.serve(tcp).await
}
