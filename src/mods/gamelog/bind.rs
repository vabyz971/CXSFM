//! Gamelog tick-side file tail: offset-following reader.
//!
//! Runs on the framework tick thread (~1 Hz poll). Plain file IO,
//! zero game interaction: this tool cannot crash the game by
//! construction.

use super::model::{MAX_BYTES_PER_POLL, MAX_LINES};
use super::GameLogMod;
use crate::mods::common::lock;
use std::io::{Read, Seek, SeekFrom};

impl GameLogMod {
    /// Read new bytes (tick thread). Rotation/truncation rewinds.
    /// Slow polls (>200 ms) log a tripwire: the present thread clones
    /// under the same lock, so a slow tick visibly stalls frames.
    pub(crate) fn poll(&self) {
        let t0 = std::time::Instant::now();
        let path = lock(&self.path).clone();
        let path = match path {
            Some(p) => p,
            None => return,
        };
        let mut file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => return,
        };
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mut offset = lock(&self.offset);
        if len < *offset {
            // Rotated (Player.log → Player-prev.log): start over.
            *offset = 0;
            lock(&self.pending).clear();
        }
        if len == *offset {
            return;
        }
        if file.seek(SeekFrom::Start(*offset)).is_err() {
            return;
        }
        let take = (len - *offset).min(MAX_BYTES_PER_POLL);
        let mut buf = vec![0u8; take as usize];
        let n = match file.read(&mut buf) {
            Ok(n) => n,
            Err(_) => return,
        };
        *offset += n as u64;
        drop(offset);
        buf.truncate(n);
        let mut pending = lock(&self.pending);
        pending.push_str(&String::from_utf8_lossy(&buf));
        // Drain complete lines, keep the tail.
        let mut lines = lock(&self.lines);
        while let Some(pos) = pending.find('\n') {
            let line: String = pending.drain(..=pos).collect();
            let line = line.trim_end_matches(['\n', '\r']).to_string();
            if !line.is_empty() {
                lines.push(line);
            }
        }
        if lines.len() > MAX_LINES {
            let drop_n = lines.len() - MAX_LINES;
            lines.drain(..drop_n);
        }
        drop(lines);
        drop(pending);
        let ms = t0.elapsed().as_millis();
        if ms > 200 {
            crate::log_line(&format!("gamelog: slow poll ({ms} ms)"));
        }
    }
}
