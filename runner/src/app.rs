//! The search runner's screens: open a `.req` file, sign in as an eDiscovery admin, check where
//! to search, pick or create a Purview case, then create the search and run Purview's estimate.
//! Everything the runner does is written to the audit log first.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use eframe::egui::{self, Align, Color32, Layout, Margin, RichText, Vec2};
use keyql_builder::query;
use keyql_builder::reqfile::{self, Contents};
use keyql_builder::ui_kit::{
    Tone, app_mark, badge, callout, field_label, numbered, primary_button, setup_style,
    status_line, tone_color,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::audit::{self, AuditLog, Entry, Verification};
use crate::auth::{self, AuthConfig, Tokens};
use crate::graph::{self, Case, CaseTarget, Estimate, Graph, Progress, RunFailed, RunJob};
use crate::plan::{self, Names, SourceKind, SourcePlan};

/// How often to check on a running estimate, and how long to wait before leaving it to Purview.
const POLL: Duration = Duration::from_secs(5);
const GIVE_UP: Duration = Duration::from_secs(20 * 60);

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Directory (tenant) ID, or the tenant's domain.
    pub tenant: String,
    /// Application (client) ID of the runner's app registration.
    pub client_id: String,
    /// Where the audit log lives; empty means the app's data folder.
    pub log_dir: String,
}

impl Settings {
    fn ready(&self) -> bool {
        !self.tenant.trim().is_empty() && !self.client_id.trim().is_empty()
    }
}

/// Microsoft Graph, or (debug builds only) a local stand-in for automated UI tests.
fn graph_base() -> String {
    if cfg!(debug_assertions)
        && let Ok(base) = std::env::var("KEYQL_RUNNER_GRAPH")
    {
        return base;
    }
    graph::GRAPH.to_owned()
}

/// Debug builds only: skip the browser and sign in as a test admin, for automated UI tests
/// against the local Graph stand-in. Release builds always use the real sign-in.
fn test_sign_in() -> Option<Tokens> {
    (cfg!(debug_assertions) && std::env::var_os("KEYQL_RUNNER_TEST_SIGN_IN").is_some()).then(|| {
        Tokens {
            access: "test-token".into(),
            refresh: None,
            expires_at: std::time::Instant::now() + Duration::from_secs(3600),
            user: auth::SignedInUser {
                name: "Test Admin".into(),
                username: "test.admin@example.gov".into(),
                tenant_id: "test-tenant".into(),
                object_id: "test-user".into(),
            },
        }
    })
}

fn default_log_dir() -> PathBuf {
    eframe::storage_dir("keyql-runner")
        .unwrap_or_else(|| PathBuf::from("."))
        .join("audit")
}

/// Who did it, before anyone signs in: the computer account.
fn computer_account() -> String {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".into());
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default();
    if host.is_empty() {
        format!("{user} (not signed in)")
    } else {
        format!("{host}\\{user} (not signed in)")
    }
}

enum Msg {
    SignedIn(Result<(Tokens, Option<graph::Me>), String>),
    Tokens(Tokens),
    Cases(Result<Vec<Case>, String>),
    Progress(Progress),
    RunDone(Result<Estimate, RunFailed>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Request,
    Audit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CaseMode {
    Existing,
    New,
}

struct Opened {
    file: String,
    sha256: String,
    contents: Contents,
}

/// The state of creating the search and running the estimate.
#[derive(Default)]
struct Run {
    lines: Vec<(Tone, String)>,
    estimate: Option<Estimate>,
    case: Option<(String, String)>,
    search: Option<(String, String)>,
    finished: bool,
    failed: Option<String>,
}

pub struct RunnerApp {
    settings: Settings,
    draft: Option<Settings>,
    page: Page,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    busy: Option<&'static str>,
    cancel: Arc<AtomicBool>,

    tokens: Option<Tokens>,
    me: Option<graph::Me>,
    sign_in_error: Option<String>,

    opened: Option<Opened>,
    open_error: Option<(String, String)>,
    plan: Option<SourcePlan>,
    names: Option<Names>,

    cases: Vec<Case>,
    cases_error: Option<String>,
    case_mode: CaseMode,
    chosen: Option<usize>,
    new_case: (String, String, String),

    confirm: bool,
    run: Option<Run>,

    audit: Result<AuditLog, String>,
    /// Counts files opened, so each one's page starts scrolled to the top.
    opened_count: u32,
    verification: Option<Verification>,
    entries: Vec<Entry>,
}

impl RunnerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_style(&cc.egui_ctx);
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        let (tx, rx) = channel();
        let mut app = Self {
            audit: Err(String::new()),
            opened_count: 0,
            settings,
            draft: None,
            page: Page::Home,
            tx,
            rx,
            busy: None,
            cancel: Arc::new(AtomicBool::new(false)),
            tokens: None,
            me: None,
            sign_in_error: None,
            opened: None,
            open_error: None,
            plan: None,
            names: None,
            cases: Vec::new(),
            cases_error: None,
            case_mode: CaseMode::Existing,
            chosen: None,
            new_case: Default::default(),
            confirm: false,
            run: None,
            verification: None,
            entries: Vec::new(),
        };
        app.open_audit();
        app.log(
            "runner_started",
            json!({"version": env!("CARGO_PKG_VERSION"), "os": std::env::consts::OS, "computer_account": computer_account()}),
        );
        if !app.settings.ready() {
            app.draft = Some(app.settings.clone());
        }
        // Debug builds only: open a file at startup, for automated UI tests.
        if cfg!(debug_assertions)
            && let Some(path) = std::env::var_os("KEYQL_RUNNER_OPEN")
        {
            app.open_path(Path::new(&path), &cc.egui_ctx);
        }
        app
    }

