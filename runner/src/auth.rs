//! Signing an administrator in to Microsoft 365 (Microsoft Entra ID) from a desktop app.
//!
//! OAuth 2.0 authorization code flow with PKCE (RFC 7636) and a loopback redirect, as Microsoft
//! recommends for desktop apps: the admin signs in in their own browser (so MFA and Conditional
//! Access work as usual), the browser hands the code back to `http://localhost:<port>`, and the
//! app redeems it. The app is a public client: there is no client secret, and tokens are kept in
//! memory only, never written to disk.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const AUTHORITY: &str = "https://login.microsoftonline.com";
/// Delegated permissions: run eDiscovery as the signed-in admin, and read who they are.
/// Also read Purview's audit log (to compare), and write the log's copy to SharePoint.
pub const SCOPES: &str = "https://graph.microsoft.com/eDiscovery.ReadWrite.All \
                          https://graph.microsoft.com/AuditLogsQuery.Read.All \
                          https://graph.microsoft.com/Sites.ReadWrite.All \
                          https://graph.microsoft.com/User.Read offline_access openid profile";
/// How long to wait for the admin to finish signing in in the browser.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug)]
pub struct AuthConfig {
    /// Directory (tenant) ID or a verified domain, e.g. `contoso.onmicrosoft.com`.
    pub tenant: String,
    /// Application (client) ID of the app registration.
    pub client_id: String,
    /// `https://login.microsoftonline.com` (tests point this elsewhere).
    pub authority: String,
}

impl AuthConfig {
    pub fn new(tenant: &str, client_id: &str) -> Self {
        Self {
            tenant: tenant.trim().to_owned(),
            client_id: client_id.trim().to_owned(),
            authority: AUTHORITY.to_owned(),
        }
    }

    fn endpoint(&self, what: &str) -> String {
        format!("{}/{}/oauth2/v2.0/{what}", self.authority, self.tenant)
    }
}

#[derive(Clone)]
pub struct Tokens {
    pub access: String,
    pub refresh: Option<String>,
    pub expires_at: Instant,
    /// Claims from the ID token: who signed in, to which tenant.
    pub user: SignedInUser,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

impl Tokens {
    /// True when the access token runs out within `margin`.
    pub fn expiring(&self, margin: Duration) -> bool {
        Instant::now() + margin >= self.expires_at
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignedInUser {
    pub name: String,
    pub username: String,
    pub tenant_id: String,
    pub object_id: String,
}

// ---------------------------------------------------------------- PKCE and URLs

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the operating system's random number generator");
    URL_SAFE_NO_PAD.encode(buf)
}

pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn new_pkce() -> Pkce {
    let verifier = random_token(32);
    let challenge = challenge_for(&verifier);
    Pkce {
        verifier,
        challenge,
    }
}

/// Percent-encode a query-string value (RFC 3986 unreserved characters pass through).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn authorize_url(cfg: &AuthConfig, redirect_uri: &str, state: &str, challenge: &str) -> String {
    format!(
        "{}?client_id={}&response_type=code&redirect_uri={}&response_mode=query&scope={}\
         &state={}&code_challenge={}&code_challenge_method=S256&prompt=select_account",
        cfg.endpoint("authorize"),
        encode(&cfg.client_id),
        encode(redirect_uri),
        encode(SCOPES),
        encode(state),
        encode(challenge),
    )
}

/// The code from the browser's redirect (`GET /?code=…&state=… HTTP/1.1`), after checking the
/// state matches the one this sign-in sent.
pub fn code_from_request_line(line: &str, expected_state: &str) -> Result<String, String> {
    let target = line
        .split_whitespace()
        .nth(1)
        .ok_or("The sign-in reply was empty.")?;
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut description = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = decode(v);
        match k {
            "code" => code = Some(v),
            "state" => state = Some(v),
            "error" => error = Some(v),
            "error_description" => description = Some(v),
            _ => {}
        }
    }
    if let Some(e) = error {
        return Err(friendly_sign_in_error(
            &e,
            description.as_deref().unwrap_or(""),
        ));
    }
    if state.as_deref() != Some(expected_state) {
        return Err(
            "The sign-in reply didn't match this sign-in, so it was ignored. Try again.".into(),
        );
    }
    code.filter(|c| !c.is_empty())
        .ok_or_else(|| "The sign-in reply had no authorization code. Try again.".into())
}

