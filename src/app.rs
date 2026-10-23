//! egui front end: a form on the left, the generated KeyQL on the right.

use chrono::NaiveDate;
use eframe::egui::{self, Color32, RichText};
use egui_extras::DatePickerButton;
use serde::{Deserialize, Serialize};

use crate::query::{
    self, Built, CondOp, Condition, Confidence, DateCond, DateField, DateOp, Importance, Join,
    KeywordScope, Level, PersonCond, PersonField, Proximity, QuerySpec, RelativeDate,
    SensitiveCond, SizeOp, SizeUnit, Tri,
};

const DATE_PROPERTIES: &[&str] = &["sent", "received", "Created", "LastModifiedTime"];

#[derive(Clone, Serialize, Deserialize)]
struct Preset {
    name: String,
    spec: QuerySpec,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyqlApp {
    spec: QuerySpec,
    presets: Vec<Preset>,
    preset_name: String,
    #[serde(skip)]
    copied_at: Option<f64>,
}

impl KeyqlApp {
    pub fn new(cc: &eframe::CreationContext<'_>, example: bool) -> Self {
        let mut app: Self = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        if example {
            app.spec = QuerySpec::example(chrono::Local::now().date_naive());
        }
        app
    }
}

impl eframe::App for KeyqlApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let built = self.spec.build();

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::right("output")
            .resizable(true)
            .default_size(440.0)
            .min_size(300.0)
            .show(ui, |ui| self.output(ui, &built));
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| form(ui, &mut self.spec));
        });
    }
}

// ---------------------------------------------------------------------------
// Toolbar and output
// ---------------------------------------------------------------------------

impl KeyqlApp {
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.heading("Purview KeyQL Builder");
            ui.separator();
            if ui
                .button("🗋 New query")
                .on_hover_text("Clear every field")
                .clicked()
            {
                self.spec = QuerySpec::default();
            }
            if ui
                .button("✨ Example")
                .on_hover_text("Fill the form with a sample query")
                .clicked()
            {
                self.spec = QuerySpec::example(chrono::Local::now().date_naive());
            }
            ui.menu_button("📂 Saved queries", |ui| {
                if self.presets.is_empty() {
                    ui.label("Nothing saved yet.");
                }
                let mut remove = None;
                for (i, p) in self.presets.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.button(&p.name).clicked() {
                            self.spec = p.spec.clone();
                            self.preset_name = p.name.clone();
                            ui.close();
                        }
                        if ui.small_button("🗑").on_hover_text("Delete").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    self.presets.remove(i);
                }
            });
            ui.add(
                egui::TextEdit::singleline(&mut self.preset_name)
                    .hint_text("Name this query…")
                    .desired_width(180.0),
            );
            let name = self.preset_name.trim().to_owned();
            if ui
                .add_enabled(!name.is_empty(), egui::Button::new("💾 Save"))
                .on_hover_text(
                    "Save the current form under this name (replaces one with the same name)",
                )
                .clicked()
            {
                let preset = Preset {
                    name: name.clone(),
                    spec: self.spec.clone(),
                };
                match self.presets.iter_mut().find(|p| p.name == name) {
                    Some(p) => *p = preset,
                    None => self.presets.push(preset),
                }
            }
        });
        ui.add_space(4.0);
    }

    fn output(&mut self, ui: &mut egui::Ui, built: &Built) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.strong("Generated KeyQL");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let now = ui.input(|i| i.time);
                let recently = self.copied_at.is_some_and(|t| now - t < 1.5);
                let label = if recently { "✔ Copied" } else { "📋 Copy" };
                if ui
                    .add_enabled(!built.query.is_empty(), egui::Button::new(label))
                    .clicked()
                {
                    ui.ctx().copy_text(built.query.clone());
                    self.copied_at = Some(now);
                }
                if recently {
                    ui.ctx().request_repaint();
                }
                ui.weak(format!("{} chars", built.query.len()));
            });
        });
        ui.add_space(4.0);

        let mut shown = built.query.clone();
        egui::ScrollArea::vertical()
            .id_salt("query_scroll")
            .max_height(ui.available_height() * 0.6)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut shown)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(8)
                        .interactive(true)
                        .hint_text("Your query will appear here."),
                );
            });
        ui.weak("Paste into Purview → eDiscovery → Search → Query → KeyQL editor.");

        ui.add_space(8.0);
        ui.separator();
        ui.strong("Checks");
        egui::ScrollArea::vertical()
            .id_salt("notices")
            .show(ui, |ui| {
                if built.notices.is_empty() {
                    ui.label(
                        RichText::new("✔ No problems found.").color(Color32::from_rgb(60, 160, 90)),
                    );
                }
                for n in &built.notices {
                    let (icon, color) = match n.level {
                        Level::Error => ("⛔", Color32::from_rgb(215, 70, 60)),
                        Level::Warning => ("⚠", Color32::from_rgb(220, 160, 40)),
                        Level::Info => ("ℹ", ui.visuals().weak_text_color()),
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(icon).color(color));
                        ui.label(&n.message);
                    });
                }
            });
    }
}

