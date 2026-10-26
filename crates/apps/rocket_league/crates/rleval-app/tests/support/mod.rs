//! Real-process startup without releasing a reserved port or losing child diagnostics.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const SERVING: &str = "RLEval unified app serving on http://";
const LOG_LIMIT: usize = 64 * 1024;

#[derive(Default)]
struct Output {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    address: Option<SocketAddr>,
}

struct Capture {
    output: Arc<Mutex<Output>>,
    readers: Vec<JoinHandle<()>>,
}

/// A running `rleval serve`, killed and reaped on drop. `.1` is its actual bound port.
pub struct Server(Child, pub u16, Capture);

impl Server {
    pub fn start() -> Self {
        let replays = format!("{}/../../rleval/assets/replays", env!("CARGO_MANIFEST_DIR"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_rleval"));
        command.args([
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            "--replays",
            &replays,
        ]);
        Self::start_command(command, Duration::from_secs(5))
            .unwrap_or_else(|error| panic!("{error}"))
    }

    pub fn start_command(mut command: Command, timeout: Duration) -> Result<Self, String> {
        let started = Instant::now();
        let child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("spawn rleval: {error}"))?;
        let mut server = Self(
            child,
            0,
            Capture {
                output: Arc::new(Mutex::new(Output::default())),
                readers: Vec::new(),
            },
        );
        let stdout = server.0.stdout.take().expect("piped stdout");
        let stderr = server.0.stderr.take().expect("piped stderr");
        server.capture(stdout, false);
        server.capture(stderr, true);
        let deadline = started + timeout;
        let mut last_probe = "no bound address reported".to_string();
        let reason = loop {
            match server.0.try_wait() {
                Ok(Some(status)) => break format!("child exited: {status}"),
                Err(error) => break format!("checking child exit: {error}"),
                Ok(None) => {}
            }
            if Instant::now() >= deadline {
                break "startup deadline elapsed".to_string();
            }
            let address = server.2.output.lock().unwrap().address;
            if let Some(address) = address {
                server.1 = address.port();
                match probe(
                    address,
                    deadline.min(Instant::now() + Duration::from_millis(250)),
                ) {
                    Ok(()) => {
                        // Do not accept a health response from a child already known to have exited.
                        match server.0.try_wait() {
                            Ok(None) => return Ok(server),
                            Ok(Some(status)) => break format!("child exited: {status}"),
                            Err(error) => break format!("checking child exit: {error}"),
                        }
                    }
                    Err(error) => last_probe = error.to_string(),
                }
            }
            thread::sleep(
                Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
            );
        };
        // Reap before joining readers so all final diagnostics are included on failure.
        server.stop();
        let output = server.2.output.lock().unwrap();
        Err(format!(
            "server did not come up after {:?}: {reason}; address: {:?}; last probe: {last_probe}\nstdout (tail):\n{}\nstderr (tail):\n{}",
            started.elapsed(), output.address,
            String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)
        ))
    }

    fn capture(&mut self, mut pipe: impl Read + Send + 'static, stderr: bool) {
        let output = Arc::clone(&self.2.output);
        self.2.readers.push(thread::spawn(move || {
            let mut chunk = [0; 4096];
            loop {
                let count = match pipe.read(&mut chunk) {
                    Ok(0) => return,
                    Ok(count) => count,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        let mut output = output.lock().unwrap();
                        output.stderr.extend_from_slice(
                            format!("\nreading child output: {error}\n").as_bytes(),
                        );
                        return;
                    }
                };
                let mut output = output.lock().unwrap();
                let tail = if stderr {
                    &mut output.stderr
                } else {
                    &mut output.stdout
                };
                tail.extend_from_slice(&chunk[..count]);
                if tail.len() > LOG_LIMIT {
                    tail.drain(..tail.len() - LOG_LIMIT);
                }
                if stderr && output.address.is_none() {
                    output.address = reported_address(&output.stderr);
                }
            }
        }));
    }

    fn stop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        for reader in self.2.readers.drain(..) {
            reader.join().expect("child output reader");
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

fn reported_address(stderr: &[u8]) -> Option<SocketAddr> {
    String::from_utf8_lossy(stderr).lines().find_map(|line| {
        let url = line.strip_prefix(SERVING)?;
        // Require the separator following the complete address, even across reads.
        let (address, _) = url.split_once("  ")?;
        let address: SocketAddr = address.parse().ok()?;
        (address.ip().is_loopback() && address.port() != 0).then_some(address)
    })
}

/// Bound every connect/write/read by the remaining elapsed deadline, including slow peers.
pub fn probe(address: SocketAddr, deadline: Instant) -> std::io::Result<()> {
    let remaining = || {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "health probe deadline elapsed",
            ))
        } else {
            Ok(left)
        }
    };
    let mut stream = TcpStream::connect_timeout(&address, remaining()?)?;
    let mut request = &b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"[..];
    while !request.is_empty() {
        stream.set_write_timeout(Some(remaining()?))?;
        let count = stream.write(request)?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "health request write stopped",
            ));
        }
        request = &request[count..];
    }
    let mut response = Vec::new();
    loop {
        stream.set_read_timeout(Some(remaining()?))?;
        let mut chunk = [0; 1024];
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        response.extend_from_slice(&chunk[..count]);
        if response.len() > 4096 {
            return Err(std::io::Error::other("oversized health response"));
        }
    }
    if response.starts_with(b"HTTP/1.1 200 ") {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "health response: {}",
            String::from_utf8_lossy(&response)
        )))
    }
}
