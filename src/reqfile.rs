//! The `.req` request file: everything the person asking answered, the plain-English summary and
//! the exact KeyQL query, in one binary file for the IT Director. The app opens it read-only.
//!
//! Layout (integers little-endian):
//!
//! | offset  | size | contents                                         |
//! |---------|------|--------------------------------------------------|
//! | 0       | 8    | `KEYQLREQ`                                       |
//! | 8       | 2    | format version (1)                               |
//! | 10      | 4    | payload length N                                 |
//! | 14      | N    | payload: [`Contents`] encoded with postcard      |
//! | 14 + N  | 4    | CRC-32 (IEEE) of every byte before it            |
//!
//! A damaged or altered file fails the checksum and is refused. The file is not encrypted:
//! anyone with the app can read it.

use std::fmt;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::query::{self, OptionalHolidays, SearchRequest};

pub const MAGIC: &[u8; 8] = b"KEYQLREQ";
pub const VERSION: u16 = 1;
pub const EXTENSION: &str = "req";
const HEADER: usize = 8 + 2 + 4;

/// Everything in a request file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Contents {
    /// The app version that saved the file.
    pub app_version: String,
    pub saved_on: NaiveDate,
    /// The answers, so a later version can reopen the request for editing.
    pub request: SearchRequest,
    /// The office holidays the deadline was counted with.
    pub holidays: OptionalHolidays,
    /// The KeyQL query, exactly as built when the file was saved.
    pub query: String,
    /// The plain-English summary as shown when it was saved: (heading, lines).
    pub summary: Vec<(String, Vec<String>)>,
}

impl Contents {
    /// Fix the request as it stands today: the query and summary are built now, so relative
    /// dates ("Last 30 days") stay the same however late the file is opened.
    pub fn new(req: &SearchRequest, today: NaiveDate) -> Self {
        let summary = req
            .describe(today)
            .into_iter()
            .filter_map(|(step, lines)| query::section_title(step).map(|t| (t.to_owned(), lines)))
            .collect();
        Self {
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            saved_on: today,
            request: req.clone(),
            holidays: req.holidays,
            query: req.build_at(today).query,
            summary,
        }
    }

    pub fn name(&self) -> String {
        self.request.display_name(self.saved_on)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReadError {
    NotARequestFile,
    NewerVersion(u16),
    Damaged,
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotARequestFile => "This isn't an eDiscovery request file.",
            Self::NewerVersion(_) => {
                "This file was saved by a newer version of the app. Use the latest version to open it."
            }
            Self::Damaged => {
                "The file is damaged or was changed after it was saved, so it can't be trusted. \
                 Ask for it to be saved and sent again."
            }
        })
    }
}

pub fn encode(c: &Contents) -> Vec<u8> {
    let payload = postcard::to_allocvec(c).expect("request contents always encode");
    let mut out = Vec::with_capacity(HEADER + payload.len() + 4);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    let crc = crc32(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    out
}

pub fn decode(bytes: &[u8]) -> Result<Contents, ReadError> {
    if bytes.len() < MAGIC.len() || &bytes[..MAGIC.len()] != MAGIC {
        return Err(ReadError::NotARequestFile);
    }
    if bytes.len() < HEADER + 4 {
        return Err(ReadError::Damaged);
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version > VERSION {
        return Err(ReadError::NewerVersion(version));
    }
    let len = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    if bytes.len() != HEADER + len + 4 {
        return Err(ReadError::Damaged);
    }
    let (body, tail) = bytes.split_at(HEADER + len);
    if crc32(body) != u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]) {
        return Err(ReadError::Damaged);
    }
    let mut c: Contents = postcard::from_bytes(&body[HEADER..]).map_err(|_| ReadError::Damaged)?;
    c.request.holidays = c.holidays;
    Ok(c)
}

/// CRC-32 (IEEE 802.3), the checksum used by zip and PNG.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 23).unwrap()
    }

    fn sample() -> Vec<u8> {
        encode(&Contents::new(&SearchRequest::example(today()), today()))
    }

    #[test]
    fn crc_matches_the_standard() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn round_trip_keeps_everything() {
        let req = SearchRequest::example(today());
        let c = decode(&sample()).unwrap();
        assert_eq!(c.query, req.build_at(today()).query);
        assert_eq!(c.saved_on, today());
        assert_eq!(c.name(), "Example - Highway 56 contract");
        assert_eq!(c.request.custodians.people, req.custodians.people);
        assert_eq!(c.request.about.reference, "DR-2026-041");
        assert_eq!(c.summary[0].0, "Request");
        assert_eq!(c.summary.len(), 6);
        assert_eq!(c.app_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn layout() {
        let b = sample();
        assert_eq!(&b[..8], b"KEYQLREQ");
        assert_eq!(u16::from_le_bytes([b[8], b[9]]), 1);
        let len = u32::from_le_bytes([b[10], b[11], b[12], b[13]]) as usize;
        assert_eq!(b.len(), 14 + len + 4);
    }

    #[test]
    fn refuses_damaged_or_foreign_files() {
        let b = sample();
        // Any single changed byte after the marker is caught.
        for i in [8, 12, 20, b.len() / 2, b.len() - 1] {
            let mut bad = b.clone();
            bad[i] ^= 0x01;
            assert!(decode(&bad).is_err(), "byte {i}");
        }
        assert_eq!(decode(&b[..b.len() - 1]).err(), Some(ReadError::Damaged));
        assert_eq!(decode(b"KEYQLREQ").err(), Some(ReadError::Damaged));
        assert_eq!(
            decode(b"(\"budget\") AND kind:email").err(),
            Some(ReadError::NotARequestFile)
        );
        assert_eq!(decode(b"").err(), Some(ReadError::NotARequestFile));
        let mut newer = b.clone();
        newer[8] = 2;
        assert_eq!(decode(&newer).err(), Some(ReadError::NewerVersion(2)));
    }
}
