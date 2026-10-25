//! egui front end: a step-by-step request wizard that saves a `.kql` file.
//!
//! The layout follows common form-usability guidance (GOV.UK "one thing per
//! page" and "check answers", NN/g wizard guidelines): a visible list of steps,
//! one question per page with labels and hints above the fields, optional
//! sections tucked away, problems explained in plain words (never by colour
//! alone), and a review page before anything is saved.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::{Duration, NaiveDate};
use eframe::egui::{
    self, Align, Color32, FontId, Key, Layout, Margin, RichText, Sense, Stroke, TextStyle, Vec2,
};
use egui_extras::DatePickerButton;
use serde::{Deserialize, Serialize};

use crate::query::{
    self, Attachments, DateField, DateMode, Found, KeywordScope, Level, Near, OptionalHolidays,
    RequestKind, SearchRequest, SizeOp, Step,
};

const SOURCE_URL: &str = "https://github.com/simontemplarST/keyql-builder";

const STEPS: [Step; 7] = [
    Step::About,
    Step::Whose,
    Step::Words,
    Step::When,
    Step::Types,
    Step::Narrow,
    Step::Review,
];

fn step_title(step: Step) -> &'static str {
    match step {
        Step::About => "About this request",
        Step::Whose => "Whose content",
        Step::Words => "Words to look for",
        Step::When => "Dates",
        Step::Types => "Types of items",
        Step::Narrow => "Narrow it down",
        Step::Review => "Check and save",
    }
}

fn step_question(step: Step) -> &'static str {
    match step {
        Step::About => "What is this request for?",
        Step::Whose => "Whose email, chats or files should be searched?",
        Step::Words => "What words should the search look for?",
        Step::When => "What dates should be searched?",
        Step::Types => "What types of items should be included?",
        Step::Narrow => "Do you want to narrow the results?",
        Step::Review => "Check your request",
    }
}

/// A line under the question, so optional pages say so once instead of on every field.
fn step_subtitle(step: Step) -> Option<&'static str> {
    Some(match step {
        Step::About => {
            "Everything here is optional, but it helps the IT Director get the search right."
        }
        Step::Words => {
            "Optional. Leave this empty to find everything that matches your other answers."
        }
        Step::When => {
            "Optional. Leave it at Any time to search every date. Dates include the whole first and last day."
        }
        Step::Types => "Optional. Leave everything unticked to search every type.",
        Step::Narrow => "Optional. Only use this if you need fewer, more specific results.",
        _ => return None,
    })
}

fn step_optional(step: Step) -> bool {
    matches!(
        step,
        Step::About | Step::Words | Step::When | Step::Types | Step::Narrow
    )
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
enum TextSize {
    #[default]
    Standard,
    Large,
    ExtraLarge,
}

impl TextSize {
    fn zoom(self) -> f32 {
        match self {
            Self::Standard => 1.0,
            Self::Large => 1.15,
            Self::ExtraLarge => 1.3,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    it_director_email: String,
    text_size: TextSize,
    theme: ThemeChoice,
    /// Holidays the office observes beyond the ones every Minnesota government closes for.
    holidays: OptionalHolidays,
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

/// Details found in a pasted request, each with a tick box.
struct Picks {
    found: Found,
    emails: Vec<bool>,
    phrases: Vec<bool>,
    dates: bool,
}

impl Picks {
    fn new(found: Found) -> Self {
        Self {
            emails: vec![true; found.emails.len()],
            phrases: vec![true; found.phrases.len()],
            dates: !found.dates.is_empty(),
            found,
        }
    }

    /// Copy the ticked details into the request. Returns a sentence saying what was added.
    fn apply(&self, req: &mut SearchRequest, pending: &mut Pending) -> String {
        let mut done = Vec::new();
        let people: Vec<&String> = self
            .found
            .emails
            .iter()
            .zip(&self.emails)
            .filter(|(_, on)| **on)
            .map(|(e, _)| e)
            .collect();
        if !people.is_empty() {
            req.custodians.everyone = false;
            for e in &people {
                toggle(&mut req.custodians.people, e, true);
            }
            done.push(count(people.len(), "person", "people") + " to search");
        }
        let words: Vec<&String> = self
            .found
            .phrases
            .iter()
            .zip(&self.phrases)
            .filter(|(_, on)| **on)
            .map(|(p, _)| p)
            .collect();
        if !words.is_empty() {
            for w in &words {
                if !req.keywords.words.iter().any(|x| x.eq_ignore_ascii_case(w)) {
                    req.keywords.words.push((*w).clone());
                }
            }
            done.push(count(words.len(), "phrase", "phrases") + " to look for");
        }
        if self.dates
            && let (Some(&first), Some(&last)) = (self.found.dates.first(), self.found.dates.last())
        {
            let d = &mut req.dates;
            d.start = first;
            d.end = last;
            d.mode = if first == last {
                DateMode::OnOrAfter
            } else {
                DateMode::Between
            };
            pending.remove("when.start");
            pending.remove("when.end");
            done.push("the dates".to_owned());
        }
        if done.is_empty() {
            "Nothing was ticked, so nothing was added.".to_owned()
        } else {
            format!(
                "Added {}. You can check them on the next pages.",
                plain_join(&done)
            )
        }
    }
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn plain_join(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [a] => a.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Text typed into a field but not yet turned into a chip or date, keyed by field.
type Pending = HashMap<String, String>;

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyqlApp {
    req: SearchRequest,
    step: Step,
    settings: Settings,
    /// Steps the user has moved on from; their problems are shown from then on.
    visited: HashSet<Step>,
    #[serde(skip)]
    pending: Pending,
    #[serde(skip)]
    saved: Option<PathBuf>,
    #[serde(skip)]
    save_error: Option<String>,
    #[serde(skip)]
    confirm_reset: bool,
    #[serde(skip)]
    show_settings: bool,
    #[serde(skip)]
    copied_at: Option<f64>,
    /// Details found in the pasted request, waiting for the user to pick which to use.
    #[serde(skip)]
    picks: Option<Picks>,
    /// What "Use these" just added, shown until the page changes.
    #[serde(skip)]
    picked_note: Option<String>,
    #[serde(skip)]
    record_note: Option<Result<String, String>>,
    /// Set when the page changes, so the next page opens at the top.
    #[serde(skip)]
    scroll_top: bool,
    #[serde(skip)]
    applied_zoom: Option<f32>,
    #[serde(skip)]
    applied_theme: Option<ThemeChoice>,
}

impl KeyqlApp {
    pub fn new(cc: &eframe::CreationContext<'_>, example: bool) -> Self {
        setup_style(&cc.egui_ctx);
        let mut app: Self = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        if example {
            app.req = SearchRequest::example(today());
            app.step = Step::Review;
        }
        app
    }

    fn go(&mut self, step: Step) {
        flush_pending(&mut self.req, &mut self.pending);
        self.visited.insert(self.step);
        self.step = step;
        self.save_error = None;
        self.picked_note = None;
        self.scroll_top = true;
    }

    /// Dates typed in a form the app can't read. The request keeps the last
    /// valid date, so saving is blocked until they're fixed.
    fn date_problems(&self) -> Vec<query::Notice> {
        let d = &self.req.dates;
        let checks = [
            (
                "when.start",
                Step::When,
                matches!(d.mode, DateMode::Between | DateMode::OnOrAfter),
            ),
            (
                "when.end",
                Step::When,
                matches!(d.mode, DateMode::Between | DateMode::OnOrBefore),
            ),
            (
                "about.needed_by",
                Step::About,
                self.req.about.needed_by.is_some(),
            ),
            (
                "about.received",
                Step::About,
                self.req.about.received.is_some(),
            ),
        ];
        let mut out: Vec<query::Notice> = Vec::new();
        for (key, step, active) in checks {
            let bad = active
                && self
                    .pending
                    .get(key)
                    .is_some_and(|t| parse_date(t).is_none());
            if bad && !out.iter().any(|n| n.step == step) {
                out.push(query::Notice {
                    level: Level::Error,
                    step,
                    message: "A date can't be read. Type it as MM/DD/YYYY, for example 03/15/2026."
                        .to_owned(),
                });
            }
        }
        out
    }

    fn step_index(&self) -> usize {
        STEPS.iter().position(|s| *s == self.step).unwrap_or(0)
    }

    fn reset(&mut self) {
        let settings = self.settings.clone();
        *self = Self {
            settings,
            ..Default::default()
        };
    }
}

fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

fn setup_style(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, FontId::proportional(24.0)),
            (TextStyle::Body, FontId::proportional(16.0)),
            (TextStyle::Button, FontId::proportional(16.0)),
            (TextStyle::Small, FontId::proportional(13.5)),
            (TextStyle::Monospace, FontId::monospace(15.0)),
        ]
        .into();
        style.spacing.item_spacing = Vec2::new(10.0, 9.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.icon_width = 18.0;
        style.spacing.icon_width_inner = 10.0;
    });
    // Stronger text and field borders than egui's defaults: hint text >= 7:1 and
    // field outlines >= 3:1 against the background (WCAG 1.4.3 / 1.4.11).
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.visuals.weak_text_color = Some(Color32::from_gray(175));
        s.visuals.hyperlink_color = Color32::from_rgb(120, 170, 245);
        s.visuals.widgets.noninteractive.fg_stroke.color = Color32::from_gray(225);
        s.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_gray(115));
    });
    ctx.style_mut_of(egui::Theme::Light, |s| {
        s.visuals.weak_text_color = Some(Color32::from_gray(80));
        // egui's default link blue is about 3:1 on white; this one is over 6:1.
        s.visuals.hyperlink_color = Color32::from_rgb(20, 90, 190);
        s.visuals.widgets.noninteractive.fg_stroke.color = Color32::from_gray(25);
        s.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_gray(120));
    });
}

