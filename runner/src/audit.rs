//! The runner's audit log: one JSON object per line, append-only, each entry chained to the one
//! before by a SHA-256 hash. Editing, removing or reordering any entry breaks the chain from that
//! point on, which `verify` reports.
//!
//! The chain shows that entries were changed; it can't stop someone with access to the file from
//! rewriting the whole log. The latest entry's hash (the "fingerprint") can be written down
//! elsewhere, such as in the request's paper or email trail, to make the log up to that point
//! provable.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";
pub const FILE_NAME: &str = "audit-log.jsonl";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub seq: u64,
    /// UTC, RFC 3339.
    pub time: String,
    /// Who: the signed-in admin's username, or the computer account before sign-in.
    pub actor: String,
    pub event: String,
    /// Event details. Keys come out sorted, so the hash input is stable.
    pub details: Value,
    pub prev: String,
    pub hash: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn entry_hash(
    seq: u64,
    time: &str,
    actor: &str,
    event: &str,
    details: &Value,
    prev: &str,
) -> String {
    // Each field on its own line, JSON-encoded so no value can imitate a separator.
    let input = format!(
        "{seq}\n{}\n{}\n{}\n{}\n{prev}",
        Value::from(time),
        Value::from(actor),
        Value::from(event),
        details
    );
    sha256_hex(input.as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    pub entries: usize,
    /// The first line that doesn't fit the chain (1-based), and why.
    pub broken: Option<(usize, String)>,
    /// The last good entry's hash.
    pub fingerprint: String,
}

impl Verification {
    pub fn ok(&self) -> bool {
        self.broken.is_none()
    }
}

pub struct AuditLog {
    path: PathBuf,
    /// The last entry's (seq, hash), read once and then kept current.
    tail: Option<(u64, String)>,
}

impl AuditLog {
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(dir)?;
        Ok(Self {
            path: dir.join(FILE_NAME),
            tail: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lines(&self) -> Vec<String> {
        match fs::File::open(&self.path) {
            Ok(f) => BufReader::new(f)
                .lines()
                .map_while(Result::ok)
                .filter(|l| !l.trim().is_empty())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn last(&mut self) -> (u64, String) {
        if let Some(t) = &self.tail {
            return t.clone();
        }
        let t = self
            .lines()
            .last()
            .and_then(|l| serde_json::from_str::<Entry>(l).ok())
            .map(|e| (e.seq, e.hash))
            .unwrap_or((0, GENESIS.to_owned()));
        self.tail = Some(t.clone());
        t
    }

    /// Add an entry and write it to disk before returning.
    pub fn append(&mut self, actor: &str, event: &str, details: Value) -> std::io::Result<Entry> {
        let (last_seq, prev) = self.last();
        let seq = last_seq + 1;
        let time = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let hash = entry_hash(seq, &time, actor, event, &details, &prev);
        let entry = Entry {
            seq,
            time,
            actor: actor.to_owned(),
            event: event.to_owned(),
            details,
            prev,
            hash,
        };
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(
            f,
            "{}",
            serde_json::to_string(&entry).map_err(std::io::Error::other)?
        )?;
        f.sync_data()?;
        self.tail = Some((seq, entry.hash.clone()));
        Ok(entry)
    }

    /// Every entry that can be read, oldest first.
    pub fn entries(&self) -> Vec<Entry> {
        self.lines()
            .iter()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    /// Walk the chain from the start.
    pub fn verify(&self) -> Verification {
        let mut prev = GENESIS.to_owned();
        let lines = self.lines();
        for (expected_seq, (i, line)) in (1u64..).zip(lines.iter().enumerate()) {
            let n = i + 1;
            let bad = |why: &str| Verification {
                entries: lines.len(),
                broken: Some((n, why.to_owned())),
                fingerprint: prev.clone(),
            };
            let Ok(e) = serde_json::from_str::<Entry>(line) else {
                return bad("the line isn't a log entry");
            };
            if e.seq != expected_seq {
                return bad("an entry is missing or out of order");
            }
            if e.prev != prev {
                return bad("it doesn't follow on from the entry before it");
            }
            if entry_hash(e.seq, &e.time, &e.actor, &e.event, &e.details, &e.prev) != e.hash {
                return bad("it was changed after it was written");
            }
            prev = e.hash;
        }
        Verification {
            entries: lines.len(),
            broken: None,
            fingerprint: prev,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("keyql-audit-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn sample(dir: &Path) -> AuditLog {
        let mut log = AuditLog::open(dir).unwrap();
        log.append("pc\\it", "runner_started", json!({"version": "0.1.0"}))
            .unwrap();
        log.append(
            "pat@contoso.gov",
            "signed_in",
            json!({"tenant": "t-1", "name": "Pat"}),
        )
        .unwrap();
        log.append(
            "pat@contoso.gov",
            "search_created",
            json!({"query": "(\"budget\") AND kind:email", "case": "c-1"}),
        )
        .unwrap();
        log
    }

    #[test]
    fn chain_verifies_and_survives_reopening() {
        let d = temp_dir("ok");
        let log = sample(&d);
        let v = log.verify();
        assert!(v.ok(), "{v:?}");
        assert_eq!(v.entries, 3);
        assert_eq!(v.fingerprint, log.entries()[2].hash);
        let mut again = AuditLog::open(&d).unwrap();
        let e = again
            .append("pat@contoso.gov", "signed_out", json!({}))
            .unwrap();
        assert_eq!(e.seq, 4);
        assert_eq!(e.prev, v.fingerprint);
        assert!(again.verify().ok());
        let _ = fs::remove_dir_all(&d);
    }

    fn tamper(name: &str, change: impl Fn(&mut Vec<String>)) -> Verification {
        let d = temp_dir(name);
        let log = sample(&d);
        let mut lines: Vec<String> = fs::read_to_string(log.path())
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        change(&mut lines);
        fs::write(log.path(), lines.join("\n") + "\n").unwrap();
        let v = log.verify();
        let _ = fs::remove_dir_all(&d);
        v
    }

    #[test]
    fn edits_deletions_and_reordering_are_caught() {
        let v = tamper("edit", |l| l[1] = l[1].replace("Pat", "Someone else"));
        assert_eq!(
            v.broken,
            Some((2, "it was changed after it was written".into()))
        );
        let v = tamper("actor", |l| {
            l[2] = l[2].replace("pat@contoso.gov", "other@contoso.gov")
        });
        assert_eq!(v.broken.unwrap().0, 3);
        let v = tamper("delete", |l| {
            l.remove(1);
        });
        assert_eq!(
            v.broken,
            Some((2, "an entry is missing or out of order".into()))
        );
        let v = tamper("swap", |l| l.swap(1, 2));
        assert_eq!(v.broken.unwrap().0, 2);
        let v = tamper("junk", |l| l.push("not json".into()));
        assert_eq!(v.broken, Some((4, "the line isn't a log entry".into())));
        // Dropping the newest entry leaves a valid shorter chain: only a noted fingerprint shows it.
        let v = tamper("truncate", |l| {
            l.pop();
        });
        assert!(v.ok());
        assert_eq!(v.entries, 2);
    }

    #[test]
    fn hash_input_cannot_be_shifted_between_fields() {
        let a = entry_hash(1, "t", "ab", "c", &json!({}), GENESIS);
        let b = entry_hash(1, "t", "a", "bc", &json!({}), GENESIS);
        assert_ne!(a, b);
    }
}