    fn log_dir(&self) -> PathBuf {
        if self.settings.log_dir.trim().is_empty() {
            default_log_dir()
        } else {
            PathBuf::from(self.settings.log_dir.trim())
        }
    }

    fn open_audit(&mut self) {
        let dir = self.log_dir();
        self.audit = AuditLog::open(&dir)
            .map_err(|e| format!("The audit log at {} can't be opened: {e}", dir.display()));
    }

    fn actor(&self) -> String {
        match (&self.me, &self.tokens) {
            (Some(me), _) if !me.upn.is_empty() => me.upn.clone(),
            (_, Some(t)) if !t.user.username.is_empty() => t.user.username.clone(),
            _ => computer_account(),
        }
    }

    /// Write an audit entry. If the log can't be written, say so: nothing is run without it.
    fn log(&mut self, event: &str, details: serde_json::Value) {
        let actor = self.actor();
        if let Ok(a) = &mut self.audit
            && let Err(e) = a.append(&actor, event, details)
        {
            self.audit = Err(format!("The audit log couldn't be written: {e}"));
        }
    }

    fn auth_config(&self) -> AuthConfig {
        AuthConfig::new(&self.settings.tenant, &self.settings.client_id)
    }

    // ---------------------------------------------------------------- background work

    fn spawn(
        &mut self,
        label: &'static str,
        ctx: &egui::Context,
        job: impl FnOnce(Sender<Msg>) + Send + 'static,
    ) {
        self.busy = Some(label);
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            job(tx);
            ctx.request_repaint();
        });
    }

    fn sign_in(&mut self, ctx: &egui::Context) {
        self.sign_in_error = None;
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let cfg = self.auth_config();
        self.log(
            "sign_in_started",
            json!({"tenant": cfg.tenant, "client_id": cfg.client_id}),
        );
        self.spawn(
            "Waiting for you to sign in in your browser…",
            ctx,
            move |tx| {
                let agent = graph::agent();
                let result = match test_sign_in() {
                    Some(t) => Ok(t),
                    None => auth::sign_in(
                        &agent,
                        &cfg,
                        |url| open::that(url).map_err(|e| e.to_string()),
                        cancel,
                    ),
                }
                .map(|t| {
                    let me = Graph {
                        agent: &agent,
                        base: graph_base(),
                        token: t.access.clone(),
                    }
                    .me()
                    .ok();
                    (t, me)
                });
                let _ = tx.send(Msg::SignedIn(result));
            },
        );
    }

    /// A job that needs a valid access token: refreshes it first when it's about to run out.
    fn with_token(
        &mut self,
        label: &'static str,
        ctx: &egui::Context,
        job: impl FnOnce(&Graph, &Sender<Msg>) + Send + 'static,
    ) {
        let Some(tokens) = self.tokens.clone() else {
            return;
        };
        let cfg = self.auth_config();
        self.spawn(label, ctx, move |tx| {
            let agent = graph::agent();
            let tokens = if tokens.expiring(Duration::from_secs(300)) {
                match auth::refresh(&agent, &cfg, &tokens) {
                    Ok(t) => {
                        let _ = tx.send(Msg::Tokens(t.clone()));
                        t
                    }
                    Err(e) => {
                        let _ = tx.send(Msg::SignedIn(Err(e)));
                        return;
                    }
                }
            } else {
                tokens
            };
            let g = Graph {
                agent: &agent,
                base: graph_base(),
                token: tokens.access,
            };
            job(&g, &tx);
        });
    }

    fn load_cases(&mut self, ctx: &egui::Context) {
        self.cases_error = None;
        self.with_token("Loading eDiscovery cases…", ctx, |g, tx| {
            let _ = tx.send(Msg::Cases(g.list_cases().map_err(|e| e.to_string())));
        });
    }

    fn start_run(&mut self, ctx: &egui::Context) {
        let (Some(o), Some(plan), Some(names)) = (&self.opened, &self.plan, &self.names) else {
            return;
        };
        let case = match self.case_mode {
            CaseMode::Existing => match self.chosen.and_then(|i| self.cases.get(i)) {
                Some(c) => CaseTarget::Existing {
                    id: c.id.clone(),
                    name: c.name.clone(),
                },
                None => return,
            },
            CaseMode::New => CaseTarget::New {
                name: self.new_case.0.trim().to_owned(),
                description: self.new_case.1.trim().to_owned(),
                external_id: self.new_case.2.trim().to_owned(),
            },
        };
        let job = RunJob {
            case,
            search_name: names.search.clone(),
            search_description: names.description.clone(),
            query: o.contents.query.clone(),
            sources: plan.clone(),
        };
        let sources: Vec<_> = plan
            .active()
            .map(|e| json!({"asked": e.asked, "kind": kind_name(e.kind), "value": e.value.trim()}))
            .collect();
        let skipped: Vec<_> = plan
            .entries
            .iter()
            .filter(|e| e.skip)
            .map(|e| e.asked.clone())
            .collect();
        let details = json!({
            "file": o.file, "file_sha256": o.sha256, "request": o.contents.name(),
            "reference": o.contents.request.about.reference, "query": o.contents.query,
            "search_name": job.search_name, "everyone": plan.everyone,
            "sources": sources, "skipped": skipped,
            "case": match &job.case {
                CaseTarget::Existing { id, name } => json!({"existing": id, "name": name}),
                CaseTarget::New { name, external_id, .. } => json!({"new": name, "external_id": external_id}),
            },
        });
        self.log("run_started", details);
        self.run = Some(Run::default());
        self.with_token("Running the search in Purview…", ctx, move |g, tx| {
            let mut send = |p| {
                let _ = tx.send(Msg::Progress(p));
            };
            let result = graph::run(g, &job, POLL, GIVE_UP, &mut send);
            let _ = tx.send(Msg::RunDone(result));
        });
    }

    fn handle(&mut self, msg: Msg, ctx: &egui::Context) {
        match msg {
            Msg::SignedIn(Ok((tokens, me))) => {
                self.busy = None;
                let u = tokens.user.clone();
                self.tokens = Some(tokens);
                self.me = me;
                let upn = self.me.as_ref().map(|m| m.upn.clone()).unwrap_or_default();
                self.log(
                    "signed_in",
                    json!({"name": u.name, "username": u.username, "upn": upn, "tenant_id": u.tenant_id, "object_id": u.object_id}),
                );
                if self.opened.is_some() {
                    self.load_cases(ctx);
                }
            }
            Msg::SignedIn(Err(e)) => {
                self.busy = None;
                self.log("sign_in_failed", json!({"reason": e}));
                self.sign_in_error = Some(e);
                self.tokens = None;
                self.me = None;
            }
            Msg::Tokens(t) => self.tokens = Some(t),
            Msg::Cases(r) => {
                self.busy = None;
                match r {
                    Ok(c) => {
                        self.cases = c;
                        if self.chosen.is_some_and(|i| i >= self.cases.len()) {
                            self.chosen = None;
                        }
                        if self.cases.is_empty() {
                            self.case_mode = CaseMode::New;
                        }
                    }
                    Err(e) => self.cases_error = Some(e),
                }
            }
            Msg::Progress(p) => self.progress(p),
            Msg::RunDone(r) => {
                self.busy = None;
                let Some(run) = self.run.as_mut() else { return };
                run.finished = true;
                match r {
                    Ok(est) => {
                        let (tone, line) = if est.ok() {
                            (Tone::Success, "Estimate finished.".to_owned())
                        } else {
                            (
                                Tone::Error,
                                format!(
                                    "The estimate didn't finish: {}",
                                    if est.error.is_empty() {
                                        &est.status
                                    } else {
                                        &est.error
                                    }
                                ),
                            )
                        };
                        run.lines.push((tone, line));
                        run.estimate = Some(est.clone());
                        let (case, search) = (run.case.clone(), run.search.clone());
                        self.log(
                            "estimate_finished",
                            json!({"status": est.status, "items": est.items, "size_bytes": est.size,
                                   "unindexed_items": est.unindexed_items, "unindexed_size_bytes": est.unindexed_size,
                                   "mailboxes": est.mailboxes, "sites": est.sites, "error": est.error,
                                   "case": case.map(|c| c.0), "search": search.map(|s| s.0)}),
                        );
                    }
                    Err(f) => {
                        let msg = format!("Couldn't {}: {}", f.step, f.error);
                        run.lines.push((Tone::Error, msg.clone()));
                        run.failed = Some(msg);
                        let (case, search) = (run.case.clone(), run.search.clone());
                        self.log(
                            "run_failed",
                            json!({"step": f.step, "status": f.error.status, "code": f.error.code, "message": f.error.message,
                                   "case": case.map(|c| c.0), "search": search.map(|s| s.0)}),
                        );
                    }
                }
            }
        }
    }

    fn progress(&mut self, p: Progress) {
        let Some(run) = self.run.as_mut() else { return };
        let (event, details) = match &p {
            Progress::CaseCreated { id, name } => {
                run.case = Some((id.clone(), name.clone()));
                run.lines
                    .push((Tone::Success, format!("Created the case \"{name}\".")));
                ("case_created", json!({"case": id, "name": name}))
            }
            Progress::CaseChosen { id, name } => {
                run.case = Some((id.clone(), name.clone()));
                run.lines
                    .push((Tone::Success, format!("Using the case \"{name}\".")));
                ("case_selected", json!({"case": id, "name": name}))
            }
            Progress::SearchCreated { id, name } => {
                run.search = Some((id.clone(), name.clone()));
                run.lines.push((
                    Tone::Success,
                    format!("Created the search \"{name}\" with the request's query."),
                ));
                (
                    "search_created",
                    json!({"search": id, "name": name, "case": run.case.as_ref().map(|c| c.0.clone())}),
                )
            }
            Progress::SourceAdded { kind, value } => {
                run.lines.push((
                    Tone::Success,
                    format!("Added the {} {value}.", kind_label(*kind)),
                ));
                (
                    "source_added",
                    json!({"kind": kind_name(*kind), "value": value, "search": run.search.as_ref().map(|s| s.0.clone())}),
                )
            }
            Progress::EstimateStarted { operation } => {
                run.lines.push((
                    Tone::Info,
                    "Started Purview's estimate. This can take a few minutes.".into(),
                ));
                ("estimate_started", json!({"operation": operation}))
            }
            Progress::EstimateUpdate(e) => {
                run.estimate = Some(e.clone());
                return;
            }
        };
        self.log(event, details);
    }

    // ---------------------------------------------------------------- files

    fn open_dialog(&mut self, ctx: &egui::Context) {
        let mut d = rfd::FileDialog::new()
            .set_title("Open a request file")
            .add_filter("eDiscovery request (.req)", &[reqfile::EXTENSION]);
        if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            d = d.set_directory(PathBuf::from(home));
        }
        if let Some(path) = d.pick_file() {
            self.open_path(&path, ctx);
        }
    }

    fn open_path(&mut self, path: &Path, ctx: &egui::Context) {
        let file = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        match std::fs::read(path) {
            Ok(bytes) => self.open_bytes(file, &bytes, ctx),
            Err(e) => {
                self.log(
                    "request_rejected",
                    json!({"file": file, "reason": e.to_string()}),
                );
                self.open_error = Some((file, format!("The file couldn't be read: {e}")));
                self.page = Page::Request;
            }
        }
    }

    fn open_bytes(&mut self, file: String, bytes: &[u8], ctx: &egui::Context) {
        let sha256 = audit::sha256_hex(bytes);
        self.opened_count += 1;
        self.run = None;
        self.confirm = false;
        self.page = Page::Request;
        match reqfile::decode(bytes) {
            Ok(c) => {
                self.log(
                    "request_opened",
                    json!({"file": file, "sha256": sha256, "bytes": bytes.len(), "request": c.name(),
                           "reference": c.request.about.reference, "kind": c.request.about.kind.label(),
                           "saved_on": c.saved_on.to_string(), "saved_with": c.app_version, "query": c.query}),
                );
                let names = plan::names(&c);
                self.new_case = (
                    names.case.clone(),
                    names.description.clone(),
                    names.external_id.clone(),
                );
                self.plan = Some(SourcePlan::from_request(&c));
                self.names = Some(names);
                self.opened = Some(Opened {
                    file,
                    sha256,
                    contents: c,
                });
                self.open_error = None;
                if self.tokens.is_some() && self.cases.is_empty() {
                    self.load_cases(ctx);
                }
            }
            Err(e) => {
                self.log(
                    "request_rejected",
                    json!({"file": file, "sha256": sha256, "reason": e.to_string()}),
                );
                self.opened = None;
                self.plan = None;
                self.open_error = Some((file, e.to_string()));
            }
        }
    }

    fn take_dropped(&mut self, ctx: &egui::Context) {
        let Some(f) = ctx.input(|i| i.raw.dropped_files.first().cloned()) else {
            return;
        };
        let path = f.path().to_path_buf();
        self.open_path(&path, ctx);
    }
}