impl eframe::App for KeyqlApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        flush_pending(&mut self.req, &mut self.pending);
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let zoom = self.settings.text_size.zoom();
        if self.applied_zoom != Some(zoom) {
            ctx.set_zoom_factor(zoom);
            self.applied_zoom = Some(zoom);
        }
        if self.applied_theme != Some(self.settings.theme) {
            ctx.set_theme(match self.settings.theme {
                ThemeChoice::System => egui::ThemePreference::System,
                ThemeChoice::Light => egui::ThemePreference::Light,
                ThemeChoice::Dark => egui::ThemePreference::Dark,
            });
            self.applied_theme = Some(self.settings.theme);
        }

        self.req.holidays = self.settings.holidays;
        let mut built = self.req.build();
        built.notices.extend(self.date_problems());

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::left("steps")
            .resizable(false)
            .exact_size(250.0)
            .show(ui, |ui| self.step_list(ui, &built));
        if self.saved.is_none() {
            egui::Panel::bottom("nav").show(ui, |ui| self.nav_bar(ui, &built));
            // Only when there's room beside the page; the check page shows the same thing in full.
            if ctx.content_rect().width() >= 1200.0 && self.step != Step::Review {
                // The current theme's style (the global one is the dark default).
                let style = ui.style().clone();
                egui::Panel::right("so_far")
                    .resizable(false)
                    .exact_size(290.0)
                    .frame(
                        egui::Frame::side_top_panel(&style)
                            .fill(style.visuals.extreme_bg_color)
                            .inner_margin(Margin::symmetric(16, 0)),
                    )
                    .show(ui, |ui| self.so_far(ui));
            }
        }
        egui::CentralPanel::default().show(ui, |ui| {
            let mut scroll = egui::ScrollArea::vertical().auto_shrink(false);
            if std::mem::take(&mut self.scroll_top) {
                scroll = scroll.vertical_scroll_offset(0.0);
            }
            scroll.show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(Margin::symmetric(28, 18))
                    .show(ui, |ui| {
                        ui.set_max_width(ui.available_width().min(780.0));
                        if self.saved.is_some() {
                            self.done_page(ui);
                        } else {
                            self.step_page(ui, &built);
                        }
                    });
            });
        });

        self.reset_dialog(&ctx);
        self.settings_window(&ctx);
    }
}

// ---------------------------------------------------------------------------
// Frame: top bar, step list, navigation
// ---------------------------------------------------------------------------

