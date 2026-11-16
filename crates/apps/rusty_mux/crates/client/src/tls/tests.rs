use super::*;

#[test]
fn hex_round_trips_and_refuses_the_wrong_length() {
    let bytes = [0xab; 32];
    assert_eq!(unhex::<32>(&hex(&bytes)), Some(bytes));
    assert_eq!(unhex::<32>(&hex(&bytes).to_uppercase()), Some(bytes));
    assert_eq!(unhex::<32>("ab"), None);
    assert_eq!(unhex::<32>(&"zz".repeat(32)), None);
    assert_eq!(unhex::<2>("é1"), None);
}

#[test]
fn an_address_needs_a_host_and_a_port() {
    assert!(resolve("127.0.0.1:7000").is_ok());
    assert_eq!(resolve("[::1]:7000").map(|(_, h)| h), Ok("::1".to_owned()));
    assert!(resolve("localhost").is_err());
    assert!(resolve(":7000").is_err());
}

// A throwaway self-signed certificate for `localhost` and its key; the key
// is a test key and protects nothing.
const CERT: &str = "MIIBlTCCATugAwIBAgIUNVCXzQ2FEC2YXUZSvYW3emOili0wCgYIKoZIzj0EAwIwFDESMBAGA1UEAwwJbG9jYWxob3N0MCAXDTI2MTExMzIxMDUwMVoYDzIxMjYxMDIwMjEwNTAxWjAUMRIwEAYDVQQDDAlsb2NhbGhvc3QwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAASGBIhUN8Mhly8WdTpSksdLzUi0wz+clGPZsfzZyD7dFD9T7CvJQK3hp5wfwXMx41/9AwO1zAKomyfSZ3t/4nLEo2kwZzAdBgNVHQ4EFgQUdz4TCJsOC9W3ESAwqTBkw2vpuLUwHwYDVR0jBBgwFoAUdz4TCJsOC9W3ESAwqTBkw2vpuLUwDwYDVR0TAQH/BAUwAwEB/zAUBgNVHREEDTALgglsb2NhbGhvc3QwCgYIKoZIzj0EAwIDSAAwRQIhAM/uQE1htwBDUiTWpEzxPvFjWH7CUGDzIOpg3S+ILq5SAiA/2pdwmEPSIk0accKOYLRpcxeOQGWPugOzlJHK3rV74w==";
const KEY: &str = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg+i39n8x4u+BcjKalH5jKPPeqGm3LgDG6fHidpE1ifRihRANCAASGBIhUN8Mhly8WdTpSksdLzUi0wz+clGPZsfzZyD7dFD9T7CvJQK3hp5wfwXMx41/9AwO1zAKomyfSZ3t/4nLE";
const TOKEN: [u8; TOKEN_LEN] = [7; TOKEN_LEN];

/// A TLS server that checks the token and echoes bytes, once; its address.
fn server() -> (String, Pin) {
    use std::io::{Read, Write};
    let cert = rusty_base64::decode_standard(CERT).unwrap();
    let key = rusty_base64::decode_standard(KEY).unwrap();
    let pin = fingerprint_sha256(&cert).unwrap();
    let acceptor = rusty_tls::TlsAcceptor::new(vec![cert], key).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        // Each connection: the probe (no token) and the relayed one.
        for tcp in listener.incoming().flatten() {
            let acceptor = acceptor.clone();
            std::thread::spawn(move || {
                let Ok(mut tls) = acceptor.accept(tcp) else {
                    return;
                };
                let mut token = [0u8; TOKEN_LEN];
                if tls.read_exact(&mut token).is_err() || token != TOKEN {
                    return;
                }
                let mut buf = [0u8; 2];
                if tls.read_exact(&mut buf).is_ok() {
                    let _ = tls.write_all(&buf);
                }
            });
        }
    });
    (addr, pin)
}

#[test]
fn the_pinned_server_gets_the_token_first_and_bytes_relay_both_ways() {
    use std::io::{Read, Write};
    let (addr, pin) = server();
    assert_eq!(fingerprint(&addr), Ok(pin));
    let relay = Relay::start(&addr, pin, TOKEN).unwrap();
    let mut local = std::os::unix::net::UnixStream::connect(relay.socket()).unwrap();
    local
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    local.write_all(b"hi").unwrap();
    let mut echoed = [0u8; 2];
    local.read_exact(&mut echoed).unwrap();
    assert_eq!(&echoed, b"hi");
    let dir = relay.dir.clone();
    drop(relay);
    assert!(!dir.exists(), "the relay's directory was removed");
}

#[test]
fn a_changed_certificate_is_refused_with_both_fingerprints() {
    let (addr, pin) = server();
    let mut other = pin;
    other[0] ^= 1;
    let err = Relay::start(&addr, other, TOKEN).err().expect("refused");
    assert!(matches!(err, StartError::Changed(_)), "{err:?}");
    let err = err.to_string();
    assert!(
        err.contains(&hex(&other)) && err.contains(&hex(&pin)),
        "{err}"
    );
}

#[test]
fn a_server_that_is_not_there_is_an_error() {
    let err = fingerprint("127.0.0.1:1").unwrap_err();
    assert!(err.contains("127.0.0.1:1"), "{err}");
}