fn kind_name(k: SourceKind) -> &'static str {
    match k {
        SourceKind::Mailbox => "mailbox",
        SourceKind::Site => "site",
    }
}

fn kind_label(k: SourceKind) -> &'static str {
    match k {
        SourceKind::Mailbox => "mailbox",
        SourceKind::Site => "site",
    }
}

fn card(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(8)
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        });
}

fn section(ui: &mut egui::Ui, n: usize, title: &str) {
    ui.add_space(22.0);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), 14.0, keyql_builder::ui_kit::ACCENT);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            n.to_string(),
            egui::FontId::proportional(15.0),
            Color32::WHITE,
        );
        ui.label(RichText::new(title).size(21.0).strong());
    });
    ui.add_space(6.0);
}

impl eframe::App for RunnerApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.settings);
    }

    fn on_exit(&mut self) {
        self.log("runner_closed", json!({}));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        while let Ok(m) = self.rx.try_recv() {
            self.handle(m, &ctx);
        }
        if self.busy.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        self.take_dropped(&ctx);

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            // Each page, and each opened file, keeps its own scroll position, starting at the top.
            let page = match self.page {
                Page::Home => "home",
                Page::Request => "request",
                Page::Audit => "audit",
            };
            egui::ScrollArea::vertical()
                .id_salt((page, self.opened_count))
                .auto_shrink(false)
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(Margin::symmetric(32, 20))
                        .show(ui, |ui| {
                            ui.set_max_width(ui.available_width().min(900.0));
                            match self.page {
                                Page::Home => self.home_page(ui),
                                Page::Request => self.request_page(ui),
                                Page::Audit => self.audit_page(ui),
                            }
                        });
                });
        });
        self.settings_window(&ctx);
        self.confirm_dialog(&ctx);
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let r = ctx.content_rect();
            let p = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("drop"),
            ));
            p.rect_filled(r, 0, Color32::from_black_alpha(150));
            p.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "Drop a request file (.req) to open it",
                egui::FontId::proportional(26.0),
                Color32::WHITE,
            );
        }
    }
}