impl KeyqlApp {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            app_mark(ui, 26.0);
            ui.label(
                RichText::new("eDiscovery Search Request")
                    .size(20.0)
                    .strong(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .button("Settings")
                    .on_hover_text("Text size, colors and the IT Director's email")
                    .clicked()
                {
                    self.show_settings = true;
                }
                if ui.button("Start a new request").clicked() {
                    self.confirm_reset = true;
                }
            });
        });
        ui.add_space(6.0);
    }

    fn step_list(&mut self, ui: &mut egui::Ui, built: &query::Built) {
        ui.add_space(14.0);
        ui.label(RichText::new("Steps").small().strong());
        ui.add_space(4.0);
        let mut target = None;
        for (i, &step) in STEPS.iter().enumerate() {
            let current = step == self.step && self.saved.is_none();
            let (state, status) = self.step_status(step, built);
            if step_item(ui, i + 1, step_title(step), state, status, current).clicked() {
                target = Some(step);
            }
        }
        if let Some(step) = target {
            self.saved = None;
            self.go(step);
        }
        ui.add_space(12.0);
        ui.separator();
        ui.label(
            RichText::new(if cfg!(target_arch = "wasm32") {
                "Your answers are kept in this browser only, even if you close the tab. Nothing is uploaded."
            } else {
                "Your answers are kept if you close the app."
            })
                .small()
                .weak(),
        );
        ui.add_space(6.0);
        if ui
            .link(RichText::new("Source code on GitHub").small())
            .on_hover_text(format!("{SOURCE_URL} (opens in a new tab)"))
            .clicked()
        {
            open_link(ui.ctx(), SOURCE_URL, true);
        }
    }

    /// "Your request so far": the plain-English summary, kept in view while answering.
    fn so_far(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.add_space(16.0);
                ui.label(RichText::new("Your request so far").size(17.0).strong());
                ui.label(RichText::new("This updates as you answer.").small().weak());
                ui.add_space(4.0);
                let mut target = None;
                for (step, lines) in self.req.describe(today()) {
                    let empty = lines.iter().all(|l| l == "Not given");
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        let title = RichText::new(step_title(step)).small().strong();
                        ui.label(if step == self.step {
                            title.color(tone_color(ui, Tone::Info))
                        } else {
                            title
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if step != self.step
                                && ui.link(RichText::new("Change").small()).clicked()
                            {
                                target = Some(step);
                            }
                        });
                    });
                    if empty {
                        ui.label(RichText::new("Nothing yet").small().weak());
                        continue;
                    }
                    for l in lines {
                        let l = if l.chars().count() > 140 {
                            format!("{}…", l.chars().take(140).collect::<String>())
                        } else {
                            l
                        };
                        ui.label(RichText::new(l).small());
                    }
                }
                if let Some(step) = target {
                    self.go(step);
                }
            });
    }

    fn step_status(&self, step: Step, built: &query::Built) -> (StepState, &'static str) {
        let r = &self.req;
        if step == Step::Review {
            return if built.can_save() {
                (StepState::Plain, "Ready to save")
            } else {
                (StepState::Plain, "")
            };
        }
        let shown = self.visited.contains(&step);
        if shown && built.for_step(step).any(|n| n.level == Level::Error) {
            return (StepState::Error, "Needs fixing");
        }
        let filled = match step {
            Step::About => {
                r.about.kind != RequestKind::NotSaid
                    || !r.about.name.trim().is_empty()
                    || !r.about.reason.trim().is_empty()
            }
            Step::Whose => {
                r.custodians.everyone
                    || !r.custodians.people.is_empty()
                    || !r.custodians.locations.is_empty()
            }
            Step::Words => {
                !r.keywords.words.is_empty()
                    || !r.keywords.none.is_empty()
                    || !r.keywords.near.is_empty()
            }
            Step::When => r.dates.mode != DateMode::AnyTime,
            Step::Types => !r.item_types.is_empty(),
            Step::Narrow => r.describe(today()).iter().any(|(s, lines)| {
                *s == Step::Narrow && !lines.iter().any(|l| l == "No extra filters")
            }),
            Step::Review => false,
        };
        if shown && built.for_step(step).any(|n| n.level == Level::Warning) {
            (StepState::Warning, "Please check")
        } else if filled {
            (StepState::Done, "Done")
        } else if step_optional(step) {
            (StepState::Plain, "Optional")
        } else {
            (StepState::Plain, "Not started")
        }
    }

    fn nav_bar(&mut self, ui: &mut egui::Ui, built: &query::Built) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let i = self.step_index();
            if i > 0 && ui.button("← Back").clicked() {
                self.go(STEPS[i - 1]);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // The last step's main action sits where Continue does on every other step.
                if self.step == Step::Review {
                    let can_save = built.can_save();
                    let save = egui::Button::new(
                        RichText::new("Save request file…")
                            .size(18.0)
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(ACCENT)
                    .min_size(Vec2::new(0.0, 42.0));
                    if ui
                        .add_enabled(can_save, save)
                        .on_disabled_hover_text("Fix the problems listed at the top first.")
                        .clicked()
                    {
                        self.save_file(built);
                    }
                    ui.label(
                        RichText::new(if !can_save {
                            "Fix the problems at the top first."
                        } else if cfg!(target_arch = "wasm32") {
                            "Your browser will download it."
                        } else {
                            "You'll choose where to save it."
                        })
                        .weak(),
                    );
                }
                if i + 1 < STEPS.len() {
                    let label = if STEPS[i + 1] == Step::Review {
                        "Check my request →"
                    } else {
                        "Continue →"
                    };
                    if primary_button(ui, label).clicked() {
                        self.go(STEPS[i + 1]);
                    }
                    if step_optional(self.step) {
                        ui.label(RichText::new("You can skip this step.").weak());
                    }
                }
            });
        });
        ui.add_space(8.0);
    }

    fn step_page(&mut self, ui: &mut egui::Ui, built: &query::Built) {
        let i = self.step_index();
        ui.label(RichText::new(format!("Step {} of {}", i + 1, STEPS.len())).weak());
        ui.label(RichText::new(step_question(self.step)).size(26.0).strong());
        if let Some(sub) = step_subtitle(self.step) {
            ui.label(RichText::new(sub).weak());
        }
        ui.add_space(10.0);

        if let Some(tip) = kind_tip(self.req.about.kind, self.step) {
            callout(ui, Tone::Tip, "Tip for this kind of request", |ui| {
                ui.label(tip);
            });
            ui.add_space(10.0);
        }

        let pending = &mut self.pending;
        match self.step {
            Step::About => about_page(
                ui,
                &mut self.req,
                pending,
                &mut self.picks,
                &mut self.picked_note,
            ),
            Step::Whose => whose_page(ui, &mut self.req, pending),
            Step::Words => words_page(ui, &mut self.req, pending),
            Step::When => when_page(ui, &mut self.req, pending),
            Step::Types => types_page(ui, &mut self.req),
            Step::Narrow => narrow_page(ui, &mut self.req, pending),
            Step::Review => {
                self.review_page(ui, built);
                return;
            }
        }

        if self.visited.contains(&self.step) {
            let notices: Vec<_> = built.for_step(self.step).cloned().collect();
            if !notices.is_empty() {
                ui.add_space(14.0);
                notice_box(ui, &notices);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------

/// Practical advice for the kind of request, shown at the top of a step.
fn kind_tip(kind: RequestKind, step: Step) -> Option<&'static str> {
    use RequestKind as K;
    Some(match (kind, step) {
        (K::PublicData, Step::Words) => {
            "Use the requester's own words where you can. If they asked about \"the jail roof\", add jail roof as a phrase."
        }
        (K::PublicData, Step::Whose) => {
            "If the request names people or a department, add them here. If you don't know who has the records, tick Search everyone."
        }
        (K::AboutThemselves, Step::Whose) => {
            "Think about who would have written to or about this person, such as their supervisor or the department they dealt with."
        }
        (K::AboutThemselves, Step::Words) => {
            "Add the person's full name and email address as words, so items that mention them are found."
        }
        (K::LegalHold, Step::Whose) => "List everyone named in the hold notice or by the attorney.",
        (K::LegalHold, Step::Words) => {
            "Legal holds often cover everything the named people have. You can leave this page empty and set only dates."
        }
        (K::Personnel, Step::Whose) => {
            "Include the employee, their supervisor and anyone else involved in the matter."
        }
        (K::Personnel, Step::Words) => {
            "Add the employee's full name. Words about the matter itself help narrow the results."
        }
        _ => return None,
    })
}

fn about_page(
    ui: &mut egui::Ui,
    req: &mut SearchRequest,
    pending: &mut Pending,
    picks: &mut Option<Picks>,
    picked_note: &mut Option<String>,
) {
    callout(ui, Tone::Info, "How this works", |ui| {
        numbered(
            ui,
            1,
            "Answer a few questions. Skip anything you're not sure about.",
        );
        numbered(ui, 2, "Save the request file.");
        numbered(
            ui,
            3,
            "Email it to the IT Director, who runs the search and sends you the results.",
        );
        ui.label(
            RichText::new("You don't need to know anything about how the search works.").weak(),
        );
    });
    ui.add_space(12.0);

    field_label(
        ui,
        "What kind of request is this?",
        None,
        "Pick the closest one.",
    );
    // Two columns: the cards are short, and the paste box below matters more.
    let width = ((ui.available_width().min(780.0) - 12.0) / 2.0).max(240.0);
    for pair in RequestKind::ALL.chunks(2) {
        ui.horizontal_top(|ui| {
            for &kind in pair {
                let on = req.about.kind == kind;
                if choice_card(
                    ui,
                    ("kind", kind as usize),
                    on,
                    kind.label(),
                    kind.description(),
                    true,
                    width,
                ) {
                    req.about.kind = if on { RequestKind::NotSaid } else { kind };
                }
            }
        });
    }
    if req.about.kind == RequestKind::AboutThemselves {
        ui.add_space(6.0);
        subject_deadline_box(ui, req, pending);
    }
    ui.add_space(14.0);

    field_label(
        ui,
        "What was asked for?",
        None,
        "Paste the request, or describe it in your own words. It's sent to the IT Director with the file.",
    );
    let r = ui.add(
        egui::TextEdit::multiline(&mut req.about.reason)
            .id(egui::Id::unique("about.reason"))
            .hint_text("e.g. All emails between Jane Doe and the Highway Department about \"Highway 56\" since January 1, 2026")
            .desired_rows(4)
            .desired_width(620.0),
    );
    if r.changed() {
        *picks = None;
        *picked_note = None;
    }
    if !req.about.reason.trim().is_empty()
        && picks.is_none()
        && ui
            .button("Pick out email addresses, phrases and dates from this")
            .on_hover_text("Nothing is added until you say so.")
            .clicked()
    {
        *picks = Some(Picks::new(query::find_details(&req.about.reason)));
        *picked_note = None;
    }
    let mut close = false;
    if let Some(p) = picks.as_mut() {
        ui.add_space(6.0);
        callout(ui, Tone::Info, "Found in what was asked for", |ui| {
            if p.found.is_empty() {
                ui.label(
                    "Nothing to pick out: no email addresses, words in \"quotes\" or full dates. \
                     You can add these on the next pages.",
                );
                if ui.button("OK").clicked() {
                    close = true;
                }
                return;
            }
            ui.label(
                RichText::new("Untick anything you don't want, then choose Use these.").weak(),
            );
            if !p.found.emails.is_empty() {
                ui.label(RichText::new("People whose email, chats and files to search").strong());
                for (e, on) in p.found.emails.iter().zip(p.emails.iter_mut()) {
                    ui.checkbox(on, e.as_str());
                }
            }
            if !p.found.phrases.is_empty() {
                ui.label(RichText::new("Words to look for").strong());
                for (w, on) in p.found.phrases.iter().zip(p.phrases.iter_mut()) {
                    ui.checkbox(on, format!("\"{w}\""));
                }
            }
            if let (Some(&a), Some(&b)) = (p.found.dates.first(), p.found.dates.last()) {
                ui.label(RichText::new("Dates").strong());
                let text = if a == b {
                    format!("On or after {}", query::long_date(a))
                } else {
                    format!("From {} to {}", query::long_date(a), query::long_date(b))
                };
                ui.checkbox(&mut p.dates, text);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if primary_button(ui, "Use these").clicked() {
                    *picked_note = Some(p.apply(req, pending));
                    close = true;
                }
                if ui.button("Not now").clicked() {
                    close = true;
                }
            });
        });
    }
    if close {
        *picks = None;
    }
    if let Some(note) = picked_note.as_deref() {
        status_line(ui, Tone::Success, note);
    }
    ui.add_space(14.0);

    field_label(
        ui,
        "Name for this request",
        None,
        "A few words so you and the IT Director can tell requests apart. Leave it blank to use the suggestion.",
    );
    let suggested = req.display_name(today());
    let hint = if req.about.name.is_empty() {
        suggested
    } else {
        String::new()
    };
    ui.add(
        egui::TextEdit::singleline(&mut req.about.name)
            .id(egui::Id::unique("about.name"))
            .hint_text(hint)
            .desired_width(520.0),
    );
    ui.add_space(14.0);

    let a = &mut req.about;
    let tracked = !a.reference.trim().is_empty() || a.received.is_some() || a.needed_by.is_some();
    optional_section(ui, "Dates and reference number", tracked, |ui| {
        field_label(
            ui,
            "Your reference number",
            None,
            "If you log requests, the number you gave this one.",
        );
        ui.add(
            egui::TextEdit::singleline(&mut a.reference)
                .id(egui::Id::unique("about.reference"))
                .hint_text("e.g. DR-2026-041")
                .desired_width(240.0),
        );
        ui.add_space(8.0);
        // For a data subject's request the date received is asked in the deadline box above.
        if a.kind != RequestKind::AboutThemselves {
            let mut has = a.received.is_some();
            if ui
                .checkbox(&mut has, "I know when the request was received")
                .changed()
            {
                a.received = has.then(today);
            }
            if let Some(d) = a.received.as_mut() {
                ui.horizontal(|ui| {
                    ui.label("Received on");
                    date_input(ui, "about.received", d, pending);
                });
            }
        }
        ui.add_space(4.0);
        let mut has = a.needed_by.is_some();
        if ui
            .checkbox(&mut has, "There's a deadline for the results")
            .changed()
        {
            a.needed_by = has.then(|| today() + Duration::days(7));
        }
        if let Some(d) = a.needed_by.as_mut() {
            ui.horizontal(|ui| {
                ui.label("Needed by");
                date_input(ui, "about.needed_by", d, pending);
            });
        }
    });
}

/// The 10-business-day deadline for someone asking for data about themselves.
fn subject_deadline_box(ui: &mut egui::Ui, req: &mut SearchRequest, pending: &mut Pending) {
    callout(ui, Tone::Info, "Minnesota deadline", |ui| {
        ui.label(format!(
            "Minnesota law ({}) says to respond immediately if you can, or within {} business days \
             of the request. Weekends and legal holidays don't count.",
            query::SUBJECT_CITATION,
            query::SUBJECT_DAYS
        ));
        ui.add_space(4.0);
        let a = &mut req.about;
        let Some(received) = a.received.as_mut() else {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("When was the request received?").strong());
                if primary_button(ui, "Today").clicked() {
                    a.received = Some(today());
                }
                if ui.button("Another day…").clicked() {
                    a.received = Some(today());
                    pending.insert("about.received".into(), String::new());
                }
            });
            return;
        };
        ui.horizontal(|ui| {
            ui.label("Received on");
            date_input(ui, "about.received", received, pending);
        });
        let due = query::subject_deadline(*received, req.holidays);
        let late = due < today();
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if late {
                badge(ui, Tone::Warning, 20.0);
            }
            ui.label(
                RichText::new(if late {
                    format!("The deadline was {}", query::weekday_date(due))
                } else {
                    format!("Respond by {}", query::weekday_date(due))
                })
                .size(19.0)
                .strong()
                .color(if late {
                    warning_color(ui)
                } else {
                    ui.visuals().strong_text_color()
                }),
            );
        });
        if !late {
            let left = (1..)
                .take_while(|&n| query::add_business_days(today(), n, req.holidays) <= due)
                .count();
            ui.label(match left {
                0 => "That's today.".to_owned(),
                1 => "That's 1 business day from today.".to_owned(),
                n => format!("That's {n} business days from today."),
            });
        }
        let a = &mut req.about;
        if !late {
            ui.horizontal_wrapped(|ui| {
                if a.needed_by == Some(due) {
                    status_line(
                        ui,
                        Tone::Success,
                        "The IT Director will be asked for results by then.",
                    );
                } else if ui
                    .button("Ask the IT Director for results by then")
                    .clicked()
                {
                    a.needed_by = Some(due);
                    pending.remove("about.needed_by");
                }
            });
            ui.label(
                RichText::new("Leave yourself time to review the results before you respond.")
                    .weak(),
            );
        }
        ui.label(
            RichText::new(
                "Counts Minnesota's legal holidays. If your office also closes on Indigenous Peoples Day \
                 or the day after Thanksgiving, tick them in Settings (top right). This is a guide, not legal advice.",
            )
            .small()
            .weak(),
        );
    });
}