// ---------------------------------------------------------------------------
// Form
// ---------------------------------------------------------------------------

fn form(ui: &mut egui::Ui, s: &mut QuerySpec) {
    ui.add_space(4.0);
    ui.label("Fill in only what you need. Sections are combined with AND; anything marked \"exclude\" or \"none of\" is removed with NOT.");
    ui.add_space(6.0);

    section(ui, "Keywords", count_keywords(s), true, |ui| {
        keywords(ui, s)
    });
    section(
        ui,
        "Words near each other",
        s.proximity.len(),
        false,
        |ui| proximity(ui, s),
    );
    section(ui, "Item types", s.kinds.len(), true, |ui| kinds(ui, s));
    section(
        ui,
        "People",
        s.people
            .iter()
            .filter(|p| !p.values.trim().is_empty())
            .count(),
        true,
        |ui| people(ui, s),
    );
    section(ui, "Dates", s.dates.len(), true, |ui| dates(ui, s));
    section(ui, "Email details", count_email(s), false, |ui| {
        email(ui, s)
    });
    section(ui, "Documents & files", count_docs(s), false, |ui| {
        docs(ui, s)
    });
    section(
        ui,
        "Size",
        usize::from(s.size.op != SizeOp::Any),
        false,
        |ui| size(ui, s),
    );
    section(
        ui,
        "Sensitive information",
        s.sensitive.len(),
        false,
        |ui| sensitive(ui, s),
    );
    section(ui, "Advanced conditions", s.advanced.len(), false, |ui| {
        advanced(ui, s)
    });
    section(
        ui,
        "Custom KeyQL",
        usize::from(!s.raw.trim().is_empty()),
        false,
        |ui| {
            ui.label("Anything typed here is added as-is, joined with AND.");
            ui.add(
                egui::TextEdit::multiline(&mut s.raw)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3)
                    .hint_text("e.g. (ItemClass:IPM.Note* OR ItemClass:IPM.Schedule*)"),
            );
        },
    );
}

fn section(
    ui: &mut egui::Ui,
    title: &str,
    active: usize,
    open: bool,
    body: impl FnOnce(&mut egui::Ui),
) {
    let text = if active > 0 {
        RichText::new(format!("{title}  ({active})")).strong()
    } else {
        RichText::new(title).strong()
    };
    egui::CollapsingHeader::new(text)
        .id_salt(title)
        .default_open(open || active > 0)
        .show(ui, |ui| {
            ui.add_space(2.0);
            body(ui);
            ui.add_space(6.0);
        });
}

fn combo<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut T,
    all: &[T],
    label: fn(T) -> &'static str,
    width: f32,
) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(label(*value))
        .width(width)
        .show_ui(ui, |ui| {
            for &v in all {
                ui.selectable_value(value, v, label(v));
            }
        });
}

fn text(ui: &mut egui::Ui, value: &mut String, hint: &str, width: f32) {
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(width),
    );
}

fn remove_button(ui: &mut egui::Ui) -> bool {
    ui.small_button("✖").on_hover_text("Remove").clicked()
}

fn keywords(ui: &mut egui::Ui, s: &mut QuerySpec) {
    let k = &mut s.keywords;
    egui::Grid::new("kw")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label("All of these words");
            text(ui, &mut k.all_words, "budget audit 2026", 360.0);
            ui.end_row();
            ui.label("Exact phrases")
                .on_hover_text("Separate several phrases with semicolons");
            text(ui, &mut k.phrases, "county board; fiscal year", 360.0);
            ui.end_row();
            ui.label("Any of these words");
            text(ui, &mut k.any_words, "draft final revised", 360.0);
            ui.end_row();
            ui.label("None of these words");
            text(ui, &mut k.none_words, "newsletter unsubscribe", 360.0);
            ui.end_row();
            ui.label("Look for them in");
            combo(
                ui,
                "kw_scope",
                &mut k.scope,
                &KeywordScope::ALL,
                KeywordScope::label,
                360.0,
            );
            ui.end_row();
        });
    ui.weak("Tips: end a word with * to match its beginning (budg* → budget, budgets). Put \"quotes\" around a phrase inside any box.");
}