// ---------------------------------------------------------------- screens

impl RunnerApp {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            app_mark(ui, 26.0);
            ui.label(
                RichText::new("eDiscovery Search Runner")
                    .size(20.0)
                    .strong(),
            );
            ui.add_space(16.0);
            for (page, label) in [
                (Page::Home, "Home"),
                (Page::Request, "Request"),
                (Page::Audit, "Audit log"),
            ] {
                if page == Page::Request && self.opened.is_none() && self.open_error.is_none() {
                    continue;
                }
                if ui.selectable_label(self.page == page, label).clicked() {
                    self.page = page;
                    if page == Page::Audit {
                        self.refresh_audit();
                    }
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Settings").clicked() {
                    self.draft = Some(self.settings.clone());
                }
                ui.separator();
                if let Some(t) = &self.tokens {
                    let who = self
                        .me
                        .as_ref()
                        .map(|m| m.upn.clone())
                        .unwrap_or_else(|| t.user.username.clone());
                    if ui.button("Sign out").clicked() {
                        self.log("signed_out", json!({}));
                        self.tokens = None;
                        self.me = None;
                        self.cases.clear();
                    } else {
                        ui.label(RichText::new(format!("Signed in as {who}")).strong());
                        badge(ui, Tone::Success, 16.0);
                    }
                } else if self.busy.is_some()
                    && self.tokens.is_none()
                    && !self.cancel.load(Ordering::Relaxed)
                {
                    if ui.button("Cancel sign-in").clicked() {
                        self.cancel.store(true, Ordering::Relaxed);
                    }
                    ui.spinner();
                } else {
                    let ok = self.settings.ready();
                    if ui
                        .add_enabled(
                            ok,
                            egui::Button::new(
                                RichText::new("Sign in").strong().color(Color32::WHITE),
                            )
                            .fill(keyql_builder::ui_kit::ACCENT),
                        )
                        .on_disabled_hover_text(
                            "Add the tenant ID and client ID in Settings first.",
                        )
                        .clicked()
                    {
                        let ctx = ui.ctx().clone();
                        self.sign_in(&ctx);
                    }
                }
            });
        });
        ui.add_space(6.0);
    }

    fn home_page(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("Run an eDiscovery search request")
                .size(26.0)
                .strong(),
        );
        ui.label(
            RichText::new(
                "For the eDiscovery administrator. Every step is recorded in the audit log.",
            )
            .weak(),
        );
        ui.add_space(12.0);
        callout(ui, Tone::Info, "How this works", |ui| {
            numbered(
                ui,
                1,
                "Sign in with your Microsoft 365 admin account (top right).",
            );
            numbered(ui, 2, "Open the .req request file you were sent.");
            numbered(
                ui,
                3,
                "Check where to search, and choose or create the Purview case.",
            );
            numbered(
                ui,
                4,
                "Create the search and run Purview's estimate. Review and export in Purview.",
            );
        });
        ui.add_space(14.0);
        if !self.settings.ready() {
            status_line(
                ui,
                Tone::Warning,
                "The app isn't set up yet: add the tenant ID and client ID in Settings.",
            );
            ui.add_space(8.0);
        }
        if let Err(e) = &self.audit {
            status_line(
                ui,
                Tone::Error,
                format!("{e} Searches can't be run until it's fixed (see Settings)."),
            );
            ui.add_space(8.0);
        }
        if let Some(e) = &self.sign_in_error {
            status_line(ui, Tone::Error, e.as_str());
            ui.add_space(8.0);
        }
        let ctx = ui.ctx().clone();
        if primary_button(ui, "Open a request file…").clicked() {
            self.open_dialog(&ctx);
        }
        ui.label(RichText::new("Or drop a .req file on this window.").weak());
    }

    fn request_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some((file, err)) = self.open_error.clone() {
            ui.label(RichText::new("Request file").weak());
            ui.label(RichText::new(&file).size(26.0).strong());
            ui.add_space(10.0);
            callout(ui, Tone::Error, "This file can't be used", |ui| {
                ui.label(err);
            });
            ui.add_space(10.0);
            if ui.button("Open another file…").clicked() {
                self.open_dialog(&ctx);
            }
            return;
        }
        let Some(o) = &self.opened else {
            self.page = Page::Home;
            return;
        };
        let c = &o.contents;
        ui.label(RichText::new("Request file").weak());
        ui.label(RichText::new(c.name()).size(26.0).strong());
        status_line(
            ui,
            Tone::Success,
            format!(
                "{} is complete and unchanged since it was saved on {}.",
                o.file,
                query::long_date(c.saved_on)
            ),
        );
        ui.label(
            RichText::new(format!("SHA-256 {}", o.sha256))
                .small()
                .weak()
                .monospace(),
        );

        // 1. What was asked for
        section(ui, 1, "What was asked for");
        for (heading, lines) in &c.summary {
            card(ui, |ui| {
                ui.label(RichText::new(heading).strong());
                for l in lines {
                    ui.add(egui::Label::new(l).wrap().selectable(true));
                }
            });
            ui.add_space(4.0);
        }
        ui.add_space(4.0);
        ui.label(RichText::new("The search (KeyQL)").strong());
        card(ui, |ui| {
            ui.add(
                egui::Label::new(RichText::new(&c.query).monospace())
                    .wrap()
                    .selectable(true),
            );
        });

        // 2. Where to search
        section(ui, 2, "Where to search");
        let locked = self.run.is_some();
        if let Some(plan) = self.plan.as_mut() {
            ui.add_enabled_ui(!locked, |ui| sources_editor(ui, plan));
        }

        // 3. Case
        section(ui, 3, "Purview case");
        ui.add_enabled_ui(!locked, |ui| self.case_picker(ui));

        // 4. Run
        section(ui, 4, "Create the search and estimate");
        self.run_panel(ui);
    }

    fn case_picker(&mut self, ui: &mut egui::Ui) {
        if self.tokens.is_none() {
            ui.label(RichText::new("Sign in (top right) to choose or create a case.").weak());
            return;
        }
        ui.radio_value(
            &mut self.case_mode,
            CaseMode::Existing,
            "Add the search to an existing case",
        );
        if self.case_mode == CaseMode::Existing {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                let label = self
                    .chosen
                    .and_then(|i| self.cases.get(i))
                    .map(case_label)
                    .unwrap_or_else(|| {
                        if self.cases.is_empty() {
                            "No cases found".into()
                        } else {
                            "Choose a case…".into()
                        }
                    });
                egui::ComboBox::from_id_salt("case")
                    .selected_text(label)
                    .width(520.0)
                    .show_ui(ui, |ui| {
                        for (i, c) in self.cases.iter().enumerate() {
                            ui.selectable_value(&mut self.chosen, Some(i), case_label(c));
                        }
                    });
                if ui.button("Refresh").clicked() {
                    let ctx = ui.ctx().clone();
                    self.load_cases(&ctx);
                }
            });
            if let Some(e) = &self.cases_error {
                status_line(ui, Tone::Error, e.as_str());
            }
        }
        ui.radio_value(
            &mut self.case_mode,
            CaseMode::New,
            "Create a new case for this request",
        );
        if self.case_mode == CaseMode::New {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.vertical(|ui| {
                    field_label(ui, "Case name", None, "");
                    ui.add(egui::TextEdit::singleline(&mut self.new_case.0).desired_width(520.0));
                    field_label(
                        ui,
                        "Reference number",
                        None,
                        "Saved as the case's external ID.",
                    );
                    ui.add(egui::TextEdit::singleline(&mut self.new_case.2).desired_width(240.0));
                    field_label(ui, "Description", None, "");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.new_case.1)
                            .desired_rows(2)
                            .desired_width(520.0),
                    );
                });
            });
        }
    }

    /// Everything that must be true before running, as (ok, text).
    fn checks(&self) -> Vec<(bool, String)> {
        let mut out = vec![
            (
                self.tokens.is_some(),
                "Signed in as an eDiscovery administrator".to_owned(),
            ),
            (self.audit.is_ok(), "The audit log is working".to_owned()),
        ];
        let problems = self.plan.as_ref().map(|p| p.problems()).unwrap_or_default();
        out.push((
            problems.is_empty(),
            if problems.is_empty() {
                "Where to search is complete".into()
            } else {
                problems.join(" ")
            },
        ));
        let case_ok = match self.case_mode {
            CaseMode::Existing => self.chosen.is_some(),
            CaseMode::New => !self.new_case.0.trim().is_empty(),
        };
        out.push((case_ok, "A case is chosen or named".to_owned()));
        out
    }

    fn run_panel(&mut self, ui: &mut egui::Ui) {
        if let Some(run) = &self.run {
            for (tone, line) in &run.lines {
                status_line(ui, *tone, line.as_str());
            }
            if let Some(est) = &run.estimate {
                if !est.finished() {
                    ui.add_space(4.0);
                    ui.add(
                        egui::ProgressBar::new(est.percent as f32 / 100.0)
                            .show_percentage()
                            .desired_width(480.0),
                    );
                } else if est.ok() {
                    ui.add_space(8.0);
                    results(ui, est);
                }
            } else if !run.finished {
                ui.spinner();
            }
            if run.finished {
                ui.add_space(10.0);
                if let (Some((_, case)), Some((_, search))) = (&run.case, &run.search) {
                    callout(ui, Tone::Info, "Next, in Microsoft Purview", |ui| {
                        ui.label(format!(
                            "Open purview.microsoft.com, go to eDiscovery, Cases, \"{case}\", and the search \"{search}\" to review the results, add them to a review set or export them."
                        ));
                    });
                }
                ui.add_space(8.0);
                if ui.button("Open another request…").clicked() {
                    let ctx = ui.ctx().clone();
                    self.open_dialog(&ctx);
                }
            }
            return;
        }
        let checks = self.checks();
        for (ok, text) in &checks {
            status_line(
                ui,
                if *ok { Tone::Success } else { Tone::Warning },
                text.as_str(),
            );
        }
        ui.add_space(8.0);
        let ready = checks.iter().all(|(ok, _)| *ok) && self.busy.is_none();
        if ui
            .add_enabled(
                ready,
                egui::Button::new(
                    RichText::new("Create the search and run the estimate…")
                        .size(17.0)
                        .strong()
                        .color(Color32::WHITE),
                )
                .fill(keyql_builder::ui_kit::ACCENT)
                .min_size(Vec2::new(0.0, 40.0)),
            )
            .clicked()
        {
            self.confirm = true;
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm {
            return;
        }
        let who = self.actor();
        let case = match self.case_mode {
            CaseMode::Existing => self
                .chosen
                .and_then(|i| self.cases.get(i))
                .map(|c| format!("the existing case \"{}\"", c.name)),
            CaseMode::New => Some(format!("a new case \"{}\"", self.new_case.0.trim())),
        }
        .unwrap_or_default();
        let sources = self
            .plan
            .as_ref()
            .map(|p| {
                if p.everyone {
                    "every mailbox and site in the organization".to_owned()
                } else {
                    let n = p.active().count();
                    format!(
                        "{n} mailbox{} or site{}",
                        if n == 1 { "" } else { "es" },
                        if n == 1 { "" } else { "s" }
                    )
                }
            })
            .unwrap_or_default();
        let mut go = false;
        let r = egui::Modal::new(egui::Id::unique("confirm_run")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(RichText::new("Run this search in Purview?").size(20.0).strong());
            ui.add_space(4.0);
            ui.label(format!("As {who}, this creates a search in {case} covering {sources}, then runs Purview's estimate."));
            ui.label(RichText::new("It's recorded in the audit log and in Purview's own audit log.").weak());
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if primary_button(ui, "Run the search").clicked() {
                    go = true;
                }
                if ui.button("Not yet").clicked() {
                    self.confirm = false;
                }
            });
        });
        if r.should_close() {
            self.confirm = false;
        }
        if go {
            self.confirm = false;
            self.start_run(ctx);
        }
    }

    fn refresh_audit(&mut self) {
        if let Ok(a) = &self.audit {
            self.entries = a.entries();
            self.entries.reverse();
        }
    }

    fn audit_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Audit log").size(26.0).strong());
        ui.label(
            RichText::new("Everything this app does, oldest entries first in the file. Each entry is chained to the one before it, so a changed or removed entry shows up when the log is checked.")
                .weak(),
        );
        ui.add_space(8.0);
        match &self.audit {
            Err(e) => {
                status_line(ui, Tone::Error, e.as_str());
                return;
            }
            Ok(a) => {
                let path = a.path().to_path_buf();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(path.display().to_string())
                            .monospace()
                            .small(),
                    );
                    if ui.small_button("Show in folder").clicked()
                        && let Some(dir) = path.parent()
                    {
                        let _ = open::that(dir);
                    }
                });
            }
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if primary_button(ui, "Check the log").clicked()
                && let Ok(a) = &self.audit
            {
                let v = a.verify();
                self.log("log_checked", json!({"intact": v.ok(), "entries": v.entries, "fingerprint": v.fingerprint, "broken_at": v.broken.as_ref().map(|b| b.0)}));
                self.verification = Some(v);
                self.refresh_audit();
            }
            if ui.button("Refresh").clicked() {
                self.refresh_audit();
            }
        });
        if let Some(v) = &self.verification {
            ui.add_space(6.0);
            match &v.broken {
                None => status_line(
                    ui,
                    Tone::Success,
                    format!("All {} entries are intact.", v.entries),
                ),
                Some((n, why)) => status_line(
                    ui,
                    Tone::Error,
                    format!(
                        "Entry {n} of {} fails the check: {why}. Entries before it are intact.",
                        v.entries
                    ),
                ),
            }
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Fingerprint {}", v.fingerprint))
                        .monospace()
                        .small(),
                );
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(v.fingerprint.clone());
                }
            });
            ui.label(
                RichText::new("Write the fingerprint down somewhere else (for example in the request's file) to be able to prove later that the log up to now wasn't rewritten.")
                    .small()
                    .weak(),
            );
        }
        ui.add_space(12.0);
        egui::Grid::new("audit_entries")
            .striped(true)
            .num_columns(4)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                for h in ["When", "Who", "What", "Details"] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for e in self.entries.iter().take(500) {
                    let when = chrono::DateTime::parse_from_rfc3339(&e.time)
                        .map(|t| {
                            t.with_timezone(&chrono::Local)
                                .format("%Y-%m-%d %H:%M:%S")
                                .to_string()
                        })
                        .unwrap_or_else(|_| e.time.clone());
                    ui.label(RichText::new(when).monospace().small());
                    ui.label(RichText::new(&e.actor).small());
                    ui.label(RichText::new(e.event.replace('_', " ")).small().strong());
                    ui.add(
                        egui::Label::new(RichText::new(details_text(&e.details)).small()).wrap(),
                    );
                    ui.end_row();
                }
            });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.draft.take() else {
            return;
        };
        let mut open = true;
        let mut done = false;
        let frame = egui::Frame::window(&ctx.style_of(ctx.theme())).inner_margin(Margin::same(18));
        egui::Window::new("Settings")
            .frame(frame)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 70.0])
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Microsoft Entra ID app registration").size(17.0).strong());
                ui.label(
                    RichText::new("From the runner's app registration in the Entra admin center (see the README: \"Setting up the search runner\").")
                        .weak(),
                );
                ui.add_space(6.0);
                field_label(ui, "Directory (tenant) ID", None, "Or the tenant's domain, such as contoso.onmicrosoft.com.");
                ui.add(egui::TextEdit::singleline(&mut draft.tenant).desired_width(460.0).hint_text("00000000-0000-0000-0000-000000000000"));
                field_label(ui, "Application (client) ID", None, "");
                ui.add(egui::TextEdit::singleline(&mut draft.client_id).desired_width(460.0).hint_text("00000000-0000-0000-0000-000000000000"));
                ui.add_space(10.0);
                field_label(ui, "Audit log folder", None, "Leave empty for the app's own data folder. A shared, backed-up folder is better.");
                ui.add(
                    egui::TextEdit::singleline(&mut draft.log_dir)
                        .desired_width(460.0)
                        .hint_text(default_log_dir().display().to_string()),
                );
                ui.add_space(14.0);
                if primary_button(ui, "Done").clicked() {
                    done = true;
                }
            });
        if done || !open {
            if draft != self.settings {
                let old = self.settings.clone();
                self.settings = draft;
                self.log(
                    "settings_changed",
                    json!({"tenant": [old.tenant, self.settings.tenant], "client_id": [old.client_id, self.settings.client_id],
                           "log_dir": [old.log_dir, self.settings.log_dir]}),
                );
                if old.log_dir != self.settings.log_dir {
                    self.open_audit();
                    self.log(
                        "log_moved",
                        json!({"from": old.log_dir, "to": self.settings.log_dir}),
                    );
                }
                if old.tenant != self.settings.tenant || old.client_id != self.settings.client_id {
                    self.tokens = None;
                    self.me = None;
                    self.cases.clear();
                }
            }
        } else {
            self.draft = Some(draft);
        }
    }
}

