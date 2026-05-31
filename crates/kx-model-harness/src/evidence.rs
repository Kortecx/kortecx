//! Evidence capture: a runstamp-keyed artifact tree under
//! `target/agentic-validation/<runstamp>/` plus small hex/formatting helpers.
//!
//! The runstamp is passed IN (never `Date::now()`), keeping the deterministic
//! paths deterministic.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A handle to one campaign run's artifact tree.
#[derive(Debug, Clone)]
pub struct Evidence {
    root: PathBuf,
}

impl Evidence {
    /// Create (or reuse) `<base>/agentic-validation/<runstamp>/`.
    pub fn open(base: &Path, runstamp: &str) -> io::Result<Self> {
        let root = base.join("agentic-validation").join(runstamp);
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// The run's artifact root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Write `bytes` to `<root>/<row>/<name>`, returning the path.
    pub fn write(&self, row: &str, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
        let dir = self.root.join(row);
        fs::create_dir_all(&dir)?;
        let path = dir.join(name);
        fs::write(&path, bytes)?;
        Ok(path)
    }

    /// Write a string artifact to `<root>/<row>/<name>`.
    pub fn write_str(&self, row: &str, name: &str, s: &str) -> io::Result<PathBuf> {
        self.write(row, name, s.as_bytes())
    }

    /// Append a line to the run's top-level `EVIDENCE.md`.
    pub fn append_evidence_md(&self, line: &str) -> io::Result<()> {
        use std::io::Write;
        let path = self.root.join("EVIDENCE.md");
        let mut f = fs::OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(f, "{line}")
    }
}

/// Lowercase hex of arbitrary bytes (for digests / refs / tokens in artifacts).
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}
