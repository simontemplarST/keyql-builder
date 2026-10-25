//! The copy of the audit log kept in Microsoft 365: one JSON file per entry in a SharePoint
//! document library, in a folder for this runner's log. Each file is the entry's exact line from
//! the local log, so the copy can be checked on its own, and an existing file is never replaced.
//! A retention label on the folder or library keeps the files for the county's retention period.

use std::collections::HashSet;

use crate::audit::Entry;
use crate::graph::{Graph, GraphError};

pub const DEFAULT_FOLDER: &str = "eDiscovery Search Runner audit log";

/// `00000042-search_created.json`: sorts in log order and says what each entry is.
pub fn file_name(e: &Entry) -> String {
    format!("{:08}-{}.json", e.seq, e.event)
}

/// The folder for one runner's log inside the chosen folder.
pub fn folder_for(base: &str, log_id: &str) -> String {
    let base = base.trim().trim_matches('/');
    let base = if base.is_empty() {
        DEFAULT_FOLDER
    } else {
        base
    };
    format!("{base}/log-{log_id}")
}

/// What's known about the SharePoint copy between syncs.
#[derive(Clone, Debug, Default)]
pub struct Known {
    pub site_id: String,
    pub folder: String,
    pub names: HashSet<String>,
}

/// Copy every entry that isn't in SharePoint yet, oldest first. Lists the folder the first time
/// (when `known` is empty), then relies on what it uploaded. Returns how many were uploaded.
pub fn sync(
    g: &Graph,
    site_url: &str,
    folder: &str,
    entries: &[(Entry, String)],
    known: &mut Known,
) -> Result<usize, GraphError> {
    if known.site_id.is_empty() || known.folder != folder {
        let site_id = g.site_id(site_url)?;
        g.ensure_folder(&site_id, folder)?;
        known.names = g.file_names(&site_id, folder)?.into_iter().collect();
        known.site_id = site_id;
        known.folder = folder.to_owned();
    }
    let mut uploaded = 0;
    for (e, line) in entries {
        let name = file_name(e);
        if known.names.contains(&name) {
            continue;
        }
        if g.upload_new(&known.site_id, &format!("{folder}/{name}"), line.as_bytes())? {
            uploaded += 1;
        }
        known.names.insert(name);
    }
    Ok(uploaded)
}

/// How many entries aren't in SharePoint yet, as far as is known.
pub fn pending(entries: &[(Entry, String)], known: &Known) -> usize {
    entries
        .iter()
        .filter(|(e, _)| !known.names.contains(&file_name(e)))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::AuditLog;
    use crate::graph::tests::fake_graph;
    use serde_json::{Value, json};

    #[test]
    fn copies_only_new_entries_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("keyql-mirror-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut log = AuditLog::open(&dir).unwrap();
        for ev in ["runner_started", "signed_in", "search_created"] {
            log.append("pat@contoso.gov", ev, json!({"n": ev})).unwrap();
        }
        let fake = fake_graph(|method, path, _body, _| match (method, path) {
            ("GET", "/v1.0/sites/contoso.sharepoint.com:/sites/eDiscovery") => {
                (200, vec![], json!({"id": "site-1"}))
            }
            // The top folder exists already; the log's folder is new.
            ("POST", "/v1.0/sites/site-1/drive/root/children") => {
                (409, vec![], json!({"error": {"code": "nameAlreadyExists"}}))
            }
            ("POST", p) if p.ends_with("/children") => (201, vec![], json!({"id": "f"})),
            ("GET", p) if p.contains("children?$select=name") => (
                200,
                vec![],
                json!({"value": [{"name": "00000001-runner_started.json"}]}),
            ),
            // Someone else's copy of entry 2 is already there: it must not be replaced.
            ("PUT", p) if p.contains("00000002") => {
                (409, vec![], json!({"error": {"code": "nameAlreadyExists"}}))
            }
            ("PUT", _) => (201, vec![], json!({"id": "file"})),
            _ => (
                404,
                vec![],
                json!({"error": {"code": "NotFound", "message": path}}),
            ),
        });
        let agent = crate::graph::agent();
        let g = Graph {
            agent: &agent,
            base: format!("{}/v1.0", fake.base),
            token: "t".into(),
        };
        let folder = folder_for("", "abc");
        assert_eq!(folder, "eDiscovery Search Runner audit log/log-abc");
        let mut known = Known::default();
        let entries = log.raw_entries();
        assert_eq!(pending(&entries, &known), 3);
        let n = sync(
            &g,
            "https://contoso.sharepoint.com/sites/eDiscovery/",
            &folder,
            &entries,
            &mut known,
        )
        .unwrap();
        assert_eq!(
            n, 1,
            "entry 1 was listed, entry 2 already existed, entry 3 is new"
        );
        assert_eq!(pending(&entries, &known), 0);

        let seen = fake.seen.lock().unwrap();
        let puts: Vec<&(String, String, Value, String)> =
            seen.iter().filter(|r| r.0 == "PUT").collect();
        assert_eq!(puts.len(), 2);
        assert!(puts[1].1.ends_with(
            "/drive/root:/eDiscovery%20Search%20Runner%20audit%20log/log-abc/00000003-search_created.json:/content?@microsoft.graph.conflictBehavior=fail"
        ));
        // The file is the entry's exact line.
        assert_eq!(
            puts[1].2,
            serde_json::from_str::<Value>(&entries[2].1).unwrap()
        );
        // A second sync uploads nothing and doesn't list the folder again.
        drop(seen);
        let before = fake.seen.lock().unwrap().len();
        assert_eq!(
            sync(
                &g,
                "https://contoso.sharepoint.com/sites/eDiscovery",
                &folder,
                &entries,
                &mut known
            )
            .unwrap(),
            0
        );
        assert_eq!(fake.seen.lock().unwrap().len(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
