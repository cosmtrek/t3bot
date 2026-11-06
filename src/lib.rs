pub mod cli;
pub mod cmd;
pub mod config;
pub mod db;
pub mod sync;
pub mod t3;

use jiff::{Timestamp, Zoned};
use rusqlite::Connection;

/// Everything a command needs. Tests build one with an in-memory DB and a fake T3.
pub struct Ctx {
    pub db: Connection,
    pub t3: Box<dyn t3::T3>,
    pub cfg: config::Config,
    pub now: Zoned,
}

impl Ctx {
    pub fn now_ts(&self) -> String {
        ts(&self.now.timestamp())
    }
}

/// Timestamps are stored as RFC 3339 UTC strings.
pub fn ts(t: &Timestamp) -> String {
    t.to_string()
}

/// Parse "3 4 5", "3-5", "3,4" style id lists.
pub fn parse_ids(args: &[String]) -> anyhow::Result<Vec<i64>> {
    let mut out = Vec::new();
    for a in args {
        for part in a.split([',', '，', ' ']).filter(|s| !s.is_empty()) {
            let part = part.trim_start_matches('#');
            if let Some((a, b)) = part.split_once('-') {
                let (a, b): (i64, i64) = (a.trim_start_matches('#').parse()?, b.trim_start_matches('#').parse()?);
                anyhow::ensure!(a <= b && b - a < 100, "bad range {part}");
                out.extend(a..=b);
            } else {
                out.push(part.parse().map_err(|_| anyhow::anyhow!("bad id {part:?}"))?);
            }
        }
    }
    anyhow::ensure!(!out.is_empty(), "no ids given");
    out.dedup();
    Ok(out)
}
