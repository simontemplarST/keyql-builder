//! The Microsoft Graph eDiscovery API calls the runner makes, as the signed-in admin:
//! list or create a case, create a search with the request's KeyQL query, add the mailboxes and
//! sites, then run Purview's estimate and wait for it.
//!
//! API reference: <https://learn.microsoft.com/graph/api/resources/security-ediscoverycase>.
//! Needs the delegated permission eDiscovery.ReadWrite.All, and the admin needs the eDiscovery
//! Manager or eDiscovery Administrator role in Microsoft Purview.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::plan::{SourceKind, SourcePlan};

pub const GRAPH: &str = "https://graph.microsoft.com/v1.0";

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(60)))
        .user_agent(concat!("keyql-runner/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphError {
    pub status: u16,
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let plain = match self.status {
            0 => "Couldn't reach Microsoft 365. Check the internet connection.".to_owned(),
            401 => "The sign-in has expired or isn't valid. Sign in again.".to_owned(),
            403 => "Your account isn't allowed to do this. It needs the eDiscovery Manager or \
                    eDiscovery Administrator role in Microsoft Purview, and the app needs the \
                    eDiscovery.ReadWrite.All permission with admin consent."
                .to_owned(),
            404 => {
                "Purview couldn't find that case or search. It may have been deleted.".to_owned()
            }
            429 | 503 => "Microsoft 365 is busy. Wait a minute and try again.".to_owned(),
            _ => "Purview refused the request.".to_owned(),
        };
        if self.message.is_empty() {
            write!(f, "{plain} (HTTP {})", self.status)
        } else {
            write!(
                f,
                "{plain} Purview said: {} (HTTP {} {})",
                self.message, self.status, self.code
            )
        }
    }
}

fn graph_error(status: u16, body: &str) -> GraphError {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    GraphError {
        status,
        code: v["error"]["code"].as_str().unwrap_or("").to_owned(),
        message: v["error"]["message"].as_str().unwrap_or("").to_owned(),
    }
}

fn transport(e: ureq::Error) -> GraphError {
    GraphError {
        status: 0,
        code: "transport".into(),
        message: e.to_string(),
    }
}

#[derive(Clone, Copy)]
enum Payload<'a> {
    Json(&'a Value),
    Bytes(&'a [u8], &'a str),
}

pub struct Reply {
    pub status: u16,
    pub location: Option<String>,
    pub body: Value,
}

pub struct Graph<'a> {
    pub agent: &'a ureq::Agent,
    pub base: String,
    pub token: String,
}

