//! A std-only fake "dev server" for the hosted-app supervisor e2e: binds
//! `127.0.0.1:<last-arg-port>` and answers every connection with HTTP 200, forever.
//! Stands in for `vite`/`next dev` so the supervisor lifecycle test needs no Node/npm.
//!
//! Optional modes (flags BEFORE the trailing port, mirroring a real dev command):
//! - `--fork-child`: spawn a copy of itself in `sleep-forever` mode and write the
//!   child's pid to `<cwd>/child.pid` — the GRANDCHILD shape (`npm run dev` →
//!   vite) the group-kill test needs. The child inherits the parent's process
//!   group, exactly like a real dev server's workers.
//! - `--dump-env <file>`: write the COMPLETE environment (`KEY=VALUE` lines) to
//!   `<cwd>/<file>` before serving — the env-hygiene witness.
//! - `sleep-forever` (first arg): park forever (the forked child's job).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::TcpListener;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("sleep-forever") {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--dump-env") {
        let file = args.get(i + 1).expect("--dump-env <file>");
        let mut out = String::new();
        for (k, v) in std::env::vars() {
            out.push_str(&k);
            out.push('=');
            out.push_str(&v);
            out.push('\n');
        }
        std::fs::write(file, out).expect("write the env dump");
    }
    if args.iter().any(|a| a == "--fork-child") {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("sleep-forever")
            .spawn()
            .expect("fork the sleeper child");
        std::fs::write("child.pid", child.id().to_string()).expect("write child.pid");
        // Deliberately never reaped: the child outliving the parent's own SIGKILL
        // is exactly the defect the group-kill test proves fixed.
        std::mem::forget(child);
    }
    // The supervisor appends the allocated port as the final argument.
    let port: u16 = args
        .last()
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