fn proximity(ui: &mut egui::Ui, s: &mut QuerySpec) {
    let mut remove = None;
    for (i, p) in s.proximity.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            text(ui, &mut p.first, "first word", 130.0);
            ui.label("within");
            ui.add(egui::DragValue::new(&mut p.distance).range(1..=999));
            ui.label("words of");
            text(ui, &mut p.second, "second word", 130.0);
            ui.checkbox(&mut p.ordered, "in this order")
                .on_hover_text("Uses ONEAR: the first word must come before the second");
            if remove_button(ui) {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        s.proximity.remove(i);
    }
    if ui.button("➕ Add proximity search").clicked() {
        s.proximity.push(Proximity::default());
    }
}

fn kinds(ui: &mut egui::Ui, s: &mut QuerySpec) {
    ui.label("Only include these kinds of items (leave all unchecked for everything):");
    egui::Grid::new("kinds")
        .num_columns(2)
        .spacing([24.0, 4.0])
        .show(ui, |ui| {
            for (i, (value, label)) in query::KINDS.iter().enumerate() {
                let mut on = s.kinds.iter().any(|k| k == value);
                if ui.checkbox(&mut on, *label).changed() {
                    if on {
                        s.kinds.push((*value).to_owned());
                    } else {
                        s.kinds.retain(|k| k != value);
                    }
                }
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
    ui.horizontal(|ui| {
        if ui.small_button("Email + Teams + chats").clicked() {
            s.kinds = vec!["email".into(), "microsoftteams".into(), "im".into()];
        }
        if ui.small_button("Documents only").clicked() {
            s.kinds = vec!["docs".into()];
        }
        if ui.small_button("Clear").clicked() {
            s.kinds.clear();
        }
    });
}

fn people(ui: &mut egui::Ui, s: &mut QuerySpec) {
    if s.people.is_empty() {
        s.people.push(PersonCond::default());
    }
    let mut remove = None;
    for (i, p) in s.people.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            combo(
                ui,
                ("person", i),
                &mut p.field,
                &PersonField::ALL,
                PersonField::label,
                230.0,
            );
            ui.label("is any of");
            text(
                ui,
                &mut p.values,
                "jane@contoso.com; John Smith; contoso.com",
                280.0,
            );
            ui.checkbox(&mut p.exclude, "exclude")
                .on_hover_text("Leave out items involving these people");
            if remove_button(ui) {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        s.people.remove(i);
    }
    if ui.button("➕ Add people").clicked() {
        s.people.push(PersonCond::default());
    }
    ui.weak("Separate multiple people with commas or semicolons. A bare domain (contoso.com) matches everyone at that domain.");
}

fn dates(ui: &mut egui::Ui, s: &mut QuerySpec) {
    let today = chrono::Local::now().date_naive();
    let mut remove = None;
    for (i, d) in s.dates.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            combo(
                ui,
                ("dfield", i),
                &mut d.field,
                &DateField::ALL,
                DateField::label,
                260.0,
            );
            combo(
                ui,
                ("dop", i),
                &mut d.op,
                &DateOp::ALL,
                DateOp::label,
                170.0,
            );
            match d.op {
                DateOp::Between => {
                    date_picker(ui, &mut d.start, &format!("ds{i}"));
                    ui.label("and");
                    date_picker(ui, &mut d.end, &format!("de{i}"));
                }
                DateOp::LastNDays => {
                    ui.add(
                        egui::DragValue::new(&mut d.days)
                            .range(1..=36500)
                            .suffix(" days"),
                    );
                }
                DateOp::Relative => {
                    combo(
                        ui,
                        ("drel", i),
                        &mut d.relative,
                        &RelativeDate::ALL,
                        RelativeDate::label,
                        120.0,
                    );
                }
                _ => date_picker(ui, &mut d.start, &format!("ds{i}")),
            }
            if remove_button(ui) {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        s.dates.remove(i);
    }
    ui.horizontal_wrapped(|ui| {
        if ui.button("➕ Add date rule").clicked() {
            s.dates.push(DateCond::new(today));
        }
        ui.weak("Quick:");
        let mut quick = |ui: &mut egui::Ui, label: &str, cond: DateCond| {
            if ui.small_button(label).clicked() {
                s.dates.push(cond);
            }
        };
        quick(
            ui,
            "Last 30 days",
            DateCond {
                op: DateOp::LastNDays,
                days: 30,
                ..DateCond::new(today)
            },
        );
        quick(
            ui,
            "Last 90 days",
            DateCond {
                op: DateOp::LastNDays,
                days: 90,
                ..DateCond::new(today)
            },
        );
        let year = chrono::Datelike::year(&today);
        for y in [year, year - 1] {
            quick(
                ui,
                &format!("All of {y}"),
                DateCond {
                    op: DateOp::Between,
                    start: NaiveDate::from_ymd_opt(y, 1, 1).unwrap_or(today),
                    end: NaiveDate::from_ymd_opt(y, 12, 31).unwrap_or(today),
                    ..DateCond::new(today)
                },
            );
        }
    });
}

/// The egui date picker works on `jiff` dates; the query model uses `chrono`.
fn to_jiff(d: NaiveDate) -> jiff::civil::Date {
    use chrono::Datelike;
    jiff::civil::Date::new(d.year() as i16, d.month() as i8, d.day() as i8)
        .unwrap_or_else(|_| jiff::civil::date(2000, 1, 1))
}

fn from_jiff(d: jiff::civil::Date) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(i32::from(d.year()), d.month() as u32, d.day() as u32)
}

fn date_picker(ui: &mut egui::Ui, date: &mut NaiveDate, id: &str) {
    let mut j = to_jiff(*date);
    let response = ui.add(
        DatePickerButton::new(&mut j)
            .id_salt(id)
            .format("%Y-%m-%d")
            .show_icon(false)
            .calendar_week(false)
            .highlight_weekends(true),
    );
    if response.changed()
        && let Some(d) = from_jiff(j)
    {
        *date = d;
    }
}

fn email(ui: &mut egui::Ui, s: &mut QuerySpec) {
    let e = &mut s.email;
    egui::Grid::new("email")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label("Has attachments");
            combo(
                ui,
                "has_att",
                &mut e.has_attachment,
                &Tri::ALL,
                Tri::label,
                150.0,
            );
            ui.end_row();
            ui.label("Importance");
            combo(
                ui,
                "importance",
                &mut e.importance,
                &Importance::ALL,
                Importance::label,
                150.0,
            );
            ui.end_row();
            ui.label("Has been read");
            combo(ui, "is_read", &mut e.is_read, &Tri::ALL, Tri::label, 150.0);
            ui.end_row();
            ui.label("Attachment name is any of");
            text(
                ui,
                &mut e.attachment_names,
                "invoice*.pdf; contract.docx",
                320.0,
            );
            ui.end_row();
            ui.label("Outlook category is any of");
            text(ui, &mut e.categories, "Legal; Red category", 320.0);
            ui.end_row();
        });
}