impl Graph<'_> {
    fn url(&self, path: &str) -> String {
        if path.starts_with("http") {
            path.to_owned()
        } else {
            format!("{}{path}", self.base)
        }
    }

    /// One call, retried a few times when Graph is throttling (429) or briefly unavailable (503).
    fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Reply, GraphError> {
        self.send(method, path, body.map(Payload::Json))
    }

    fn send(&self, method: &str, path: &str, body: Option<Payload>) -> Result<Reply, GraphError> {
        let url = self.url(path);
        let auth = format!("Bearer {}", self.token);
        for attempt in 0..4 {
            let res = match (method, body) {
                ("GET", _) => self.agent.get(&url).header("Authorization", &auth).call(),
                ("PUT", Some(Payload::Bytes(bytes, content_type))) => self
                    .agent
                    .put(&url)
                    .header("Authorization", &auth)
                    .header("Content-Type", content_type)
                    .send(bytes),
                (_, Some(Payload::Json(b))) => self
                    .agent
                    .post(&url)
                    .header("Authorization", &auth)
                    .send_json(b),
                _ => self
                    .agent
                    .post(&url)
                    .header("Authorization", &auth)
                    .send_json(json!({})),
            };
            let mut res = res.map_err(transport)?;
            let status = res.status().as_u16();
            let location = res
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let retry_after = res
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            let text = res.body_mut().read_to_string().unwrap_or_default();
            if (status == 429 || status == 503) && attempt < 3 {
                std::thread::sleep(Duration::from_secs(retry_after.unwrap_or(5).min(30)));
                continue;
            }
            if !(200..300).contains(&status) {
                return Err(graph_error(status, &text));
            }
            let body = if text.trim().is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&text).unwrap_or(Value::Null)
            };
            return Ok(Reply {
                status,
                location,
                body,
            });
        }
        unreachable!("the last attempt always returns")
    }

    pub fn me(&self) -> Result<Me, GraphError> {
        let v = self
            .call("GET", "/me?$select=id,displayName,userPrincipalName", None)?
            .body;
        Ok(Me {
            id: s(&v["id"]),
            name: s(&v["displayName"]),
            upn: s(&v["userPrincipalName"]),
        })
    }

    pub fn list_cases(&self) -> Result<Vec<Case>, GraphError> {
        let mut out = Vec::new();
        let mut next = Some(
            "/security/cases/ediscoveryCases?$select=id,displayName,status,externalId,createdDateTime"
                .to_owned(),
        );
        while let Some(path) = next.take() {
            let v = self.call("GET", &path, None)?.body;
            out.extend(
                v["value"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(Case::from_json),
            );
            next = v["@odata.nextLink"].as_str().map(str::to_owned);
            if out.len() > 2000 {
                break;
            }
        }
        out.sort_by(|a, b| b.created.cmp(&a.created));
        Ok(out)
    }

    pub fn create_case(
        &self,
        name: &str,
        description: &str,
        external_id: &str,
    ) -> Result<Case, GraphError> {
        let mut body = json!({ "displayName": name, "description": description });
        if !external_id.is_empty() {
            body["externalId"] = json!(external_id);
        }
        let v = self
            .call("POST", "/security/cases/ediscoveryCases", Some(&body))?
            .body;
        Ok(Case::from_json(&v))
    }

    pub fn create_search(
        &self,
        case_id: &str,
        name: &str,
        description: &str,
        query: &str,
        scopes: Option<&str>,
    ) -> Result<String, GraphError> {
        let mut body = json!({
            "displayName": name,
            "description": description,
            "contentQuery": query,
        });
        if let Some(sc) = scopes {
            body["dataSourceScopes"] = json!(sc);
        }
        let v = self
            .call(
                "POST",
                &format!("/security/cases/ediscoveryCases/{case_id}/searches"),
                Some(&body),
            )?
            .body;
        Ok(s(&v["id"]))
    }

    pub fn add_source(
        &self,
        case_id: &str,
        search_id: &str,
        kind: SourceKind,
        value: &str,
    ) -> Result<(), GraphError> {
        let body = match kind {
            SourceKind::Mailbox => json!({
                "@odata.type": "microsoft.graph.security.userSource",
                "email": value,
            }),
            SourceKind::Site => json!({
                "@odata.type": "microsoft.graph.security.siteSource",
                "site": { "webUrl": value },
            }),
        };
        self.call(
            "POST",
            &format!(
                "/security/cases/ediscoveryCases/{case_id}/searches/{search_id}/additionalSources"
            ),
            Some(&body),
        )?;
        Ok(())
    }

    /// Start Purview's estimate. Returns the address of the operation to poll.
    pub fn start_estimate(&self, case_id: &str, search_id: &str) -> Result<String, GraphError> {
        let r = self.call(
            "POST",
            &format!(
                "/security/cases/ediscoveryCases/{case_id}/searches/{search_id}/estimateStatistics"
            ),
            Some(&json!({})),
        )?;
        r.location.ok_or_else(|| GraphError {
            status: r.status,
            code: "noLocation".into(),
            message: "Purview started the estimate but didn't say where to check on it.".into(),
        })
    }

    pub fn estimate(&self, operation: &str) -> Result<Estimate, GraphError> {
        Ok(Estimate::from_json(
            &self.call("GET", operation, None)?.body,
        ))
    }
}

// ---------------------------------------------------------------- Purview's audit log

/// One record from the Microsoft 365 unified audit log.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditRecord {
    pub id: String,
    pub time: String,
    pub record_type: String,
    pub operation: String,
    pub user: String,
    pub object_id: String,
    /// The whole record as Graph sent it, for matching and for showing details.
    pub raw: Value,
}

impl AuditRecord {
    fn from_json(v: &Value) -> Self {
        Self {
            id: s(&v["id"]),
            time: s(&v["createdDateTime"]),
            record_type: s(&v["auditLogRecordType"]),
            operation: s(&v["operation"]),
            user: s(&v["userPrincipalName"]),
            object_id: s(&v["objectId"]),
            raw: v.clone(),
        }
    }
}

