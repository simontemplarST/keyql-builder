//! Comparing the runner's audit log with Microsoft Purview's own audit log, which records the
//! eDiscovery activity under the admin's account. Each search the runner created should show up
//! there; anything Purview recorded that the runner didn't do is listed separately.

use chrono::{DateTime, Duration, Utc};

use crate::audit::Entry;
use crate::graph::AuditRecord;

/// Purview can take a while to record activity; until then a run is "waiting", not "missing".
pub const GRACE: Duration = Duration::hours(24);

/// A search the runner created, from its audit log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub seq: u64,
    pub time: String,
    pub actor: String,
    pub case_id: String,
    pub case_name: String,
    pub search_id: String,
    pub search_name: String,
}

pub fn runs(entries: &[Entry]) -> Vec<Run> {
    let mut case_names: Vec<(String, String)> = Vec::new();
    let mut out = Vec::new();
    for e in entries {
        let d = &e.details;
        let text = |k: &str| d[k].as_str().unwrap_or("").to_owned();
        match e.event.as_str() {
            "case_created" | "case_selected" => case_names.push((text("case"), text("name"))),
            "search_created" => {
                let case_id = text("case");
                let case_name = case_names
                    .iter()
                    .rev()
                    .find(|(id, _)| *id == case_id)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_default();
                out.push(Run {
                    seq: e.seq,
                    time: e.time.clone(),
                    actor: e.actor.clone(),
                    case_id,
                    case_name,
                    search_id: text("search"),
                    search_name: text("name"),
                });
            }
            _ => {}
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Purview has records for it (indexes into the records).
    Confirmed(Vec<usize>),
    /// Not in Purview yet, but recent enough that it may still appear.
    Waiting,
    /// Older than a day and Purview has no record of it.
    Missing,
}

pub struct Comparison {
    pub runs: Vec<(Run, Status)>,
    /// Purview records that match none of the runner's searches: done some other way.
    pub outside: Vec<usize>,
}

impl Comparison {
    pub fn count(&self, f: impl Fn(&Status) -> bool) -> usize {
        self.runs.iter().filter(|(_, s)| f(s)).count()
    }
}

fn mentions(record: &AuditRecord, run: &Run) -> bool {
    let hay = record.raw.to_string().to_lowercase();
    [
        &run.search_id,
        &run.search_name,
        &run.case_id,
        &run.case_name,
    ]
    .iter()
    .filter(|n| n.chars().count() >= 6)
    .any(|n| hay.contains(&n.to_lowercase()))
}

pub fn compare(runs: Vec<Run>, records: &[AuditRecord], now: DateTime<Utc>) -> Comparison {
    let mut used = vec![false; records.len()];
    let runs = runs
        .into_iter()
        .map(|run| {
            let hits: Vec<usize> = records
                .iter()
                .enumerate()
                .filter(|(_, r)| mentions(r, &run))
                .map(|(i, _)| i)
                .collect();
            for &i in &hits {
                used[i] = true;
            }
            let status = if !hits.is_empty() {
                Status::Confirmed(hits)
            } else {
                let at = DateTime::parse_from_rfc3339(&run.time)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or(now);
                if now - at < GRACE {
                    Status::Waiting
                } else {
                    Status::Missing
                }
            };
            (run, status)
        })
        .collect();
    Comparison {
        runs,
        outside: (0..records.len()).filter(|&i| !used[i]).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(seq: u64, time: &str, event: &str, details: serde_json::Value) -> Entry {
        Entry {
            seq,
            time: time.into(),
            actor: "pat@contoso.gov".into(),
            event: event.into(),
            details,
            prev: String::new(),
            hash: String::new(),
        }
    }

    fn record(id: &str, op: &str, data: serde_json::Value) -> AuditRecord {
        AuditRecord {
            id: id.into(),
            time: "2026-10-20T10:05:00Z".into(),
            record_type: "discovery".into(),
            operation: op.into(),
            user: "pat@contoso.gov".into(),
            object_id: String::new(),
            raw: json!({"operation": op, "auditData": data}),
        }
    }

    #[test]
    fn runs_are_matched_waiting_or_missing() {
        let entries = vec![
            entry(
                1,
                "2026-10-20T10:00:00Z",
                "case_created",
                json!({"case": "case-1111", "name": "DR-041 Highway 56"}),
            ),
            entry(
                2,
                "2026-10-20T10:00:01Z",
                "search_created",
                json!({"case": "case-1111", "search": "search-aaaa", "name": "Highway 56 (2026-10-20)"}),
            ),
            entry(
                3,
                "2026-10-22T09:00:00Z",
                "case_selected",
                json!({"case": "case-2222", "name": "Smith v. County"}),
            ),
            entry(
                4,
                "2026-10-22T09:00:01Z",
                "search_created",
                json!({"case": "case-2222", "search": "search-bbbb", "name": "Old request (2026-10-22)"}),
            ),
            entry(
                5,
                "2026-10-25T08:00:00Z",
                "search_created",
                json!({"case": "case-3333", "search": "search-cccc", "name": "New request (2026-10-25)"}),
            ),
        ];
        let r = runs(&entries);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].case_name, "DR-041 Highway 56");
        let records = vec![
            record(
                "r1",
                "SearchCreated",
                json!({"ObjectId": "Highway 56 (2026-10-20)", "CaseId": "CASE-1111"}),
            ),
            record("r2", "CaseAdded", json!({"ObjectId": "DR-041 Highway 56"})),
            record(
                "r3",
                "SearchExported",
                json!({"ObjectId": "Something else entirely"}),
            ),
        ];
        let now = DateTime::parse_from_rfc3339("2026-10-25T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let c = compare(r, &records, now);
        assert_eq!(c.runs[0].1, Status::Confirmed(vec![0, 1]));
        assert_eq!(c.runs[1].1, Status::Missing);
        assert_eq!(c.runs[2].1, Status::Waiting);
        assert_eq!(c.outside, vec![2]);
        assert_eq!(c.count(|s| matches!(s, Status::Confirmed(_))), 1);
    }
}
