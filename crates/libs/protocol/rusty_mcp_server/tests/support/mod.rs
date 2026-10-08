#![allow(dead_code, clippy::unwrap_used)]
//! Helpers shared by the tests that drive a server through the stdio loop on
//! in-memory pipes.

use rusty_json::Value;
use rusty_mcp_server::{serve_lines, Server, StdioConfig};
use std::io::{self, Cursor, Read, Write};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

/// A writer whose contents the test reads back.
#[derive(Clone, Default)]
pub struct Out(pub Arc<Mutex<Vec<u8>>>);

impl Write for Out {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Out {
    /// Every JSON-RPC message written so far, in order.
    pub fn messages(&self) -> Vec<Value> {
        let bytes = self.0.lock().unwrap().clone();
        String::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(|l| Value::from_json_str(l).unwrap_or_else(|e| panic!("{l}: {e}")))
            .collect()
    }
}

/// Feed `input` to `server` and return everything it wrote.
pub fn run_with(server: Arc<Server>, input: &str) -> Vec<Value> {
    let out = Out::default();
    serve_lines(
        server,
        Cursor::new(input.as_bytes().to_vec()),
        out.clone(),
        StdioConfig::default(),
    )
    .unwrap();
    out.messages()
}

/// The reply to request `id`.
pub fn reply(out: &[Value], id: i64) -> &Value {
    out.iter()
        .find(|m| m.get("id").and_then(Value::as_i64) == Some(id))
        .unwrap_or_else(|| panic!("no reply to {id} in {out:?}"))
}

/// The JSON-RPC error code of an error reply.
pub fn code(m: &Value) -> i32 {
    let c = m
        .get("error")
        .and_then(|e| e.get("code"))
        .and_then(Value::as_i64)
        .unwrap_or_else(|| panic!("not an error: {m:?}"));
    i32::try_from(c).expect("a 32-bit code")
}

/// The `result` of a successful reply.
pub fn result(m: &Value) -> &Value {
    m.get("result")
        .unwrap_or_else(|| panic!("not a result: {m:?}"))
}

/// A reader fed from a channel, so a test can send a line after the server
/// has started working on an earlier one.
pub struct Feed(pub Receiver<Vec<u8>>, pub Vec<u8>);

impl Read for Feed {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.1.is_empty() {
            match self.0.recv() {
                Ok(chunk) => self.1 = chunk,
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.1.len());
        buf[..n].copy_from_slice(&self.1[..n]);
        self.1.drain(..n);
        Ok(n)
    }
}