/// eDiscovery activity in the unified audit log: eDiscovery (Standard) and (Premium).
pub const EDISCOVERY_RECORD_TYPES: [&str; 2] = ["discovery", "aeD"];

impl Graph<'_> {
    /// Ask Purview's audit log for eDiscovery activity between two times (RFC 3339), wait for the
    /// query to finish, and return its records. Needs AuditLogsQuery.Read.All.
    pub fn purview_audit(
        &self,
        start: &str,
        end: &str,
        poll: Duration,
        give_up: Duration,
    ) -> Result<Vec<AuditRecord>, GraphError> {
        let body = json!({
            "displayName": "eDiscovery Search Runner comparison",
            "filterStartDateTime": start,
            "filterEndDateTime": end,
            "recordTypeFilters": EDISCOVERY_RECORD_TYPES,
        });
        let q = self
            .call("POST", "/security/auditLog/queries", Some(&body))?
            .body;
        let id = s(&q["id"]);
        let started = Instant::now();
        loop {
            let v = self
                .call("GET", &format!("/security/auditLog/queries/{id}"), None)?
                .body;
            match v["status"].as_str().unwrap_or("") {
                "succeeded" => break,
                "failed" | "cancelled" => {
                    return Err(GraphError {
                        status: 0,
                        code: "auditQueryFailed".into(),
                        message: "Purview couldn't finish the audit log search. Try again later."
                            .into(),
                    });
                }
                _ if started.elapsed() > give_up => {
                    return Err(GraphError {
                        status: 0,
                        code: "timeout".into(),
                        message:
                            "Purview's audit log search is taking too long. Try a shorter period."
                                .into(),
                    });
                }
                _ => std::thread::sleep(poll),
            }
        }
        let mut out = Vec::new();
        let mut next = Some(format!("/security/auditLog/queries/{id}/records"));
        while let Some(path) = next.take() {
            let v = self.call("GET", &path, None)?.body;
            out.extend(
                v["value"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(AuditRecord::from_json),
            );
            next = v["@odata.nextLink"].as_str().map(str::to_owned);
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- SharePoint copy of the log

/// Percent-encode each part of a drive path, keeping the slashes.
pub fn encode_path(path: &str) -> String {
    path.split('/')
        .map(crate::auth::encode)
        .collect::<Vec<_>>()
        .join("/")
}

impl Graph<'_> {
    /// The site ID for `https://contoso.sharepoint.com/sites/Name`. Needs Sites.ReadWrite.All.
    pub fn site_id(&self, site_url: &str) -> Result<String, GraphError> {
        let rest = site_url.trim().trim_end_matches('/');
        let rest = rest.strip_prefix("https://").unwrap_or(rest);
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        let path = if path.is_empty() {
            format!("/sites/{host}")
        } else {
            format!("/sites/{host}:/{}", encode_path(path))
        };
        Ok(s(&self.call("GET", &path, None)?.body["id"]))
    }

    /// Make each folder of `folder` in the site's default document library, if it isn't there yet.
    pub fn ensure_folder(&self, site_id: &str, folder: &str) -> Result<(), GraphError> {
        let mut parent = String::new();
        for name in folder.split('/').filter(|p| !p.is_empty()) {
            let at = if parent.is_empty() {
                format!("/sites/{site_id}/drive/root/children")
            } else {
                format!(
                    "/sites/{site_id}/drive/root:/{}:/children",
                    encode_path(&parent)
                )
            };
            let body =
                json!({"name": name, "folder": {}, "@microsoft.graph.conflictBehavior": "fail"});
            match self.call("POST", &at, Some(&body)) {
                Ok(_) => {}
                Err(e) if e.status == 409 => {}
                Err(e) => return Err(e),
            }
            parent = if parent.is_empty() {
                name.to_owned()
            } else {
                format!("{parent}/{name}")
            };
        }
        Ok(())
    }

    /// The names of the files already in the folder.
    pub fn file_names(&self, site_id: &str, folder: &str) -> Result<Vec<String>, GraphError> {
        let mut out = Vec::new();
        let mut next = Some(format!(
            "/sites/{site_id}/drive/root:/{}:/children?$select=name&$top=999",
            encode_path(folder)
        ));
        while let Some(path) = next.take() {
            let v = self.call("GET", &path, None)?.body;
            out.extend(
                v["value"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|f| s(&f["name"])),
            );
            next = v["@odata.nextLink"].as_str().map(str::to_owned);
        }
        Ok(out)
    }

    /// Upload a new file. An existing file of the same name is never replaced: Ok(false) says it
    /// was already there.
    pub fn upload_new(&self, site_id: &str, path: &str, bytes: &[u8]) -> Result<bool, GraphError> {
        let url = format!(
            "/sites/{site_id}/drive/root:/{}:/content?@microsoft.graph.conflictBehavior=fail",
            encode_path(path)
        );
        match self.send("PUT", &url, Some(Payload::Bytes(bytes, "application/json"))) {
            Ok(_) => Ok(true),
            Err(e) if e.status == 409 => Ok(false),
            Err(e) => Err(e),
        }
    }
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_owned()
}

fn n(v: &Value) -> i64 {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|t| t.parse().ok()))
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Me {
    pub id: String,
    pub name: String,
    pub upn: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    pub id: String,
    pub name: String,
    pub status: String,
    pub external_id: String,
    pub created: String,
}

impl Case {
    fn from_json(v: &Value) -> Self {
        Self {
            id: s(&v["id"]),
            name: s(&v["displayName"]),
            status: s(&v["status"]),
            external_id: s(&v["externalId"]),
            created: s(&v["createdDateTime"]),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Estimate {
    pub status: String,
    pub percent: i64,
    pub items: i64,
    pub size: i64,
    pub unindexed_items: i64,
    pub unindexed_size: i64,
    pub mailboxes: i64,
    pub sites: i64,
    pub error: String,
}

impl Estimate {
    fn from_json(v: &Value) -> Self {
        Self {
            status: s(&v["status"]),
            percent: n(&v["percentProgress"]),
            items: n(&v["indexedItemCount"]),
            size: n(&v["indexedItemsSize"]),
            unindexed_items: n(&v["unindexedItemCount"]),
            unindexed_size: n(&v["unindexedItemsSize"]),
            mailboxes: n(&v["mailboxCount"]),
            sites: n(&v["siteCount"]),
            error: v["resultInfo"]["message"].as_str().unwrap_or("").to_owned(),
        }
    }

    pub fn finished(&self) -> bool {
        matches!(
            self.status.as_str(),
            "succeeded" | "partiallySucceeded" | "failed" | "submissionFailed"
        )
    }

    pub fn ok(&self) -> bool {
        matches!(self.status.as_str(), "succeeded" | "partiallySucceeded")
    }
}

/// "1.4 GB"
pub fn bytes(n: i64) -> String {
    let units = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = n.max(0) as f64;
    let mut u = 0;
    while v >= 1024.0 && u < units.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} bytes")
    } else {
        format!("{v:.1} {}", units[u])
    }
}

// ---------------------------------------------------------------- the whole run

/// Where the search goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaseTarget {
    Existing {
        id: String,
        name: String,
    },
    New {
        name: String,
        description: String,
        external_id: String,
    },
}

pub struct RunJob {
    pub case: CaseTarget,
    pub search_name: String,
    pub search_description: String,
    pub query: String,
    pub sources: SourcePlan,
}

/// What happened, step by step, for the screen and the audit log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    CaseCreated { id: String, name: String },
    CaseChosen { id: String, name: String },
    SearchCreated { id: String, name: String },
    SourceAdded { kind: SourceKind, value: String },
    EstimateStarted { operation: String },
    EstimateUpdate(Estimate),
}