fn friendly_sign_in_error(error: &str, description: &str) -> String {
    let first = description.lines().next().unwrap_or("").trim();
    match error {
        "access_denied" => "Sign-in was cancelled, or the permissions weren't granted.".into(),
        "consent_required" | "interaction_required" => format!(
            "An administrator needs to grant this app its permissions in Microsoft Entra ID first. ({first})"
        ),
        _ if first.is_empty() => format!("Sign-in failed ({error})."),
        _ => format!("Sign-in failed: {first}"),
    }
}

// ---------------------------------------------------------------- tokens

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    expires_in: u64,
    #[serde(default)]
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct TokenError {
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

#[derive(Deserialize, Default)]
struct IdClaims {
    #[serde(default)]
    name: String,
    #[serde(default)]
    preferred_username: String,
    #[serde(default)]
    tid: String,
    #[serde(default)]
    oid: String,
}

/// Read the claims from an ID token. The token came straight from the token endpoint over TLS,
/// so its signature isn't re-checked here; it's only used to say who signed in.
fn id_claims(id_token: &str) -> IdClaims {
    id_token
        .split('.')
        .nth(1)
        .and_then(|p| URL_SAFE_NO_PAD.decode(p.trim_end_matches('=')).ok())
        .and_then(|json| serde_json::from_slice(&json).ok())
        .unwrap_or_default()
}

fn token_request(
    agent: &ureq::Agent,
    cfg: &AuthConfig,
    form: &[(&str, &str)],
    previous: Option<&SignedInUser>,
) -> Result<Tokens, String> {
    let mut res = agent
        .post(&cfg.endpoint("token"))
        .send_form(form.iter().copied())
        .map_err(|e| format!("Couldn't reach Microsoft sign-in: {e}"))?;
    let status = res.status().as_u16();
    let body = res.body_mut().read_to_string().unwrap_or_default();
    if status != 200 {
        let e: TokenError = serde_json::from_str(&body).unwrap_or(TokenError {
            error: format!("HTTP {status}"),
            error_description: String::new(),
        });
        return Err(friendly_sign_in_error(&e.error, &e.error_description));
    }
    let t: TokenResponse = serde_json::from_str(&body)
        .map_err(|_| "Microsoft sign-in sent an unexpected reply.".to_owned())?;
    let user = match t.id_token.as_deref().map(id_claims) {
        Some(c) => SignedInUser {
            name: c.name,
            username: c.preferred_username,
            tenant_id: c.tid,
            object_id: c.oid,
        },
        None => previous.cloned().unwrap_or_default(),
    };
    Ok(Tokens {
        access: t.access_token,
        refresh: t.refresh_token,
        expires_at: Instant::now() + Duration::from_secs(t.expires_in.saturating_sub(60)),
        user,
    })
}

pub fn redeem_code(
    agent: &ureq::Agent,
    cfg: &AuthConfig,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<Tokens, String> {
    token_request(
        agent,
        cfg,
        &[
            ("client_id", &cfg.client_id),
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
            ("scope", SCOPES),
        ],
        None,
    )
}

/// A fresh access token from the refresh token, without asking the admin again.
pub fn refresh(agent: &ureq::Agent, cfg: &AuthConfig, tokens: &Tokens) -> Result<Tokens, String> {
    let rt = tokens
        .refresh
        .as_deref()
        .ok_or("The sign-in has expired. Sign in again.")?;
    let mut t = token_request(
        agent,
        cfg,
        &[
            ("client_id", &cfg.client_id),
            ("grant_type", "refresh_token"),
            ("refresh_token", rt),
            ("scope", SCOPES),
        ],
        Some(&tokens.user),
    )?;
    if t.refresh.is_none() {
        t.refresh = tokens.refresh.clone();
    }
    Ok(t)
}

// ---------------------------------------------------------------- the loopback sign-in

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Signed in</title>\
<body style=\"font:16px system-ui;margin:3em\"><h1>You're signed in</h1>\
<p>You can close this tab and go back to the eDiscovery Search Runner.</p>";
const FAIL_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Sign-in failed</title>\
<body style=\"font:16px system-ui;margin:3em\"><h1>Sign-in didn't finish</h1>\
<p>Go back to the eDiscovery Search Runner to see why and try again.</p>";

fn reply(mut stream: TcpStream, ok: bool) {
    let page = if ok { DONE_PAGE } else { FAIL_PAGE };
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
}

/// Sign the admin in: open the browser at Microsoft's sign-in page, wait for the redirect to
/// `http://localhost:<port>`, and redeem the code. Blocks; run it off the UI thread. `cancel`
/// stops the wait.
pub fn sign_in(
    agent: &ureq::Agent,
    cfg: &AuthConfig,
    open_browser: impl FnOnce(&str) -> Result<(), String>,
    cancel: Arc<AtomicBool>,
) -> Result<Tokens, String> {
    if cfg.tenant.is_empty() || cfg.client_id.is_empty() {
        return Err("Add the tenant ID and the app's client ID in Settings first.".into());
    }
    let v4 =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("Couldn't start sign-in: {e}"))?;
    let port = v4.local_addr().map_err(|e| e.to_string())?.port();
    // Browsers may try ::1 for "localhost" first; listen there too when the system has IPv6.
    let v6 = TcpListener::bind(("::1", port)).ok();
    let listeners: Vec<&TcpListener> = std::iter::once(&v4).chain(v6.as_ref()).collect();
    for l in &listeners {
        l.set_nonblocking(true).map_err(|e| e.to_string())?;
    }
    let redirect_uri = format!("http://localhost:{port}");
    let pkce = new_pkce();
    let state = random_token(16);
    open_browser(&authorize_url(cfg, &redirect_uri, &state, &pkce.challenge))?;

    let deadline = Instant::now() + SIGN_IN_TIMEOUT;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Sign-in was cancelled.".into());
        }
        if Instant::now() > deadline {
            return Err("Sign-in timed out. Try again.".into());
        }
        let accepted = listeners.iter().find_map(|l| l.accept().ok());
        let Some((stream, _)) = accepted else {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        if reader.read_line(&mut line).is_err() || !line.starts_with("GET /") {
            continue;
        }
        // Browsers also ask for /favicon.ico; only the redirect carries a query string.
        if !line.contains('?') {
            reply(stream, true);
            continue;
        }
        let result = code_from_request_line(&line, &state);
        reply(stream, result.is_ok());
        let code = result?;
        return redeem_code(agent, cfg, &code, &pkce.verifier, &redirect_uri);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc_7636_example() {
        // RFC 7636, Appendix B.
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let p = new_pkce();
        assert_eq!(p.verifier.len(), 43);
        assert_eq!(challenge_for(&p.verifier), p.challenge);
        assert_ne!(new_pkce().verifier, p.verifier);
    }

    #[test]
    fn authorize_url_is_well_formed() {
        let cfg = AuthConfig::new(
            " contoso.onmicrosoft.com ",
            "11111111-2222-3333-4444-555555555555",
        );
        let u = authorize_url(&cfg, "http://localhost:50123", "st", "ch");
        assert!(u.starts_with(
            "https://login.microsoftonline.com/contoso.onmicrosoft.com/oauth2/v2.0/authorize?"
        ));
        assert!(u.contains("client_id=11111111-2222-3333-4444-555555555555"));
        assert!(u.contains("redirect_uri=http%3A%2F%2Flocalhost%3A50123"));
        assert!(
            u.contains("scope=https%3A%2F%2Fgraph.microsoft.com%2FeDiscovery.ReadWrite.All%20")
        );
        assert!(u.contains("&code_challenge=ch&code_challenge_method=S256"));
        assert!(u.contains("&state=st&"));
    }

    #[test]
    fn redirect_parsing() {
        assert_eq!(
            code_from_request_line(
                "GET /?code=0.AbC%2Fd-e&state=xyz&session_state=1 HTTP/1.1",
                "xyz"
            ),
            Ok("0.AbC/d-e".to_owned())
        );
        assert!(code_from_request_line("GET /?code=abc&state=other HTTP/1.1", "xyz").is_err());
        let denied = code_from_request_line(
            "GET /?error=access_denied&error_description=AADSTS65004%3A+User+declined&state=xyz HTTP/1.1",
            "xyz",
        );
        assert_eq!(
            denied,
            Err("Sign-in was cancelled, or the permissions weren't granted.".to_owned())
        );
        assert!(code_from_request_line("GET / HTTP/1.1", "xyz").is_err());
    }

    #[test]
    fn loopback_sign_in_end_to_end() {
        use crate::graph::tests::fake_graph;
        use serde_json::json;
        let id_token = format!(
            "e30.{}.sig",
            URL_SAFE_NO_PAD.encode(br#"{"name":"Pat Admin","preferred_username":"pat@contoso.gov","tid":"t-1","oid":"o-1"}"#)
        );
        // A stand-in for login.microsoftonline.com's token endpoint.
        let fake = fake_graph(move |method, path, _, _| {
            assert_eq!((method, path), ("POST", "/contoso.gov/oauth2/v2.0/token"));
            (
                200,
                vec![],
                json!({"access_token": "at", "refresh_token": "rt", "expires_in": 3600, "id_token": id_token}),
            )
        });
        let mut cfg = AuthConfig::new("contoso.gov", "client-1");
        cfg.authority = fake.base.clone();
        let agent = crate::graph::agent();
        // The "browser": follow the sign-in URL's redirect back to the app, as Entra would.
        let browser = |url: &str| -> Result<(), String> {
            let q = url.split_once('?').unwrap().1;
            let get = |k: &str| {
                q.split('&')
                    .find_map(|p| p.strip_prefix(&format!("{k}=")))
                    .map(decode)
                    .unwrap()
            };
            let (redirect, state) = (get("redirect_uri"), get("state"));
            assert!(redirect.starts_with("http://localhost:"));
            std::thread::spawn(move || {
                let agent = crate::graph::agent();
                let _ = agent.get(&format!("{redirect}/favicon.ico")).call();
                let mut res = agent
                    .get(&format!("{redirect}/?code=the-code&state={state}"))
                    .call()
                    .unwrap();
                assert!(
                    res.body_mut()
                        .read_to_string()
                        .unwrap()
                        .contains("You're signed in")
                );
            });
            Ok(())
        };
        let t = sign_in(&agent, &cfg, browser, Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(
            (t.access.as_str(), t.refresh.as_deref()),
            ("at", Some("rt"))
        );
        assert_eq!(t.user.username, "pat@contoso.gov");
        assert!(!t.expiring(Duration::from_secs(60)));
        assert_eq!(fake.seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn id_token_claims() {
        let payload = URL_SAFE_NO_PAD.encode(
            br#"{"name":"Pat Admin","preferred_username":"pat@contoso.gov","tid":"t-1","oid":"o-1"}"#,
        );
        let c = id_claims(&format!("e30.{payload}.sig"));
        assert_eq!(
            (c.name.as_str(), c.preferred_username.as_str()),
            ("Pat Admin", "pat@contoso.gov")
        );
        assert_eq!((c.tid.as_str(), c.oid.as_str()), ("t-1", "o-1"));
        assert_eq!(id_claims("garbage").name, "");
    }
}
