//! Connect to an RDP server and drive the deterministic connection sequence.
//!
//! ```sh
//! cargo run --example connect -- 192.0.2.10:3389 alice
//! ```
//!
//! This performs the X.224 security negotiation and, when the server selects
//! standard RDP security, continues through the GCC/MCS connect and channel
//! setup, printing each step. The security-dependent PDUs (Security Exchange,
//! Client Info, capabilities) are intentionally not driven here — this example
//! exercises the parts that need no cryptographic handshake against the server.

use std::net::TcpStream;
use std::process::ExitCode;

use rusty_rdp::gcc::{
    ClientClusterData, ClientCoreData, ClientNetworkData, ClientSecurityData, ServerNetworkData,
    UserDataBlock, ENCRYPTION_METHOD_128BIT, ENCRYPTION_METHOD_40BIT,
};
use rusty_rdp::mcs::MCS_GLOBAL_CHANNEL_ID;
use rusty_rdp::nego::SecurityProtocols;
use rusty_rdp::net::RdpTransport;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:3389".to_string());
    let username = args.next();

    match run(&addr, username.as_deref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(addr: &str, username: Option<&str>) -> std::io::Result<()> {
    println!("connecting to {addr} ...");
    let stream = TcpStream::connect(addr)?;
    let mut rdp = RdpTransport::new(stream);

    // 1. X.224 security negotiation. Offer everything we can frame; the server
    // picks one.
    let requested = SecurityProtocols::RDP | SecurityProtocols::SSL | SecurityProtocols::HYBRID;
    let selected = rdp.negotiate(requested, username)?;
    println!("negotiated security: {selected:?}");

    if selected != SecurityProtocols::RDP {
        println!(
            "server selected TLS/CredSSP ({selected:?}); this example only drives the \
             standard-RDP path further, so stopping here."
        );
        return Ok(());
    }

    // 2. MCS connect: send our client settings, read the server's back.
    let client_blocks = vec![
        UserDataBlock::ClientCore(ClientCoreData::new(1024, 768, "rusty-rdp")),
        UserDataBlock::ClientSecurity(ClientSecurityData {
            encryption_methods: ENCRYPTION_METHOD_40BIT | ENCRYPTION_METHOD_128BIT,
            ext_encryption_methods: 0,
        }),
        UserDataBlock::ClientNetwork(ClientNetworkData { channels: vec![] }),
        UserDataBlock::ClientCluster(ClientClusterData {
            flags: 0x0D,
            redirected_session_id: 0,
        }),
    ];
    let server_blocks = rdp.mcs_connect(&client_blocks)?;
    println!(
        "MCS connect complete; server sent {} block(s)",
        server_blocks.len()
    );

    let io_channel = server_blocks
        .iter()
        .find_map(|b| match b {
            UserDataBlock::ServerNetwork(ServerNetworkData { io_channel_id, .. }) => {
                Some(*io_channel_id)
            }
            _ => None,
        })
        .unwrap_or(MCS_GLOBAL_CHANNEL_ID);

    // 3. Channel setup.
    rdp.erect_domain()?;
    let user_id = rdp.attach_user()?;
    println!("attached as MCS user {user_id}");

    rdp.join_channel(user_id, user_id)?;
    rdp.join_channel(user_id, io_channel)?;
    println!("joined user channel {user_id} and I/O channel {io_channel}");

    println!(
        "connection sequence up to channel setup complete. The security \
         commencement, Client Info, and capability exchange come next."
    );
    Ok(())
}