fn whose_page(ui: &mut egui::Ui, req: &mut SearchRequest, pending: &mut Pending) {
    let c = &mut req.custodians;
    ui.checkbox(&mut c.everyone, "Search everyone in the organization")
        .on_hover_text(
            "Use this when you don't know who has the records. The search will take longer.",
        );
    ui.add_space(8.0);
    ui.add_enabled_ui(!c.everyone, |ui| {
        field_label(
            ui,
            "People",
            None,
            "Type a person's work email address, or their full name if you don't know it, and press Enter. \
             Add as many people as you need.",
        );
        chips(ui, "whose.people", &mut c.people, pending, "jane.doe@example.gov or Jane Doe", None);
        ui.add_space(14.0);
        field_label(
            ui,
            "Shared mailboxes, Teams or SharePoint sites",
            Some("Optional"),
            "For example a department's shared mailbox, a Team's name or a SharePoint site address.",
        );
        chips(ui, "whose.locations", &mut c.locations, pending, "Highway Department shared mailbox", None);
    });
    ui.add_space(14.0);
    callout(ui, Tone::Info, "Good to know", |ui| {
        ui.label(
            "This list isn't part of the search file. It's included in the email to the IT \
             Director, who chooses these mailboxes and sites in Purview.",
        );
    });
}

fn words_page(ui: &mut egui::Ui, req: &mut SearchRequest, pending: &mut Pending) {
    let k = &mut req.keywords;
    field_label(
        ui,
        "Words or phrases",
        None,
        "Type a word or phrase and press Enter. A phrase like snow removal is only found as those words together, in that order.",
    );
    chips(ui, "words.words", &mut k.words, pending, "budget", None);
    ui.label(
        RichText::new(
            "Tip: put * at the end of a word to find other endings. negotiat* finds negotiate, \
             negotiation and negotiating. Capital letters don't matter.",
        )
        .weak(),
    );

    if k.words.len() > 1 {
        ui.add_space(14.0);
        field_label(ui, "Find items that contain", None, "");
        ui.radio_value(&mut k.match_all, false, "Any of these words (finds more)");
        ui.radio_value(&mut k.match_all, true, "All of these words (finds fewer)");
        let first = &k.words[0];
        let rest = k.words.len() - 1;
        let others = if rest == 1 {
            format!("\"{}\"", k.words[1])
        } else {
            "the others".to_owned()
        };
        ui.label(
            RichText::new(if k.match_all {
                format!("For example, an email that mentions \"{first}\" but not {others} will be left out.")
            } else {
                format!("For example, an email that mentions only \"{first}\" will be found.")
            })
            .weak(),
        );
    }
    if !k.words.is_empty() {
        ui.add_space(14.0);
        field_label(ui, "Where should the words appear?", None, "");
        ui.radio_value(&mut k.scope, KeywordScope::Anywhere, "Anywhere in the item");
        ui.radio_value(
            &mut k.scope,
            KeywordScope::SubjectTitle,
            "Only in an email's subject line or a file's title",
        );
    }

    ui.add_space(14.0);
    optional_section(
        ui,
        "Leave out items containing certain words",
        !k.none.is_empty(),
        |ui| {
            field_label(
                ui,
                "Leave out anything containing",
                None,
                "Use this for things like newsletters or automatic notices.",
            );
            chips(ui, "words.none", &mut k.none, pending, "newsletter", None);
        },
    );
    optional_section(
        ui,
        "Find words that are close together",
        !k.near.is_empty(),
        |ui| {
            ui.label(RichText::new("For example, payment within 10 words of late.").weak());
            let mut remove = None;
            for (i, n) in k.near.iter_mut().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut n.first)
                            .id(egui::Id::unique(("near.first", i)))
                            .hint_text("word")
                            .desired_width(150.0),
                    );
                    ui.label("within");
                    ui.add(egui::DragValue::new(&mut n.distance).range(1..=100));
                    ui.label("words of");
                    ui.add(
                        egui::TextEdit::singleline(&mut n.second)
                            .id(egui::Id::unique(("near.second", i)))
                            .hint_text("word")
                            .desired_width(150.0),
                    );
                    if ui.button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                k.near.remove(i);
            }
            if ui.button("+ Add a pair of words").clicked() {
                k.near.push(Near::default());
            }
        },
    );
}

fn when_page(ui: &mut egui::Ui, req: &mut SearchRequest, pending: &mut Pending) {
    let d = &mut req.dates;
    let now = today();
    for mode in DateMode::ALL {
        ui.horizontal(|ui| {
            ui.radio_value(&mut d.mode, mode, mode.label());
            if let Some((Some(a), Some(b))) = mode.range(d.start, d.end, now)
                && !matches!(mode, DateMode::Between)
            {
                ui.label(
                    RichText::new(format!("{} – {}", query::long_date(a), query::long_date(b)))
                        .weak(),
                );
            }
        });
        if mode == d.mode {
            let indent = |ui: &mut egui::Ui, f: &mut dyn FnMut(&mut egui::Ui)| {
                ui.horizontal(|ui| {
                    ui.add_space(30.0);
                    ui.vertical(|ui| f(ui));
                });
            };
            match mode {
                DateMode::Between => indent(ui, &mut |ui| {
                    ui.horizontal(|ui| {
                        ui.add_sized([50.0, 30.0], egui::Label::new("From"));
                        date_input(ui, "when.start", &mut d.start, pending);
                    });
                    ui.horizontal(|ui| {
                        ui.add_sized([50.0, 30.0], egui::Label::new("To"));
                        date_input(ui, "when.end", &mut d.end, pending);
                    });
                }),
                DateMode::OnOrAfter => indent(ui, &mut |ui| {
                    ui.horizontal(|ui| {
                        ui.add_sized([50.0, 30.0], egui::Label::new("From"));
                        date_input(ui, "when.start", &mut d.start, pending);
                    });
                }),
                DateMode::OnOrBefore => indent(ui, &mut |ui| {
                    ui.horizontal(|ui| {
                        ui.add_sized([50.0, 30.0], egui::Label::new("Up to"));
                        date_input(ui, "when.end", &mut d.end, pending);
                    });
                }),
                _ => {}
            }
        }
    }
    if d.mode != DateMode::AnyTime {
        ui.add_space(14.0);
        field_label(
            ui,
            "Which date should be used?",
            None,
            "Most requests can leave this as it is.",
        );
        egui::ComboBox::from_id_salt("date_field")
            .selected_text(d.field.label())
            .width(420.0)
            .show_ui(ui, |ui| {
                for f in DateField::ALL {
                    ui.selectable_value(&mut d.field, f, f.label());
                }
            });
    }
}

fn types_page(ui: &mut egui::Ui, req: &mut SearchRequest) {
    for t in query::ITEM_TYPES.iter().filter(|t| t.common) {
        type_card(ui, &mut req.item_types, t);
    }
    let more = query::ITEM_TYPES
        .iter()
        .any(|t| !t.common && req.item_types.iter().any(|l| l == t.label));
    ui.add_space(6.0);
    optional_section(ui, "More types", more, |ui| {
        for t in query::ITEM_TYPES.iter().filter(|t| !t.common) {
            let mut on = req.item_types.iter().any(|l| l == t.label);
            if ui
                .checkbox(&mut on, t.label)
                .on_hover_text(t.description)
                .changed()
            {
                toggle(&mut req.item_types, t.label, on);
            }
        }
    });
}

fn type_card(ui: &mut egui::Ui, selected: &mut Vec<String>, t: &query::ItemType) {
    let on = selected.iter().any(|l| l == t.label);
    if choice_card(
        ui,
        ("type_card", t.label),
        on,
        t.label,
        t.description,
        false,
        490.0,
    ) {
        toggle(selected, t.label, !on);
    }
}

/// A clickable card with a tick box (or a round radio mark), a title and a description.
/// Returns true when it was clicked.
fn choice_card(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    on: bool,
    title: &str,
    description: &str,
    radio: bool,
    width: f32,
) -> bool {
    let visuals = ui.visuals().clone();
    let id = ui.scope_id().with(&id_salt);
    let hovered = ui.ctx().read_response(id).is_some_and(|r| r.hovered());
    let stroke = if on {
        Stroke::new(2.0, visuals.selection.stroke.color)
    } else if hovered {
        Stroke::new(1.5, visuals.selection.stroke.color.gamma_multiply(0.8))
    } else {
        Stroke::new(
            1.0,
            visuals.widgets.inactive.bg_stroke.color.gamma_multiply(2.0),
        )
    };
    let fill = if on {
        visuals.selection.bg_fill.gamma_multiply(0.25)
    } else {
        visuals.extreme_bg_color
    };
    let frame = egui::Frame::new()
        .stroke(stroke)
        .fill(fill)
        .corner_radius(8)
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            // The frame's margin and stroke sit outside this width.
            ui.set_width(width - 30.0);
            ui.horizontal(|ui| {
                // Drawn for show; the whole card handles the click.
                let mut shown = on;
                if radio {
                    ui.add(egui::RadioButton::new(shown, ""));
                } else {
                    ui.add(egui::Checkbox::without_text(&mut shown));
                }
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).strong());
                    if !description.is_empty() {
                        ui.label(RichText::new(description).weak());
                    }
                });
            });
        });
    let resp = ui
        .interact(frame.response.rect, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(
            if radio {
                egui::Role::RadioButton
            } else {
                egui::Role::CheckBox
            },
            true,
            on,
            title,
        )
    });
    if resp.has_focus() {
        ui.painter().rect_stroke(
            frame.response.rect.expand(2.0),
            8,
            visuals.selection.stroke,
            egui::StrokeKind::Outside,
        );
    }
    ui.add_space(4.0);
    resp.clicked()
}