fn docs(ui: &mut egui::Ui, s: &mut QuerySpec) {
    let d = &mut s.docs;
    ui.label("File type is any of:");
    ui.horizontal_wrapped(|ui| {
        for ext in query::EXTENSIONS {
            let mut on = d.extensions.iter().any(|e| e == ext);
            if ui.checkbox(&mut on, *ext).changed() {
                if on {
                    d.extensions.push((*ext).to_owned());
                } else {
                    d.extensions.retain(|e| e != ext);
                }
            }
        }
    });
    egui::Grid::new("docs")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label("Other file types");
            text(ui, &mut d.custom_extensions, "dwg, xlsm, kml", 320.0);
            ui.end_row();
            ui.label("File name is any of");
            text(ui, &mut d.file_names, "budget*; Q3 report.xlsx", 320.0);
            ui.end_row();
            ui.label("Site / folder URL");
            text(
                ui,
                &mut d.paths,
                "https://contoso.sharepoint.com/sites/Legal",
                320.0,
            );
            ui.end_row();
        });
    ui.weak(
        "A site URL matches everything stored under it. Author and modified-by live under People.",
    );
}

fn size(ui: &mut egui::Ui, s: &mut QuerySpec) {
    let z = &mut s.size;
    ui.horizontal_wrapped(|ui| {
        combo(ui, "size_op", &mut z.op, &SizeOp::ALL, SizeOp::label, 130.0);
        if z.op != SizeOp::Any {
            ui.add(egui::DragValue::new(&mut z.a).range(0.0..=1.0e9).speed(1.0));
            if z.op == SizeOp::Between {
                ui.label("and");
                ui.add(egui::DragValue::new(&mut z.b).range(0.0..=1.0e9).speed(1.0));
            }
            combo(
                ui,
                "size_unit",
                &mut z.unit,
                &SizeUnit::ALL,
                SizeUnit::label,
                70.0,
            );
        }
    });
}

