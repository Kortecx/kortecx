//! A std-only fake "dev server" for the hosted-app supervisor e2e: binds
//! `127.0.0.1:<last-arg-port>` and answers every connection with HTTP 200, forever.
//! Stands in for `vite`/`next dev` so the supervisor lifecycle test needs no Node/npm.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::TcpListener;

fn main() {
    // The supervisor appends the allocated port as the final argument.
    let port: u16 = std::env::args()
        .next_back()
        .and_then(|a| a.parse().ok())
        .expect("a port as the final argument");
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind the fake dev-server port");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);
        let body = "<!doctype html><title>kortecx hosted fake</title>ok";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.flush();
    }
}