fn narrow_page(ui: &mut egui::Ui, req: &mut SearchRequest, pending: &mut Pending) {
    let f = &mut req.filters;

    let people = !f.sent_by.is_empty() || !f.sent_to.is_empty() || !f.leave_out_people.is_empty();
    optional_section(ui, "Who sent or received it", people, |ui| {
        field_label(ui, "Sent by", None, "Only items sent by these people.");
        chips(
            ui,
            "narrow.sent_by",
            &mut f.sent_by,
            pending,
            "name@example.gov",
            Some(query::looks_like_address),
        );
        field_label(
            ui,
            "Sent to",
            None,
            "Only items sent to these people (To, CC or BCC).",
        );
        chips(
            ui,
            "narrow.sent_to",
            &mut f.sent_to,
            pending,
            "name@example.gov",
            Some(query::looks_like_address),
        );
        field_label(
            ui,
            "Leave out anything sent by or to",
            None,
            "For example automatic senders like noreply addresses.",
        );
        chips(
            ui,
            "narrow.leave_out",
            &mut f.leave_out_people,
            pending,
            "noreply@example.gov",
            Some(query::looks_like_address),
        );
    });

    let att = f.attachments != Attachments::Any || !f.attachment_names.is_empty();
    optional_section(ui, "Attachments", att, |ui| {
        ui.radio_value(&mut f.attachments, Attachments::Any, "Doesn't matter");
        ui.radio_value(
            &mut f.attachments,
            Attachments::With,
            "Only items with attachments",
        );
        ui.radio_value(
            &mut f.attachments,
            Attachments::Without,
            "Only items without attachments",
        );
        field_label(
            ui,
            "Attachment name",
            None,
            "Only items with an attachment of this name. A * at the end matches the rest of the name.",
        );
        chips(
            ui,
            "narrow.att_names",
            &mut f.attachment_names,
            pending,
            "invoice*",
            None,
        );
    });

    let files = !f.file_types.is_empty()
        || !f.other_extensions.is_empty()
        || !f.file_names.is_empty()
        || !f.authors.is_empty();
    optional_section(ui, "Files", files, |ui| {
        field_label(ui, "File types", None, "Only these types of files.");
        egui::Grid::new("file_types")
            .num_columns(3)
            .spacing([24.0, 6.0])
            .show(ui, |ui| {
                for (i, (_, label)) in query::FILE_TYPES.iter().enumerate() {
                    let mut on = f.file_types.iter().any(|l| l == label);
                    if ui.checkbox(&mut on, *label).changed() {
                        toggle(&mut f.file_types, label, on);
                    }
                    if i % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
        field_label(
            ui,
            "Other file types",
            None,
            "Type the ending of the file name, for example dwg or kml.",
        );
        chips(
            ui,
            "narrow.other_ext",
            &mut f.other_extensions,
            pending,
            "dwg",
            None,
        );
        field_label(
            ui,
            "File name",
            None,
            "A * at the end matches the rest of the name.",
        );
        chips(
            ui,
            "narrow.file_names",
            &mut f.file_names,
            pending,
            "budget*",
            None,
        );
        field_label(ui, "Written by", None, "The author recorded in the file.");
        chips(
            ui,
            "narrow.authors",
            &mut f.authors,
            pending,
            "Jane Doe",
            None,
        );
    });

    optional_section(ui, "Importance", f.high_importance, |ui| {
        ui.checkbox(&mut f.high_importance, "Only emails marked high importance");
    });

    optional_section(ui, "Size", f.size_op != SizeOp::Any, |ui| {
        ui.radio_value(&mut f.size_op, SizeOp::Any, "Any size");
        ui.horizontal(|ui| {
            ui.radio_value(&mut f.size_op, SizeOp::LargerThan, "Larger than");
            ui.radio_value(&mut f.size_op, SizeOp::SmallerThan, "Smaller than");
            if f.size_op != SizeOp::Any {
                ui.add(
                    egui::DragValue::new(&mut f.size_mb)
                        .range(0.0..=100_000.0)
                        .speed(0.5)
                        .suffix(" MB"),
                );
            }
        });
    });

    optional_section(
        ui,
        "Sensitive information (files only)",
        !f.sensitive.is_empty(),
        |ui| {
            ui.label(
            RichText::new(
                "Finds OneDrive and SharePoint files that contain these. Emails and Teams messages \
                 can't be searched this way.",
            )
            .weak(),
        );
            for (name, label) in query::SENSITIVE_TYPES {
                let mut on = f.sensitive.iter().any(|s| s == name);
                if ui.checkbox(&mut on, *label).changed() {
                    toggle(&mut f.sensitive, name, on);
                }
            }
        },
    );
}

impl KeyqlApp {
    fn review_page(&mut self, ui: &mut egui::Ui, built: &query::Built) {
        flush_pending(&mut self.req, &mut self.pending);
        let errors: Vec<_> = built
            .notices
            .iter()
            .filter(|n| n.level == Level::Error)
            .cloned()
            .collect();
        let warnings: Vec<_> = built
            .notices
            .iter()
            .filter(|n| n.level == Level::Warning)
            .cloned()
            .collect();
        let mut target = None;

        if !errors.is_empty() {
            problem_box(
                ui,
                Level::Error,
                "Fix these before saving",
                &errors,
                &mut target,
            );
            ui.add_space(10.0);
        }
        if !warnings.is_empty() {
            problem_box(
                ui,
                Level::Warning,
                "Please check these",
                &warnings,
                &mut target,
            );
            ui.add_space(10.0);
        }
        ui.label("Make sure everything below is right. Use Change to go back and edit a section.");
        ui.add_space(6.0);

        for (step, lines) in self.req.describe(today()) {
            egui::Frame::new()
                .fill(ui.visuals().extreme_bg_color)
                .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                .corner_radius(8)
                .inner_margin(Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(step_title(step)).strong());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .link("Change")
                                .on_hover_text(format!(
                                    "Change {}",
                                    step_title(step).to_lowercase()
                                ))
                                .clicked()
                            {
                                target = Some(step);
                            }
                        });
                    });
                    for l in lines {
                        ui.label(l);
                    }
                });
            ui.add_space(4.0);
        }

        let infos: Vec<_> = built
            .notices
            .iter()
            .filter(|n| n.level == Level::Info)
            .cloned()
            .collect();
        if !infos.is_empty() {
            ui.add_space(6.0);
            notice_box(ui, &infos);
        }

        ui.add_space(16.0);
        ui.label(
            RichText::new(
                "When everything looks right, choose Save request file at the bottom right. \
                 Next, you'll email the file to the IT Director.",
            )
            .weak(),
        );
        if let Some(e) = &self.save_error {
            ui.add_space(6.0);
            status_line(ui, Tone::Error, format!("The file couldn't be saved: {e}"));
        }

        if let Some(step) = target {
            self.go(step);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save_file(&mut self, built: &query::Built) {
        let name = self.req.file_name(today());
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save search request")
            .set_file_name(&name)
            .add_filter("Purview search (.kql)", &["kql"]);
        if let Some(dir) = documents_dir() {
            dialog = dialog.set_directory(dir);
        }
        // Debug builds can skip the dialog so automated UI tests never open one.
        let test_dir = cfg!(debug_assertions)
            .then(|| std::env::var_os("KEYQL_TEST_SAVE_DIR"))
            .flatten();
        let chosen = match test_dir {
            Some(dir) => Some(PathBuf::from(dir).join(&name)),
            None => dialog.save_file(),
        };
        let Some(mut path) = chosen else {
            return;
        };
        if path
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("kql"))
        {
            path.set_extension("kql");
        }
        match std::fs::write(&path, &built.query) {
            Ok(()) => {
                self.saved = Some(path);
                self.save_error = None;
                self.record_note = None;
            }
            Err(e) => self.save_error = Some(e.to_string()),
        }
    }

    /// In the browser there's no save dialog: the file is downloaded and lands
    /// wherever the browser keeps downloads.
    #[cfg(target_arch = "wasm32")]
    fn save_file(&mut self, built: &query::Built) {
        let name = self.req.file_name(today());
        match download(&name, &built.query) {
            Ok(()) => {
                self.saved = Some(PathBuf::from(name));
                self.save_error = None;
                self.record_note = None;
            }
            Err(e) => self.save_error = Some(e),
        }
    }

    fn record_name(&self) -> String {
        self.req
            .file_name(today())
            .trim_end_matches(".kql")
            .to_owned()
            + " - record.txt"
    }

    fn record_text(&self, kql_file: &str) -> String {
        self.req.record(kql_file, &self.req.build().query, today())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save_record(&mut self, kql_file: &str, dir: Option<&std::path::Path>) {
        let name = self.record_name();
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save a copy for your records")
            .set_file_name(&name)
            .add_filter("Text file (.txt)", &["txt"]);
        if let Some(dir) = dir {
            dialog = dialog.set_directory(dir);
        }
        let test_dir = cfg!(debug_assertions)
            .then(|| std::env::var_os("KEYQL_TEST_SAVE_DIR"))
            .flatten();
        let chosen = match test_dir {
            Some(dir) => Some(PathBuf::from(dir).join(&name)),
            None => dialog.save_file(),
        };
        let Some(path) = chosen else {
            return;
        };
        self.record_note = Some(
            std::fs::write(&path, self.record_text(kql_file))
                .map(|()| format!("Saved {}", path.display()))
                .map_err(|e| e.to_string()),
        );
    }

    #[cfg(target_arch = "wasm32")]
    fn save_record(&mut self, kql_file: &str, _dir: Option<&std::path::Path>) {
        let name = self.record_name();
        self.record_note = Some(
            download(&name, &self.record_text(kql_file)).map(|()| format!("Downloaded {name}")),
        );
    }

    fn done_page(&mut self, ui: &mut egui::Ui) {
        let Some(path) = self.saved.clone() else {
            return;
        };
        let file = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let folder = if cfg!(target_arch = "wasm32") {
            "your browser's Downloads folder".to_owned()
        } else {
            path.parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        };

        let green = success_color(ui);
        egui::Frame::new()
            .fill(green.gamma_multiply(0.12))
            .stroke(Stroke::new(2.0, green))
            .corner_radius(10)
            .inner_margin(Margin::same(20))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    badge(ui, Tone::Success, 30.0);
                    ui.label(
                        RichText::new("Your request file is saved")
                            .size(26.0)
                            .strong()
                            .color(ui.visuals().strong_text_color()),
                    );
                });
                ui.add_space(4.0);
                ui.label(RichText::new(&file).strong());
                ui.label(RichText::new(format!("in {folder}")).weak());
            });
        ui.add_space(20.0);
        ui.label(RichText::new("What to do next").size(20.0).strong());
        ui.add_space(4.0);
        numbered(ui, 1, "Email the file to the IT Director as an attachment.");
        numbered(
            ui,
            2,
            "Add the email text, which says whose content to search and what was asked for. \
             Write the email does this for you.",
        );
        numbered(
            ui,
            3,
            "The IT Director runs the search and gets back to you with the results.",
        );
        ui.add_space(12.0);

        let (subject, body) = self.req.email(&file, today());
        ui.horizontal_wrapped(|ui| {
            if primary_button(ui, "Write the email")
                .on_hover_text("Opens your email program. Remember to attach the file.")
                .clicked()
            {
                let to = self.settings.it_director_email.trim();
                open_link(ui.ctx(), &mailto(to, &subject, &body), false);
            }
            #[cfg(not(target_arch = "wasm32"))]
            if ui.button("Show the file in its folder").clicked()
                && let Some(dir) = path.parent()
            {
                let _ = open::that(dir);
            }
            let now = ui.input(|i| i.time);
            let copied = self.copied_at.is_some_and(|t| now - t < 2.0);
            if ui
                .button(if copied { "Copied" } else { "Copy email text" })
                .clicked()
            {
                ui.ctx().copy_text(format!("Subject: {subject}\n\n{body}"));
                self.copied_at = Some(now);
            }
            if copied {
                ui.ctx().request_repaint();
            }
        });
        if self.settings.it_director_email.trim().is_empty() {
            ui.label(
                RichText::new("Tip: add the IT Director's email address in Settings (top right) to fill it in automatically.")
                    .weak(),
            );
        }
        ui.add_space(14.0);
        ui.label(RichText::new("For your files").size(17.0).strong());
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Save a copy for my records")
                .on_hover_text(
                    "A text file with what was asked for, the summary and the exact search.",
                )
                .clicked()
            {
                self.save_record(&file, path.parent());
            }
            match &self.record_note {
                Some(Ok(m)) => status_line(ui, Tone::Success, m.as_str()),
                Some(Err(e)) => status_line(ui, Tone::Error, format!("Couldn't save it: {e}")),
                None => {}
            }
        });
        ui.add_space(10.0);
        egui::CollapsingHeader::new("Show the email text").show(ui, |ui| {
            let mut text = format!("Subject: {subject}\n\n{body}");
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
        });
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.button("← Back to my request").clicked() {
                self.saved = None;
            }
            if ui.button("Start a new request").clicked() {
                self.reset();
            }
        });
    }

    fn reset_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_reset {
            return;
        }
        let resp = egui::Modal::new(egui::Id::unique("confirm_reset")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(RichText::new("Start a new request?").size(20.0).strong());
            ui.label("Everything you've entered for this request will be cleared. Files you've already saved are not affected.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if primary_button(ui, "Clear and start again").clicked() {
                    self.reset();
                }
                if ui.button("Keep my answers").clicked() {
                    self.confirm_reset = false;
                }
            });
        });
        if resp.should_close() {
            self.confirm_reset = false;
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        let mut close = false;
        let frame = egui::Frame::window(&ctx.style_of(ctx.theme())).inner_margin(Margin::same(18));
        egui::Window::new("Settings")
            .frame(frame)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
            .default_width(470.0)
            .show(ctx, |ui| {
                field_label(
                    ui,
                    "IT Director's email address",
                    None,
                    "Used to fill in the To line when you write the email.",
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.it_director_email)
                        .hint_text("it.director@example.gov")
                        .desired_width(360.0),
                );
                ui.add_space(10.0);
                field_label(
                    ui,
                    "Office holidays",
                    None,
                    "Minnesota lets each office choose whether these are holidays. Tick the ones yours \
                     closes for, and they're skipped when counting the 10 business days.",
                );
                ui.checkbox(
                    &mut self.settings.holidays.indigenous_peoples_day,
                    "Indigenous Peoples Day (second Monday in October)",
                );
                ui.checkbox(
                    &mut self.settings.holidays.day_after_thanksgiving,
                    "The Friday after Thanksgiving",
                );
                ui.add_space(10.0);
                field_label(ui, "Text size", None, "");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.settings.text_size, TextSize::Standard, "Standard");
                    ui.radio_value(&mut self.settings.text_size, TextSize::Large, "Large");
                    ui.radio_value(
                        &mut self.settings.text_size,
                        TextSize::ExtraLarge,
                        "Extra large",
                    );
                });
                ui.add_space(10.0);
                field_label(ui, "Colors", None, "");
                ui.horizontal(|ui| {
                    ui.radio_value(
                        &mut self.settings.theme,
                        ThemeChoice::System,
                        "Match my computer",
                    );
                    ui.radio_value(&mut self.settings.theme, ThemeChoice::Light, "Light");
                    ui.radio_value(&mut self.settings.theme, ThemeChoice::Dark, "Dark");
                });
                ui.add_space(14.0);
                if primary_button(ui, "Done").clicked() {
                    close = true;
                }
            });
        self.show_settings = open && !close;
    }
}