fn sensitive(ui: &mut egui::Ui, s: &mut QuerySpec) {
    let mut remove = None;
    for (i, c) in s.sensitive.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt(("sit", i))
                .selected_text("Pick…")
                .width(70.0)
                .show_ui(ui, |ui| {
                    for name in query::SENSITIVE_TYPES {
                        if ui.selectable_label(c.name == *name, *name).clicked() {
                            c.name = (*name).to_owned();
                        }
                    }
                });
            text(ui, &mut c.name, "Sensitive info type name", 280.0);
            ui.label("at least");
            ui.add(egui::DragValue::new(&mut c.min_count).range(1..=100_000));
            ui.label("times,");
            combo(
                ui,
                ("sitconf", i),
                &mut c.confidence,
                &Confidence::ALL,
                Confidence::label,
                170.0,
            );
            if remove_button(ui) {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        s.sensitive.remove(i);
    }
    if ui.button("➕ Add sensitive info type").clicked() {
        s.sensitive.push(SensitiveCond::default());
    }
    ui.weak("Matches any of the listed types. Type the exact name of a custom sensitive info type from your tenant.");
}

fn advanced(ui: &mut egui::Ui, s: &mut QuerySpec) {
    ui.label("Build your own property conditions. They are read top to bottom, each one combined with everything above it.");
    let mut remove = None;
    for (i, c) in s.advanced.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            if i == 0 {
                ui.add_sized([80.0, 18.0], egui::Label::new("Where"));
            } else {
                combo(ui, ("join", i), &mut c.join, &Join::ALL, Join::label, 80.0);
            }
            property_picker(ui, i, c);
            combo(ui, ("op", i), &mut c.op, &CondOp::ALL, CondOp::label, 140.0);
            let is_date = DATE_PROPERTIES.contains(&c.property.as_str());
            value_field(ui, &mut c.value, is_date, &format!("av{i}"));
            if c.op == CondOp::Range {
                ui.label("to");
                value_field(ui, &mut c.value2, is_date, &format!("bv{i}"));
            }
            if remove_button(ui) {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        s.advanced.remove(i);
    }
    if ui.button("➕ Add condition").clicked() {
        s.advanced.push(Condition::default());
    }
}

fn property_picker(ui: &mut egui::Ui, i: usize, c: &mut Condition) {
    let known = query::PROPERTIES.iter().any(|(p, _)| *p == c.property);
    let shown = if known {
        c.property.clone()
    } else {
        "Custom…".to_owned()
    };
    egui::ComboBox::from_id_salt(("prop", i))
        .selected_text(shown)
        .width(150.0)
        .show_ui(ui, |ui| {
            for (name, hint) in query::PROPERTIES {
                if ui
                    .selectable_label(c.property == *name, *name)
                    .on_hover_text(*hint)
                    .clicked()
                {
                    c.property = (*name).to_owned();
                }
            }
            ui.separator();
            if ui.selectable_label(!known, "Custom…").clicked() && known {
                c.property.clear();
            }
        });
    if !known {
        text(ui, &mut c.property, "PropertyName", 120.0);
    }
}

fn value_field(ui: &mut egui::Ui, value: &mut String, is_date: bool, id: &str) {
    text(
        ui,
        value,
        if is_date { "YYYY-MM-DD" } else { "value" },
        if is_date { 95.0 } else { 180.0 },
    );
    if is_date {
        let current = NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
            .unwrap_or_else(|_| chrono::Local::now().date_naive());
        let mut d = to_jiff(current);
        let picked = ui.add(
            DatePickerButton::new(&mut d)
                .id_salt(id)
                .show_icon(false)
                .format("Pick…"),
        );
        if picked.changed()
            && let Some(d) = from_jiff(d)
        {
            *value = d.format("%Y-%m-%d").to_string();
        }
    }
}

// ---------------------------------------------------------------------------
// Section badges
// ---------------------------------------------------------------------------

fn count_keywords(s: &QuerySpec) -> usize {
    let k = &s.keywords;
    query::tokenize(&k.all_words).len()
        + query::split_phrases(&k.phrases).len()
        + query::tokenize(&k.any_words).len()
        + query::tokenize(&k.none_words).len()
}

fn count_email(s: &QuerySpec) -> usize {
    let e = &s.email;
    usize::from(e.has_attachment != Tri::Any)
        + usize::from(e.importance != Importance::Any)
        + usize::from(e.is_read != Tri::Any)
        + query::split_list(&e.attachment_names).len()
        + query::split_list(&e.categories).len()
}

fn count_docs(s: &QuerySpec) -> usize {
    let d = &s.docs;
    d.extensions.len()
        + query::split_list(&d.custom_extensions).len()
        + query::split_list(&d.file_names).len()
        + query::split_list(&d.paths).len()
}
