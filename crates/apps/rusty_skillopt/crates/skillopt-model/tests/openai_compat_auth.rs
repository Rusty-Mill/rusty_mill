use std::io::{Read, Write};
use std::net::TcpListener;

use skillopt_core::{ChatBackend, Message};
use skillopt_model::OpenAiCompatBackend;

/// Starts a one-shot HTTP server that captures the request headers of the
/// first connection, replies with a minimal valid chat-completion response,
/// and returns the captured headers (lowercased) once the client is done.
fn capture_one_request(
    addr_tx: std::sync::mpsc::Sender<String>,
) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        addr_tx
            .send(listener.local_addr().unwrap().to_string())
            .unwrap();

        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 8192];
        let mut received = String::new();
        // Read until we see the end of headers ("\r\n\r\n"); the body isn't
        // needed for this test.
        loop {
            let n = stream.read(&mut buf).unwrap();
            received.push_str(&String::from_utf8_lossy(&buf[..n]));
            if received.contains("\r\n\r\n") || n == 0 {
                break;
            }
        }

        let body = r#"{"choices":[{"message":{"content":"ok"}}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();

        received.to_lowercase()
    })
}

#[tokio::test]
async fn sends_no_authorization_header_when_api_key_is_none() {
    let (addr_tx, addr_rx) = std::sync::mpsc::channel();
    let server = capture_one_request(addr_tx);
    let addr = addr_rx.recv().unwrap();

    let backend = OpenAiCompatBackend::new(
        None,
        Some(format!("http://{addr}")),
        "test-model".into(),
        None,
        64,
    );
    backend.chat(&[Message::user("hi")]).await.unwrap();

    let headers = server.join().unwrap();
    assert!(
        !headers.contains("authorization:"),
        "expected no Authorization header, got:\n{headers}"
    );
}

#[tokio::test]
async fn sends_authorization_header_when_api_key_is_set() {
    let (addr_tx, addr_rx) = std::sync::mpsc::channel();
    let server = capture_one_request(addr_tx);
    let addr = addr_rx.recv().unwrap();

    let backend = OpenAiCompatBackend::new(
        Some("secret-key".into()),
        Some(format!("http://{addr}")),
        "test-model".into(),
        None,
        64,
    );
    backend.chat(&[Message::user("hi")]).await.unwrap();

    let headers = server.join().unwrap();
    assert!(
        headers.contains("authorization: bearer secret-key"),
        "expected Authorization header, got:\n{headers}"
    );
}