// ---------------------------------------------------------------------------
// Widgets
// ---------------------------------------------------------------------------

/// What a message is about. Sets its color and the drawn badge beside it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tone {
    Info,
    Tip,
    Success,
    Warning,
    Error,
}

impl From<Level> for Tone {
    fn from(l: Level) -> Self {
        match l {
            Level::Error => Tone::Error,
            Level::Warning => Tone::Warning,
            Level::Info => Tone::Info,
        }
    }
}

const ACCENT: Color32 = Color32::from_rgb(30, 100, 200);

fn tone_color(ui: &egui::Ui, tone: Tone) -> Color32 {
    let dark = ui.visuals().dark_mode;
    match tone {
        Tone::Info if dark => Color32::from_rgb(120, 170, 245),
        Tone::Info => ACCENT,
        Tone::Tip if dark => Color32::from_rgb(230, 180, 90),
        Tone::Tip => Color32::from_rgb(160, 100, 0),
        Tone::Success => success_color(ui),
        Tone::Warning => warning_color(ui),
        Tone::Error => error_color(ui),
    }
}

/// A round badge drawn with shapes (no emoji): i, !, a tick or a cross.
fn paint_badge(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    tone: Tone,
    color: Color32,
) {
    painter.circle_filled(center, radius, color);
    let ink = Color32::WHITE;
    let w = (radius * 0.2).max(1.6);
    match tone {
        Tone::Success => {
            let r = radius;
            painter.line(
                vec![
                    center + Vec2::new(-0.45 * r, 0.02 * r),
                    center + Vec2::new(-0.12 * r, 0.36 * r),
                    center + Vec2::new(0.48 * r, -0.32 * r),
                ],
                Stroke::new(w, ink),
            );
        }
        Tone::Error => {
            let d = radius * 0.38;
            painter.line_segment(
                [center + Vec2::new(-d, -d), center + Vec2::new(d, d)],
                Stroke::new(w, ink),
            );
            painter.line_segment(
                [center + Vec2::new(-d, d), center + Vec2::new(d, -d)],
                Stroke::new(w, ink),
            );
        }
        Tone::Warning => {
            painter.line_segment(
                [
                    center + Vec2::new(0.0, -0.5 * radius),
                    center + Vec2::new(0.0, 0.12 * radius),
                ],
                Stroke::new(w, ink),
            );
            painter.circle_filled(center + Vec2::new(0.0, 0.45 * radius), w * 0.6, ink);
        }
        Tone::Info | Tone::Tip => {
            painter.circle_filled(center + Vec2::new(0.0, -0.45 * radius), w * 0.6, ink);
            painter.line_segment(
                [
                    center + Vec2::new(0.0, -0.12 * radius),
                    center + Vec2::new(0.0, 0.5 * radius),
                ],
                Stroke::new(w, ink),
            );
        }
    }
}

fn badge(ui: &mut egui::Ui, tone: Tone, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_badge(
        ui.painter(),
        rect.center(),
        size / 2.0,
        tone,
        tone_color(ui, tone),
    );
}

