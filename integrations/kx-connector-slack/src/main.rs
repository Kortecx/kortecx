// SPDX-License-Identifier: Apache-2.0
//! The `kx-connector-slack` binary: a newline-delimited JSON-RPC 2.0 MCP server
//! over stdio. It builds its Slack client from the environment (the injected
//! bot-token credential, D81) once at start, then answers one request per input
//! line. The credential value is never written to stdout, stderr, or a log.

use std::io::{BufRead, Write};

use kx_connector_slack::{handle_line, SlackClient};

fn main() {
    let client = SlackClient::from_env();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = handle_line(&line, &client);
        if writeln!(out, "{reply}").is_err() || out.flush().is_err() {
            break;
        }
    }
}
