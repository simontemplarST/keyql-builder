//! The look shared by the request builder and the search runner: colors, drawn badges (no emoji),
//! callouts, buttons, cards and the step list. Contrast targets: hint text at least 7:1 and field
//! borders at least 3:1 in both themes (WCAG 1.4.3 / 1.4.11).

use eframe::egui::{self, Color32, FontId, Margin, RichText, Sense, Stroke, TextStyle, Vec2};

use crate::query::Level;

pub fn setup_style(ctx: &egui::Context) {
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

/// What a message is about. Sets its color and the drawn badge beside it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
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

pub const ACCENT: Color32 = Color32::from_rgb(30, 100, 200);

pub fn tone_color(ui: &egui::Ui, tone: Tone) -> Color32 {
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
pub fn paint_badge(
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

pub fn badge(ui: &mut egui::Ui, tone: Tone, size: f32) {
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
pub fn status_line(ui: &mut egui::Ui, tone: Tone, text: impl Into<String>) {
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
pub enum StepState {
    Plain,
    Done,
    Warning,
    Error,
}

/// One row of the step list: a numbered circle (or a drawn tick), the title and a status line.
pub fn step_item(
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

pub fn error_color(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(255, 120, 110)
    } else {
        Color32::from_rgb(180, 30, 25)
    }
}

pub fn warning_color(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(245, 190, 80)
    } else {
        Color32::from_rgb(150, 90, 0)
    }
}

pub fn success_color(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(110, 200, 130)
    } else {
        Color32::from_rgb(20, 120, 50)
    }
}

pub fn primary(text: RichText) -> egui::Button<'static> {
    egui::Button::new(text.strong().color(Color32::WHITE))
        .fill(ACCENT)
        .min_size(Vec2::new(0.0, 36.0))
}

pub fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(primary(RichText::new(label)))
}

/// Bold label, an optional "Required"/"Optional" tag, and hint text below.
pub fn field_label(ui: &mut egui::Ui, label: &str, tag: Option<&str>, hint: &str) {
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
pub fn app_mark(ui: &mut egui::Ui, size: f32) {
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
pub fn numbered(ui: &mut egui::Ui, n: usize, text: &str) {
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
pub fn callout(ui: &mut egui::Ui, tone: Tone, title: &str, body: impl FnOnce(&mut egui::Ui)) {
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

/// A clickable card with a tick box (or a round radio mark), a title and a description.
/// Returns true when it was clicked.
pub fn choice_card(
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