/// A badge and a line of text in the tone's color, wrapping as needed.
fn status_line(ui: &mut egui::Ui, tone: Tone, text: impl Into<String>) {
    let color = tone_color(ui, tone);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add_space(1.0);
        ui.vertical(|ui| {
            ui.add_space(2.0);
            badge(ui, tone, 16.0);
        });
        let text: String = text.into();
        ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StepState {
    Plain,
    Done,
    Warning,
    Error,
}

/// One row of the step list: a numbered circle (or a drawn tick), the title and a status line.
fn step_item(
    ui: &mut egui::Ui,
    number: usize,
    title: &str,
    state: StepState,
    status: &str,
    current: bool,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 54.0), Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            true,
            format!("Step {number}: {title}. {status}"),
        )
    });
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let v = ui.visuals().clone();
    let painter = ui.painter();
    if current {
        painter.rect_filled(rect, 6, v.selection.bg_fill.gamma_multiply(0.35));
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, Vec2::new(4.0, rect.height())),
            2,
            ACCENT,
        );
    } else if resp.hovered() {
        painter.rect_filled(rect, 6, v.widgets.hovered.weak_bg_fill);
    }
    if resp.has_focus() {
        painter.rect_stroke(rect, 6, v.selection.stroke, egui::StrokeKind::Inside);
    }

    let center = rect.left_center() + Vec2::new(24.0, 0.0);
    if state == StepState::Done && !current {
        paint_badge(painter, center, 13.0, Tone::Success, success_color(ui));
    } else {
        let (fill, ink) = if current {
            (ACCENT, Color32::WHITE)
        } else {
            (Color32::TRANSPARENT, v.text_color())
        };
        painter.circle(
            center,
            13.0,
            fill,
            Stroke::new(1.5, if current { ACCENT } else { v.weak_text_color() }),
        );
        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            number.to_string(),
            FontId::proportional(14.0),
            ink,
        );
    }

    let x = rect.left() + 48.0;
    painter.text(
        egui::pos2(x, rect.top() + 8.0),
        egui::Align2::LEFT_TOP,
        title,
        FontId::proportional(16.0),
        if current {
            v.strong_text_color()
        } else {
            v.text_color()
        },
    );
    let (tone, color) = match state {
        StepState::Error => (Some(Tone::Error), error_color(ui)),
        StepState::Warning => (Some(Tone::Warning), warning_color(ui)),
        _ => (None, v.weak_text_color()),
    };
    let mut sx = x;
    if let Some(tone) = tone {
        paint_badge(
            painter,
            egui::pos2(x + 6.0, rect.top() + 38.0),
            6.5,
            tone,
            color,
        );
        sx += 17.0;
    }
    painter.text(
        egui::pos2(sx, rect.top() + 30.0),
        egui::Align2::LEFT_TOP,
        status,
        FontId::proportional(13.5),
        color,
    );
    resp
}

fn error_color(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(255, 120, 110)
    } else {
        Color32::from_rgb(180, 30, 25)
    }
}

fn warning_color(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(245, 190, 80)
    } else {
        Color32::from_rgb(150, 90, 0)
    }
}

fn success_color(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(110, 200, 130)
    } else {
        Color32::from_rgb(20, 120, 50)
    }
}

fn primary(text: RichText) -> egui::Button<'static> {
    egui::Button::new(text.strong().color(Color32::WHITE))
        .fill(ACCENT)
        .min_size(Vec2::new(0.0, 36.0))
}

fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(primary(RichText::new(label)))
}

/// Bold label, an optional "Required"/"Optional" tag, and hint text below.
fn field_label(ui: &mut egui::Ui, label: &str, tag: Option<&str>, hint: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(17.0).strong());
        if let Some(tag) = tag {
            let text = RichText::new(format!("({})", tag.to_lowercase()));
            ui.label(if tag == "Required" {
                text.strong()
            } else {
                text.weak()
            });
        }
    });
    if !hint.is_empty() {
        ui.label(RichText::new(hint).weak());
    }
}

/// The page-and-magnifier mark from the loading screen, drawn with shapes.
fn app_mark(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let p = ui.painter();
    let u = size / 76.0; // the loading screen's SVG is 76 units wide
    let at = |x: f32, y: f32| rect.min + Vec2::new(x * u, y * u);
    let ink = ui.visuals().weak_text_color();
    let accent = tone_color(ui, Tone::Info);
    p.rect(
        egui::Rect::from_min_max(at(12.0, 8.0), at(52.0, 60.0)),
        5.0 * u,
        ui.visuals().extreme_bg_color,
        Stroke::new(2.5 * u, ink),
        egui::StrokeKind::Middle,
    );
    for (y, w) in [(19.0, 26.0), (28.0, 20.0), (37.0, 24.0)] {
        p.rect_filled(
            egui::Rect::from_min_size(at(19.0, y), Vec2::new(w, 3.5) * u),
            1.75 * u,
            ink,
        );
    }
    p.circle(
        at(42.0, 40.0),
        12.0 * u,
        accent.gamma_multiply(0.25),
        Stroke::new(3.5 * u, accent),
    );
    p.line_segment(
        [at(51.0, 49.0), at(62.0, 60.0)],
        Stroke::new(5.0 * u, accent),
    );
}

/// "1  Text" with the number in a small circle, for short lists of steps.
fn numbered(ui: &mut egui::Ui, n: usize, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
        let color = ui.visuals().weak_text_color();
        ui.painter()
            .circle_stroke(rect.center(), 10.0, Stroke::new(1.3, color));
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            n.to_string(),
            FontId::proportional(13.0),
            ui.visuals().text_color(),
        );
        ui.add(egui::Label::new(text).wrap());
    });
}

/// A tinted box with a colored edge, a badge and a title.
fn callout(ui: &mut egui::Ui, tone: Tone, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    let color = tone_color(ui, tone);
    let tint = if ui.visuals().dark_mode { 0.10 } else { 0.07 };
    let inner = egui::Frame::new()
        .fill(color.gamma_multiply(tint))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.35)))
        .corner_radius(8)
        .inner_margin(Margin {
            left: 20,
            right: 14,
            top: 10,
            bottom: 12,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                badge(ui, tone, 18.0);
                ui.label(
                    RichText::new(title)
                        .strong()
                        .color(ui.visuals().strong_text_color()),
                );
            });
            ui.add_space(2.0);
            body(ui);
        });
    let r = inner.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_size(r.min, Vec2::new(4.0, r.height())),
        egui::CornerRadius {
            nw: 8,
            sw: 8,
            ne: 0,
            se: 0,
        },
        color,
    );
}

fn optional_section(
    ui: &mut egui::Ui,
    title: &str,
    has_content: bool,
    body: impl FnOnce(&mut egui::Ui),
) {
    let mut job = egui::text::LayoutJob::default();
    let body_font = FontId::proportional(16.0);
    job.append(
        title,
        0.0,
        egui::TextFormat::simple(body_font.clone(), ui.visuals().strong_text_color()),
    );
    if has_content {
        job.append(
            "filled in",
            10.0,
            egui::TextFormat::simple(FontId::proportional(13.5), success_color(ui)),
        );
    }
    egui::CollapsingHeader::new(job)
        .id_salt(title)
        .default_open(has_content)
        .show(ui, |ui| {
            ui.add_space(4.0);
            body(ui);
            ui.add_space(8.0);
        });
}

fn notice_box(ui: &mut egui::Ui, notices: &[query::Notice]) {
    for n in notices {
        let tone = Tone::from(n.level);
        if tone == Tone::Info {
            // Notes read as plain text with a badge; only problems take a color.
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    badge(ui, tone, 16.0);
                });
                ui.add(egui::Label::new(&n.message).wrap());
            });
        } else {
            status_line(ui, tone, &n.message);
        }
    }
}

/// The error summary on the check page: each problem is a link to the step that fixes it.
fn problem_box(
    ui: &mut egui::Ui,
    level: Level,
    title: &str,
    notices: &[query::Notice],
    target: &mut Option<Step>,
) {
    let tone = Tone::from(level);
    let color = tone_color(ui, tone);
    egui::Frame::new()
        .stroke(Stroke::new(3.0, color))
        .fill(color.gamma_multiply(0.05))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                badge(ui, tone, 20.0);
                ui.label(RichText::new(title).size(18.0).strong().color(color));
            });
            ui.add_space(2.0);
            for n in notices {
                if n.step == Step::Review {
                    ui.label(format!("• {}", n.message));
                } else if ui
                    .link(format!("• {}", n.message))
                    .on_hover_text(format!("Go to {}", step_title(n.step)))
                    .clicked()
                {
                    *target = Some(n.step);
                }
            }
        });
}

fn toggle(list: &mut Vec<String>, value: &str, on: bool) {
    if on {
        if !list.iter().any(|v| v == value) {
            list.push(value.to_owned());
        }
    } else {
        list.retain(|v| v != value);
    }
}

/// Separators that split typed or pasted text into several chips.
fn separators(key: &str) -> &'static [char] {
    if key.starts_with("words.") {
        &[';', '\n']
    } else {
        &[',', ';', '\n']
    }
}

fn add_entries(items: &mut Vec<String>, text: &str, seps: &[char]) {
    for part in text.split(|c| seps.contains(&c)) {
        let e = query::clean_entry(part);
        if !e.is_empty() && !items.iter().any(|i| i.eq_ignore_ascii_case(&e)) {
            items.push(e);
        }
    }
}