/// An entry's details for reading: "key: value" pairs on one line, long values shortened.
fn details_text(v: &serde_json::Value) -> String {
    let Some(map) = v.as_object() else {
        return v.to_string();
    };
    let short = |s: String| {
        if s.chars().count() > 90 {
            format!("{}…", s.chars().take(89).collect::<String>())
        } else {
            s
        }
    };
    map.iter()
        .filter(|(_, v)| !v.is_null() && v.as_str() != Some(""))
        .map(|(k, v)| {
            let val = match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            format!("{}: {}", k.replace('_', " "), short(val))
        })
        .collect::<Vec<_>>()
        .join("  ·  ")
}

fn case_label(c: &Case) -> String {
    let mut s = c.name.clone();
    if !c.external_id.is_empty() {
        s.push_str(&format!("  ({})", c.external_id));
    }
    if !c.status.is_empty() && c.status != "active" {
        s.push_str(&format!("  [{}]", c.status));
    }
    s
}

fn sources_editor(ui: &mut egui::Ui, plan: &mut SourcePlan) {
    ui.checkbox(
        &mut plan.everyone,
        "Search every mailbox and site in the organization",
    );
    if plan.everyone {
        ui.label(
            RichText::new(
                "The request asked to search everyone. Any entries below are added as well.",
            )
            .weak(),
        );
    }
    if plan.entries.is_empty() {
        if !plan.everyone {
            status_line(
                ui,
                Tone::Warning,
                "The request doesn't name anyone to search.",
            );
        }
        return;
    }
    ui.add_space(6.0);
    for (i, e) in plan.entries.iter_mut().enumerate() {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if e.is_person { "Person" } else { "Place" })
                        .small()
                        .weak(),
                );
                ui.label(RichText::new(&e.asked).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.checkbox(&mut e.skip, "Leave out");
                });
            });
            if !e.skip {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("kind", i))
                        .selected_text(if e.kind == SourceKind::Mailbox {
                            "Mailbox and OneDrive"
                        } else {
                            "SharePoint site"
                        })
                        .width(190.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut e.kind,
                                SourceKind::Mailbox,
                                "Mailbox and OneDrive",
                            );
                            ui.selectable_value(&mut e.kind, SourceKind::Site, "SharePoint site");
                        });
                    ui.add(
                        egui::TextEdit::singleline(&mut e.value)
                            .desired_width(440.0)
                            .hint_text(if e.kind == SourceKind::Mailbox {
                                "name@example.gov"
                            } else {
                                "https://contoso.sharepoint.com/sites/…"
                            }),
                    );
                });
                if let Some(p) = e.problem() {
                    status_line(ui, Tone::Warning, p);
                }
            }
        });
        ui.add_space(4.0);
    }
}

fn results(ui: &mut egui::Ui, e: &Estimate) {
    let green = tone_color(ui, Tone::Success);
    egui::Frame::new()
        .fill(green.gamma_multiply(0.10))
        .stroke(egui::Stroke::new(2.0, green))
        .corner_radius(10)
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Estimated results").size(19.0).strong());
            ui.add_space(4.0);
            egui::Grid::new("estimate")
                .num_columns(2)
                .spacing([24.0, 6.0])
                .show(ui, |ui| {
                    let rows = [
                        ("Items found", format!("{}", e.items)),
                        ("Size", graph::bytes(e.size)),
                        ("Mailboxes with results", e.mailboxes.to_string()),
                        ("Sites with results", e.sites.to_string()),
                        (
                            "Partly indexed items",
                            format!("{} ({})", e.unindexed_items, graph::bytes(e.unindexed_size)),
                        ),
                    ];
                    for (k, v) in rows {
                        ui.label(RichText::new(k).weak());
                        ui.label(RichText::new(v).strong());
                        ui.end_row();
                    }
                });
            if e.status == "partiallySucceeded" {
                status_line(
                    ui,
                    Tone::Warning,
                    "Some locations couldn't be searched. Check the search in Purview.",
                );
            }
        });
}
