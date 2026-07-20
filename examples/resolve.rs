//! Quick check of --internal-dns resolution against real public DNS servers.
fn main() {
    let host = std::env::args().nth(1).unwrap_or_else(|| "one.one.one.one".into());
    match rusty_croc::models::resolve_host(&host, true) {
        Some(ip) => println!("internal-dns: {host} -> {ip}"),
        None => println!("internal-dns: could not resolve {host} (public DNS unreachable?)"),
    }
    match rusty_croc::models::resolve_host(&host, false) {
        Some(ip) => println!("system-dns:   {host} -> {ip}"),
        None => println!("system-dns:   could not resolve {host}"),
    }
}