/// The list a chip field writes to, for committing text left in the box.
fn chip_list<'a>(req: &'a mut SearchRequest, key: &str) -> Option<&'a mut Vec<String>> {
    Some(match key {
        "whose.people" => &mut req.custodians.people,
        "whose.locations" => &mut req.custodians.locations,
        "words.words" => &mut req.keywords.words,
        "words.none" => &mut req.keywords.none,
        "narrow.sent_by" => &mut req.filters.sent_by,
        "narrow.sent_to" => &mut req.filters.sent_to,
        "narrow.leave_out" => &mut req.filters.leave_out_people,
        "narrow.att_names" => &mut req.filters.attachment_names,
        "narrow.other_ext" => &mut req.filters.other_extensions,
        "narrow.file_names" => &mut req.filters.file_names,
        "narrow.authors" => &mut req.filters.authors,
        _ => return None,
    })
}

/// Turn anything still typed in a chip box into chips, so leaving a page never loses it.
fn flush_pending(req: &mut SearchRequest, pending: &mut Pending) {
    pending.retain(|key, text| match chip_list(req, key) {
        Some(list) => {
            add_entries(list, text, separators(key));
            false
        }
        None => true,
    });
}

/// A list of removable chips plus a box to add more (Enter, Add, a separator, or leaving the box).
fn chips(
    ui: &mut egui::Ui,
    key: &str,
    items: &mut Vec<String>,
    pending: &mut Pending,
    example: &str,
    check: Option<fn(&str) -> bool>,
) {
    let seps = separators(key);
    let mut remove = None;
    if !items.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for (i, item) in items.iter().enumerate() {
                let bad = check.is_some_and(|f| !f(item));
                let (fill, stroke) = if bad {
                    (
                        warning_color(ui).gamma_multiply(0.10),
                        Stroke::new(1.5, warning_color(ui)),
                    )
                } else {
                    (
                        ACCENT.gamma_multiply(if ui.visuals().dark_mode { 0.22 } else { 0.08 }),
                        Stroke::new(1.0, ACCENT.gamma_multiply(0.45)),
                    )
                };
                egui::Frame::new()
                    .fill(fill)
                    .stroke(stroke)
                    .corner_radius(16)
                    .inner_margin(Margin {
                        left: 12,
                        right: 4,
                        top: 3,
                        bottom: 3,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if bad {
                                badge(ui, Tone::Warning, 14.0);
                            }
                            ui.label(RichText::new(item).color(ui.visuals().strong_text_color()));
                            let x = egui::Button::new(RichText::new("×").size(18.0))
                                .frame_when_inactive(false)
                                .corner_radius(12)
                                .min_size(Vec2::splat(24.0));
                            if ui.add(x).on_hover_text(format!("Remove {item}")).clicked() {
                                remove = Some(i);
                            }
                        });
                    });
            }
        });
    }
    if let Some(i) = remove {
        items.remove(i);
    }
    if let Some(f) = check {
        let bad: Vec<&str> = items.iter().filter(|i| !f(i)).map(String::as_str).collect();
        if !bad.is_empty() {
            status_line(
                ui,
                Tone::Warning,
                format!(
                    "{} {} not a full email address. Names can match the wrong person.",
                    bad.join(", "),
                    if bad.len() == 1 { "is" } else { "are" }
                ),
            );
        }
    }

    let text = pending.entry(key.to_owned()).or_default();
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(text)
                .id(egui::Id::unique(("chip_input", key)))
                .hint_text(format!("e.g. {example}"))
                .desired_width(380.0),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let add = ui
            .add_enabled(!text.trim().is_empty(), egui::Button::new("Add"))
            .clicked();
        if enter || add || r.lost_focus() {
            add_entries(items, text, seps);
            text.clear();
            if enter || add {
                r.request_focus();
            }
        } else if let Some(pos) = text.rfind(|c| seps.contains(&c)) {
            // A separator was typed or pasted: everything before it becomes chips.
            let rest = text[pos + 1..].to_owned();
            add_entries(items, &text[..pos], seps);
            *text = rest;
        }
    });
    ui.add_space(6.0);
}

fn us_date(d: NaiveDate) -> String {
    d.format("%m/%d/%Y").to_string()
}

/// Accepts 3/5/2026, 03/05/2026, 3-5-2026 and 2026-03-05.
fn parse_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    ["%m/%d/%Y", "%m-%d-%Y", "%Y-%m-%d"]
        .iter()
        .find_map(|f| NaiveDate::parse_from_str(s, f).ok())
        .filter(|d| (1990..=2100).contains(&chrono::Datelike::year(d)))
}

/// A date that can be typed (MM/DD/YYYY) or picked on a calendar.
fn date_input(ui: &mut egui::Ui, key: &str, date: &mut NaiveDate, pending: &mut Pending) {
    let text = pending
        .entry(key.to_owned())
        .or_insert_with(|| us_date(*date));
    let r = ui.add(
        egui::TextEdit::singleline(text)
            .id(egui::Id::unique(("date_input", key)))
            .hint_text("MM/DD/YYYY")
            .desired_width(120.0),
    );
    if r.changed()
        && let Some(d) = parse_date(text)
    {
        *date = d;
    }
    let parsed = parse_date(text);
    if !r.has_focus() && parsed.is_some() && parsed != Some(*date) {
        *text = us_date(*date);
    }
    if simple_date(ui, key, date) {
        *text = us_date(*date);
    }
    if parsed.is_none() && !r.has_focus() {
        status_line(
            ui,
            Tone::Warning,
            "Type the date as MM/DD/YYYY, for example 03/15/2026",
        );
    }
}

/// Calendar picker button. Returns true when a date was picked.
fn simple_date(ui: &mut egui::Ui, key: &str, date: &mut NaiveDate) -> bool {
    use chrono::Datelike;
    let mut j = jiff::civil::Date::new(date.year() as i16, date.month() as i8, date.day() as i8)
        .unwrap_or_else(|_| jiff::civil::date(2000, 1, 1));
    let resp = ui.add(
        DatePickerButton::new(&mut j)
            .id_salt(key)
            .show_icon(false)
            .format("Calendar ▾")
            .calendar_week(false)
            .highlight_weekends(false),
    );
    if resp.changed()
        && let Some(d) =
            NaiveDate::from_ymd_opt(i32::from(j.year()), j.month() as u32, j.day() as u32)
    {
        *date = d;
        return true;
    }
    false
}

#[cfg(not(target_arch = "wasm32"))]
fn open_link(_ctx: &egui::Context, url: &str, _new_tab: bool) {
    let _ = open::that(url);
}

#[cfg(target_arch = "wasm32")]
fn open_link(ctx: &egui::Context, url: &str, new_tab: bool) {
    ctx.open_url(egui::OpenUrl {
        url: url.to_owned(),
        new_tab,
    });
}

/// Hand `text` to the browser as a file download named `name`.
#[cfg(target_arch = "wasm32")]
fn download(name: &str, text: &str) -> Result<(), String> {
    use wasm_bindgen::JsCast;
    let err = |e: wasm_bindgen::JsValue| format!("{e:?}");

    let parts = js_sys::Array::of1(&text.into());
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("text/plain;charset=utf-8");
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts).map_err(err)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(err)?;
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("no document")?;
    let a: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(err)?
        .dyn_into()
        .map_err(|_| "not an anchor".to_owned())?;
    a.set_href(&url);
    a.set_download(name);
    a.click();
    let _ = web_sys::Url::revoke_object_url(&url);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn documents_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let docs = PathBuf::from(home).join("Documents");
    docs.is_dir().then_some(docs)
}

/// Percent-encode for a mailto: URL.
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.replace('\n', "\r\n").bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn mailto(to: &str, subject: &str, body: &str) -> String {
    // Some mail programs cut off very long links; fall back to a short body.
    let full = format!(
        "mailto:{}?subject={}&body={}",
        encode(to),
        encode(subject),
        encode(body)
    );
    if full.len() <= 1900 {
        return full;
    }
    let short = "Hello,\n\nPlease run the attached Purview search. The request details are below.\n\n\
                 (Paste the details here using \"Copy email text\" in the app.)\n";
    format!(
        "mailto:{}?subject={}&body={}",
        encode(to),
        encode(subject),
        encode(short)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_parse() {
        let d = NaiveDate::from_ymd_opt(2026, 3, 5).unwrap();
        assert_eq!(parse_date("3/5/2026"), Some(d));
        assert_eq!(parse_date("03/05/2026"), Some(d));
        assert_eq!(parse_date("3-5-2026"), Some(d));
        assert_eq!(parse_date("2026-03-05"), Some(d));
        assert_eq!(parse_date("13/05/2026"), None);
        assert_eq!(parse_date("3/5/26"), None);
    }

    #[test]
    fn chips_split_and_dedupe() {
        let mut items = vec!["a@x.gov".to_owned()];
        add_entries(
            &mut items,
            "A@x.gov, b@x.gov;\nc@x.gov,,",
            &[',', ';', '\n'],
        );
        assert_eq!(items, vec!["a@x.gov", "b@x.gov", "c@x.gov"]);
    }

    #[test]
    fn pending_text_is_kept() {
        let mut req = SearchRequest::default();
        let mut pending = Pending::new();
        pending.insert("words.words".into(), "snow removal; plow".into());
        pending.insert("when.start".into(), "01/01/2026".into());
        flush_pending(&mut req, &mut pending);
        assert_eq!(req.keywords.words, vec!["snow removal", "plow"]);
        assert!(pending.contains_key("when.start"));
    }

    #[test]
    fn mailto_is_encoded() {
        let m = mailto("it@x.gov", "A & B", "Line 1\nLine 2");
        assert_eq!(
            m,
            "mailto:it%40x.gov?subject=A%20%26%20B&body=Line%201%0D%0ALine%202"
        );
    }
}