#[derive(Debug, Clone)]
pub struct RunFailed {
    pub step: &'static str,
    pub error: GraphError,
}

/// Do it all. `poll` is how long to wait between estimate checks; `give_up` bounds the wait.
pub fn run(
    g: &Graph,
    job: &RunJob,
    poll: Duration,
    give_up: Duration,
    progress: &mut dyn FnMut(Progress),
) -> Result<Estimate, RunFailed> {
    let fail = |step: &'static str| move |error: GraphError| RunFailed { step, error };
    let case_id = match &job.case {
        CaseTarget::Existing { id, name } => {
            progress(Progress::CaseChosen {
                id: id.clone(),
                name: name.clone(),
            });
            id.clone()
        }
        CaseTarget::New {
            name,
            description,
            external_id,
        } => {
            let c = g
                .create_case(name, description, external_id)
                .map_err(fail("create the case"))?;
            progress(Progress::CaseCreated {
                id: c.id.clone(),
                name: c.name.clone(),
            });
            c.id
        }
    };
    let search_id = g
        .create_search(
            &case_id,
            &job.search_name,
            &job.search_description,
            &job.query,
            job.sources.scopes(),
        )
        .map_err(fail("create the search"))?;
    progress(Progress::SearchCreated {
        id: search_id.clone(),
        name: job.search_name.clone(),
    });
    for e in job.sources.active() {
        let value = e.value.trim();
        g.add_source(&case_id, &search_id, e.kind, value)
            .map_err(fail("add a mailbox or site"))?;
        progress(Progress::SourceAdded {
            kind: e.kind,
            value: value.to_owned(),
        });
    }
    let op = g
        .start_estimate(&case_id, &search_id)
        .map_err(fail("start the estimate"))?;
    progress(Progress::EstimateStarted {
        operation: op.clone(),
    });
    let started = Instant::now();
    loop {
        let est = g.estimate(&op).map_err(fail("check the estimate"))?;
        progress(Progress::EstimateUpdate(est.clone()));
        if est.finished() {
            return Ok(est);
        }
        if started.elapsed() > give_up {
            return Err(RunFailed {
                step: "wait for the estimate",
                error: GraphError {
                    status: 0,
                    code: "timeout".into(),
                    message: "The estimate is still running. It will finish in Purview; check the \
                              search there."
                        .into(),
                },
            });
        }
        std::thread::sleep(poll);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::plan::SourceEntry;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// One request the fake received: (method, path, JSON body, Authorization header).
    pub type Seen = (String, String, Value, String);

    /// A stand-in for graph.microsoft.com: answers each request from `route` and records it.
    pub struct FakeGraph {
        pub base: String,
        pub seen: Arc<Mutex<Vec<Seen>>>,
    }

    pub fn fake_graph(
        route: impl Fn(&str, &str, &Value, &str) -> (u16, Vec<(String, String)>, Value) + Send + 'static,
    ) -> FakeGraph {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let host = base.clone();
        std::thread::spawn(move || {
            for stream in l.incoming().flatten() {
                let mut r = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if r.read_line(&mut line).is_err() || line.is_empty() {
                    continue;
                }
                let mut parts = line.split_whitespace();
                let (method, path) = (
                    parts.next().unwrap_or("").to_owned(),
                    parts.next().unwrap_or("").to_owned(),
                );
                let (mut len, mut auth) = (0usize, String::new());
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h == "\r\n" || h.is_empty() {
                        break;
                    }
                    let (k, v) = h.split_once(':').unwrap_or((&h, ""));
                    match k.to_ascii_lowercase().as_str() {
                        "content-length" => len = v.trim().parse().unwrap_or(0),
                        "authorization" => auth = v.trim().to_owned(),
                        _ => {}
                    }
                }
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                log.lock().unwrap().push((
                    method.clone(),
                    path.clone(),
                    body.clone(),
                    auth.clone(),
                ));
                let (status, headers, reply) = route(&method, &path, &body, &host);
                let text = if reply.is_null() {
                    String::new()
                } else {
                    reply.to_string()
                };
                let mut w = stream;
                let mut head = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    text.len()
                );
                for (k, v) in headers {
                    head.push_str(&format!("{k}: {v}\r\n"));
                }
                let _ = write!(w, "{head}\r\n{text}");
            }
        });
        FakeGraph { base, seen }
    }

    fn sources() -> SourcePlan {
        let e = |kind, value: &str| SourceEntry {
            asked: value.into(),
            is_person: true,
            kind,
            value: value.into(),
            skip: false,
        };
        SourcePlan {
            everyone: false,
            entries: vec![
                e(SourceKind::Mailbox, "jane.doe@example.gov"),
                e(
                    SourceKind::Site,
                    "https://contoso.sharepoint.com/sites/Highway",
                ),
                SourceEntry {
                    skip: true,
                    ..e(SourceKind::Mailbox, "")
                },
            ],
        }
    }

    #[test]
    fn full_run_against_a_fake_graph() {
        let polls = Arc::new(Mutex::new(0));
        let p2 = polls.clone();
        let fake = fake_graph(move |method, path, _body, host| match (method, path) {
            ("POST", "/v1.0/security/cases/ediscoveryCases") => (
                201,
                vec![],
                json!({"id": "case-1", "displayName": "DR-1 Test", "status": "active"}),
            ),
            ("POST", "/v1.0/security/cases/ediscoveryCases/case-1/searches") => {
                (201, vec![], json!({"id": "search-1"}))
            }
            ("POST", p) if p.ends_with("/additionalSources") => (201, vec![], json!({"id": "src"})),
            ("POST", p) if p.ends_with("/estimateStatistics") => (
                202,
                vec![(
                    "Location".into(),
                    format!(
                        "{host}/v1.0/security/cases/ediscoveryCases('case-1')/operations('op-1')"
                    ),
                )],
                Value::Null,
            ),
            ("GET", p) if p.contains("operations") => {
                let mut n = p2.lock().unwrap();
                *n += 1;
                if *n < 3 {
                    (
                        200,
                        vec![],
                        json!({"status": "running", "percentProgress": 40}),
                    )
                } else {
                    (
                        200,
                        vec![],
                        json!({"status": "succeeded", "percentProgress": 100, "indexedItemCount": 1234,
                        "indexedItemsSize": 52428800, "unindexedItemCount": 3, "unindexedItemsSize": 1024,
                        "mailboxCount": 1, "siteCount": 1}),
                    )
                }
            }
            _ => (
                404,
                vec![],
                json!({"error": {"code": "NotFound", "message": "no route"}}),
            ),
        });
        let agent = agent();
        let g = Graph {
            agent: &agent,
            base: format!("{}/v1.0", fake.base),
            token: "tok".into(),
        };
        let job = RunJob {
            case: CaseTarget::New {
                name: "DR-1 Test".into(),
                description: "why".into(),
                external_id: "DR-1".into(),
            },
            search_name: "Test (2026-10-23)".into(),
            search_description: "why".into(),
            query: "(budget) AND kind:email".into(),
            sources: sources(),
        };
        let mut steps = Vec::new();
        let est = run(
            &g,
            &job,
            Duration::from_millis(10),
            Duration::from_secs(10),
            &mut |p| steps.push(p),
        )
        .unwrap();
        assert_eq!(est.items, 1234);
        assert_eq!(bytes(est.size), "50.0 MB");
        assert!(est.ok());

        let seen = fake.seen.lock().unwrap();
        assert!(seen.iter().all(|(_, _, _, auth)| auth == "Bearer tok"));
        assert_eq!(
            seen[0].2,
            json!({"displayName": "DR-1 Test", "description": "why", "externalId": "DR-1"})
        );
        assert_eq!(
            seen[1].2,
            json!({"displayName": "Test (2026-10-23)", "description": "why", "contentQuery": "(budget) AND kind:email"})
        );
        assert_eq!(
            seen[2].2,
            json!({"@odata.type": "microsoft.graph.security.userSource", "email": "jane.doe@example.gov"})
        );
        assert_eq!(
            seen[3].2,
            json!({"@odata.type": "microsoft.graph.security.siteSource", "site": {"webUrl": "https://contoso.sharepoint.com/sites/Highway"}})
        );
        assert_eq!(
            seen.len(),
            4 + 1 + 3,
            "case, search, 2 sources (skipped one left out), estimate, 3 polls"
        );
        assert!(matches!(steps[0], Progress::CaseCreated { .. }));
        assert!(
            matches!(steps.last(), Some(Progress::EstimateUpdate(e)) if e.status == "succeeded")
        );
    }

    #[test]
    fn errors_are_explained() {
        let fake = fake_graph(|_, _, _, _| {
            (
                403,
                vec![],
                json!({"error": {"code": "Forbidden", "message": "User is not authorized."}}),
            )
        });
        let agent = agent();
        let g = Graph {
            agent: &agent,
            base: format!("{}/v1.0", fake.base),
            token: "t".into(),
        };
        let job = RunJob {
            case: CaseTarget::Existing {
                id: "c".into(),
                name: "Case".into(),
            },
            search_name: "s".into(),
            search_description: String::new(),
            query: "q".into(),
            sources: SourcePlan {
                everyone: true,
                entries: vec![],
            },
        };
        let err = run(&g, &job, Duration::ZERO, Duration::ZERO, &mut |_| {}).unwrap_err();
        assert_eq!(err.step, "create the search");
        assert_eq!(err.error.status, 403);
        let text = err.error.to_string();
        assert!(
            text.contains("eDiscovery Manager") && text.contains("User is not authorized."),
            "{text}"
        );
        // "Search everyone" sends the tenant-wide scopes.
        assert_eq!(
            fake.seen.lock().unwrap()[0].2["dataSourceScopes"],
            json!("allTenantMailboxes,allTenantSites")
        );
    }

    #[test]
    fn purview_audit_query_waits_and_pages() {
        let polls = Arc::new(Mutex::new(0));
        let p2 = polls.clone();
        let fake = fake_graph(move |method, path, _, host| match (method, path) {
            ("POST", "/v1.0/security/auditLog/queries") => {
                (201, vec![], json!({"id": "q1", "status": "notStarted"}))
            }
            ("GET", "/v1.0/security/auditLog/queries/q1") => {
                let mut n = p2.lock().unwrap();
                *n += 1;
                (
                    200,
                    vec![],
                    json!({"id": "q1", "status": if *n < 2 { "running" } else { "succeeded" }}),
                )
            }
            ("GET", "/v1.0/security/auditLog/queries/q1/records") => (
                200,
                vec![],
                json!({
                "value": [{"id": "a", "operation": "SearchCreated", "auditLogRecordType": "discovery",
                           "userPrincipalName": "pat@contoso.gov", "createdDateTime": "2026-10-25T10:00:00Z"}],
                "@odata.nextLink": format!("{host}/v1.0/security/auditLog/queries/q1/records?page=2")}),
            ),
            ("GET", p) if p.contains("page=2") => (
                200,
                vec![],
                json!({"value": [{"id": "b", "operation": "SearchStarted", "auditLogRecordType": "aeD"}]}),
            ),
            _ => (
                404,
                vec![],
                json!({"error": {"code": "NotFound", "message": path}}),
            ),
        });
        let agent = agent();
        let g = Graph {
            agent: &agent,
            base: format!("{}/v1.0", fake.base),
            token: "t".into(),
        };
        let recs = g
            .purview_audit(
                "2026-10-18T00:00:00Z",
                "2026-10-25T23:59:59Z",
                Duration::from_millis(5),
                Duration::from_secs(5),
            )
            .unwrap();
        assert_eq!(
            recs.iter()
                .map(|r| r.operation.as_str())
                .collect::<Vec<_>>(),
            vec!["SearchCreated", "SearchStarted"]
        );
        assert_eq!(recs[0].user, "pat@contoso.gov");
        let seen = fake.seen.lock().unwrap();
        assert_eq!(seen[0].2["recordTypeFilters"], json!(["discovery", "aeD"]));
        assert_eq!(
            seen[0].2["filterStartDateTime"],
            json!("2026-10-18T00:00:00Z")
        );
    }

    #[test]
    fn cases_follow_next_links() {
        let fake = fake_graph(|_, path, _, host| {
            if path.contains("page2") {
                (
                    200,
                    vec![],
                    json!({"value": [{"id": "b", "displayName": "B", "createdDateTime": "2026-02-01T00:00:00Z"}]}),
                )
            } else {
                (
                    200,
                    vec![],
                    json!({"value": [{"id": "a", "displayName": "A", "createdDateTime": "2026-01-01T00:00:00Z"}],
                    "@odata.nextLink": format!("{host}/v1.0/security/cases/ediscoveryCases?page2")}),
                )
            }
        });
        let agent = agent();
        let g = Graph {
            agent: &agent,
            base: format!("{}/v1.0", fake.base),
            token: "t".into(),
        };
        let cases = g.list_cases().unwrap();
        assert_eq!(
            cases.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec!["b", "a"]
        );
    }
}
