//! Visual language matched to Bakery 8.3.0 (style.css + racing-theme.css):
//! near-black surfaces, hairline borders, Geist type with heavy uppercase
//! headings, white primary buttons and a muted green "live" state.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Response,
    RichText, Sense, Shadow, Stroke, StrokeKind, TextStyle, Vec2,
};
use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
use std::sync::Arc;
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
};

// Surfaces.
pub const BG: Color32 = Color32::from_rgb(0x0e, 0x0f, 0x13); // --background
pub const PANEL: Color32 = Color32::from_rgb(0x11, 0x13, 0x17); // --card, sidebar
pub const POPOVER: Color32 = Color32::from_rgb(0x18, 0x1a, 0x20); // --popover / --secondary
pub const RAISED: Color32 = Color32::from_rgb(0x22, 0x24, 0x2a); // --accent (hover surfaces)
pub const INPUT_BG: Color32 = Color32::from_rgb(0x0a, 0x0b, 0x0e);
// Text.
pub const TEXT: Color32 = Color32::WHITE; // --foreground
pub const TEXT_DIM: Color32 = Color32::from_rgb(0xa2, 0xa3, 0xa8); // --muted-foreground
pub const PRIMARY_FG: Color32 = Color32::from_rgb(0x0b, 0x0c, 0x0f); // text on white buttons
// States.
pub const LIVE: Color32 = Color32::from_rgb(0x4a, 0xde, 0x80);
pub const STANDBY: Color32 = Color32::from_rgb(0xe0, 0xb8, 0x6a);
pub const DESTRUCTIVE_TEXT: Color32 = Color32::from_rgb(0xe1, 0xb0, 0xaf);

/// White at `a` (0-255) alpha: Bakery's #ffffffXX borders and overlays.
pub const fn white(a: u8) -> Color32 {
    Color32::from_rgba_premultiplied(a, a, a, a)
}
pub const BORDER: Color32 = white(0x10);
pub const INPUT_BORDER: Color32 = white(0x14);

pub fn alpha(color: Color32, pct: u8) -> Color32 {
    let a = (pct as u32 * 255 / 100) as u8;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a)
}

pub const RADIUS_CARD: u8 = 8;
pub const RADIUS_BUTTON: u8 = 6;
pub const RADIUS_INPUT: u8 = 5;

// Geist weights, bundled (SIL Open Font License, see assets/fonts/OFL.txt).
const GEIST_REGULAR: &[u8] = include_bytes!("../assets/fonts/Geist-Regular.ttf");
const GEIST_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Geist-SemiBold.ttf");
const GEIST_EXTRABOLD: &[u8] = include_bytes!("../assets/fonts/Geist-ExtraBold.ttf");
const GEIST_BLACK: &[u8] = include_bytes!("../assets/fonts/Geist-Black.ttf");

pub fn semibold() -> FontFamily {
    FontFamily::Name("geist-600".into())
}
pub fn extrabold() -> FontFamily {
    FontFamily::Name("geist-800".into())
}
pub fn black() -> FontFamily {
    FontFamily::Name("geist-900".into())
}

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(13.0)),
            (TextStyle::Button, FontId::new(11.0, extrabold())),
            (TextStyle::Heading, FontId::new(22.0, black())),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();

        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(8.0, 8.0);
        s.button_padding = Vec2::new(14.0, 6.0);
        s.interact_size = Vec2::new(36.0, 32.0);
        s.window_margin = Margin::same(22);
        s.menu_margin = Margin::same(6);
        s.combo_width = 200.0;
        s.text_edit_width = 240.0;
        s.icon_width = 16.0;
        s.icon_width_inner = 9.0;

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = BG;
        v.window_fill = POPOVER;
        v.window_stroke = Stroke::new(1.0, white(0x1c));
        v.window_corner_radius = CornerRadius::same(RADIUS_CARD);
        v.menu_corner_radius = CornerRadius::same(RADIUS_BUTTON);
        v.window_shadow = Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(110),
        };
        v.popup_shadow = Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(68),
        };
        v.extreme_bg_color = INPUT_BG;
        v.text_edit_bg_color = Some(INPUT_BG);
        v.faint_bg_color = PANEL;
        v.code_bg_color = INPUT_BG;
        v.hyperlink_color = Color32::from_rgb(0xc5, 0xca, 0xd3);
        v.warn_fg_color = STANDBY;
        v.error_fg_color = DESTRUCTIVE_TEXT;
        v.selection.bg_fill = white(0x40);
        v.selection.stroke = Stroke::new(2.0, Color32::from_rgb(0xc3, 0xcc, 0xd5));
        v.collapsing_header_frame = false;
        v.indent_has_left_vline = false;

        let w = &mut v.widgets;
        let r = CornerRadius::same(RADIUS_BUTTON);
        w.noninteractive.bg_fill = PANEL;
        w.noninteractive.weak_bg_fill = PANEL;
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.noninteractive.corner_radius = r;

        // Outline-style controls (inputs, combo boxes, secondary buttons).
        w.inactive.bg_fill = INPUT_BG;
        w.inactive.weak_bg_fill = POPOVER;
        w.inactive.bg_stroke = Stroke::new(1.0, INPUT_BORDER);
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.corner_radius = r;
        w.inactive.expansion = 0.0;

        w.hovered.bg_fill = INPUT_BG;
        w.hovered.weak_bg_fill = RAISED;
        w.hovered.bg_stroke = Stroke::new(1.0, white(0x25));
        w.hovered.fg_stroke = Stroke::new(1.0, TEXT);
        w.hovered.corner_radius = r;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = INPUT_BG;
        w.active.weak_bg_fill = RAISED;
        w.active.bg_stroke = Stroke::new(1.0, white(0x40));
        w.active.fg_stroke = Stroke::new(1.0, TEXT);
        w.active.corner_radius = r;
        w.active.expansion = 0.0;

        w.open = w.active;
    });
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    // Built-in fonts stay as fallbacks (symbols, CJK, emoji).
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();

    for (key, bytes) in [
        ("geist-400", GEIST_REGULAR),
        ("geist-600", GEIST_SEMIBOLD),
        ("geist-800", GEIST_EXTRABOLD),
        ("geist-900", GEIST_BLACK),
    ] {
        fonts
            .font_data
            .insert(key.into(), Arc::new(FontData::from_static(bytes)));
        let mut stack = vec![key.to_owned()];
        stack.extend(fallback.iter().cloned());
        if key == "geist-400" {
            fonts.families.insert(FontFamily::Proportional, stack);
        } else {
            fonts.families.insert(FontFamily::Name(key.into()), stack);
        }
    }

    // Bakery uses Consolas for console/log output.
    if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\consola.ttf") {
        fonts
            .font_data
            .insert("consolas".into(), Arc::new(FontData::from_owned(bytes)));
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "consolas".into());
    }
    ctx.set_fonts(fonts);
}

// ------------------------------------------------------------------- icons

/// Line icons in the style of Bakery's lucide set, drawn as vectors.
#[derive(Clone, Copy)]
pub enum Icon {
    Play,
    Gamepad,
    Gear,
    List,
    Plus,
    Monitor,
    User,
    Logo,
    Home,
    Power,
}

pub fn paint_icon(
    painter: &egui::Painter,
    icon: Icon,
    center: egui::Pos2,
    size: f32,
    color: Color32,
) {
    use egui::{pos2, vec2};
    let s = size / 2.0;
    let st = Stroke::new((size / 12.0).max(1.25), color);
    match icon {
        Icon::Play => {
            let pts = vec![
                center + vec2(-s * 0.5, -s * 0.7),
                center + vec2(s * 0.75, 0.0),
                center + vec2(-s * 0.5, s * 0.7),
            ];
            painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
        }
        Icon::Gamepad => {
            let body = egui::Rect::from_center_size(center, vec2(size * 0.95, size * 0.6));
            painter.rect_stroke(
                body,
                CornerRadius::same((size * 0.28) as u8),
                st,
                StrokeKind::Middle,
            );
            let d = center + vec2(-s * 0.42, 0.0);
            let arm = s * 0.2;
            painter.line_segment([d - vec2(arm, 0.0), d + vec2(arm, 0.0)], st);
            painter.line_segment([d - vec2(0.0, arm), d + vec2(0.0, arm)], st);
            let b = center + vec2(s * 0.42, 0.0);
            painter.circle_filled(b + vec2(-s * 0.11, s * 0.09), s * 0.08, color);
            painter.circle_filled(b + vec2(s * 0.11, -s * 0.09), s * 0.08, color);
        }
        Icon::Gear => {
            for i in 0..8 {
                let a = i as f32 / 8.0 * std::f32::consts::TAU;
                let dir = vec2(a.cos(), a.sin());
                painter.line_segment(
                    [center + dir * s * 0.6, center + dir * s * 0.9],
                    Stroke::new(st.width * 1.6, color),
                );
            }
            painter.circle_stroke(center, s * 0.6, st);
            painter.circle_stroke(center, s * 0.22, st);
        }
        Icon::List => {
            for (i, w) in [(-1.0f32, 1.0f32), (0.0, 0.7), (1.0, 0.85)] {
                let y = center.y + i * s * 0.5;
                let left = center.x - s * 0.75;
                painter.line_segment([pos2(left, y), pos2(left + size * 0.75 * w, y)], st);
            }
        }
        Icon::Plus => {
            painter.line_segment(
                [center - vec2(s * 0.7, 0.0), center + vec2(s * 0.7, 0.0)],
                st,
            );
            painter.line_segment(
                [center - vec2(0.0, s * 0.7), center + vec2(0.0, s * 0.7)],
                st,
            );
        }
        Icon::Monitor => {
            let screen = egui::Rect::from_center_size(
                center - vec2(0.0, s * 0.12),
                vec2(size * 0.85, size * 0.58),
            );
            painter.rect_stroke(screen, CornerRadius::same(2), st, StrokeKind::Middle);
            let base_y = screen.bottom() + s * 0.32;
            painter.line_segment(
                [pos2(center.x, screen.bottom()), pos2(center.x, base_y)],
                st,
            );
            painter.line_segment(
                [
                    pos2(center.x - s * 0.35, base_y),
                    pos2(center.x + s * 0.35, base_y),
                ],
                st,
            );
        }
        Icon::Home => {
            // House outline: roof + body with a door.
            let roof = vec![
                center + vec2(-s * 0.8, -s * 0.05),
                center + vec2(0.0, -s * 0.8),
                center + vec2(s * 0.8, -s * 0.05),
            ];
            painter.add(egui::Shape::line(roof, st));
            let body = vec![
                center + vec2(-s * 0.58, -s * 0.22),
                center + vec2(-s * 0.58, s * 0.75),
                center + vec2(s * 0.58, s * 0.75),
                center + vec2(s * 0.58, -s * 0.22),
            ];
            painter.add(egui::Shape::line(body, st));
            let door = vec![
                center + vec2(-s * 0.18, s * 0.75),
                center + vec2(-s * 0.18, s * 0.25),
                center + vec2(s * 0.18, s * 0.25),
                center + vec2(s * 0.18, s * 0.75),
            ];
            painter.add(egui::Shape::line(door, st));
        }
        Icon::Power => {
            let r = s * 0.68;
            let n = 24;
            let pts: Vec<_> = (0..=n)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2
                        + 0.75
                        + (std::f32::consts::TAU - 1.5) * i as f32 / n as f32;
                    center + vec2(a.cos(), a.sin()) * r
                })
                .collect();
            painter.add(egui::Shape::line(pts, st));
            painter.line_segment(
                [center - vec2(0.0, s * 0.85), center - vec2(0.0, s * 0.15)],
                st,
            );
        }
        Icon::User => {
            painter.circle_stroke(center - vec2(0.0, s * 0.3), s * 0.32, st);
            let n = 16;
            let pts: Vec<_> = (0..=n)
                .map(|i| {
                    let a = std::f32::consts::PI + std::f32::consts::PI * i as f32 / n as f32;
                    center + vec2(0.0, s * 0.85) + vec2(a.cos() * s * 0.65, a.sin() * s * 0.6)
                })
                .collect();
            painter.add(egui::Shape::line(pts, st));
        }
        Icon::Logo => {
            // App mark: a rounded play glyph drawn in strokes, like Bakery's ink logo.
            let st = Stroke::new(size / 13.0, color);
            let pts = vec![
                center + vec2(-s * 0.42, -s * 0.62),
                center + vec2(s * 0.62, 0.0),
                center + vec2(-s * 0.42, s * 0.62),
                center + vec2(-s * 0.42, -s * 0.62),
            ];
            painter.add(egui::Shape::line(pts, st));
            let ring = egui::Rect::from_center_size(center, Vec2::splat(size * 0.98));
            painter.rect_stroke(
                ring,
                CornerRadius::same((size * 0.26) as u8),
                st,
                StrokeKind::Middle,
            );
        }
    }
}

pub fn icon(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_icon(ui.painter(), icon, rect.center(), size, color);
}

// ----------------------------------------------------------------- widgets

/// `.panel` / `.server-card`: #111317, hairline border, 8px radius.
pub fn panel() -> egui::Frame {
    egui::Frame::NONE
        .fill(PANEL)
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .stroke(Stroke::new(1.0, BORDER))
        .shadow(Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(18),
        })
}

/// White primary button (shadcn default variant), uppercase heavy label.
pub fn primary_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(button_text(text).color(PRIMARY_FG))
        .fill(TEXT)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(RADIUS_BUTTON))
        .min_size(Vec2::new(0.0, 35.0))
}

/// Outline button (shadcn outline variant).
pub fn outline_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(button_text(text).color(TEXT))
        .fill(white(0x04))
        .stroke(Stroke::new(1.0, INPUT_BORDER))
        .corner_radius(CornerRadius::same(RADIUS_BUTTON))
        .min_size(Vec2::new(0.0, 35.0))
}

/// Muted red outline, like Bakery's "Stop server".
pub fn danger_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(button_text(text).color(DESTRUCTIVE_TEXT))
        .fill(Color32::from_rgba_unmultiplied(0xb9, 0x6e, 0x6a, 0x08))
        .stroke(Stroke::new(
            1.0,
            Color32::from_rgba_unmultiplied(0xb9, 0x6e, 0x6a, 0x40),
        ))
        .corner_radius(CornerRadius::same(RADIUS_BUTTON))
        .min_size(Vec2::new(0.0, 35.0))
}

/// Text-only button (ghost variant).
pub fn ghost_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(button_text(text).color(TEXT_DIM))
        .frame(false)
        .min_size(Vec2::new(0.0, 35.0))
}

/// Button with a leading line icon (e.g. "+ ADD CONSOLE"), painted directly so
/// the icon and label are always centred in the 35px pill.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, text: &str, primary: bool) -> Response {
    icon_button_sized(ui, icon, text, primary, 35.0, 11.0, 13.0, 14.0)
}

/// The main call to action (e.g. START STREAM): taller, larger label.
pub fn big_button(ui: &mut egui::Ui, icon: Icon, text: &str) -> Response {
    icon_button_sized(ui, icon, text, true, 46.0, 12.5, 14.0, 24.0)
}

#[allow(clippy::too_many_arguments)]
fn icon_button_sized(
    ui: &mut egui::Ui,
    icon: Icon,
    text: &str,
    primary: bool,
    height: f32,
    font_size: f32,
    icon_size: f32,
    pad_x: f32,
) -> Response {
    let label = text.to_uppercase();
    let fg = if primary { PRIMARY_FG } else { TEXT };
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &label,
        0.0,
        egui::TextFormat {
            font_id: FontId::new(font_size, extrabold()),
            color: fg,
            extra_letter_spacing: font_size * 0.06,
            ..Default::default()
        },
    );
    let galley = ui.painter().layout_job(job);
    let gap = 8.0;
    let size = Vec2::new(pad_x * 2.0 + icon_size + gap + galley.size().x, height);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    let p = ui.painter();
    if primary {
        // hover: bg-primary/90
        let bg = if hovered {
            Color32::from_gray(230)
        } else {
            TEXT
        };
        p.rect_filled(rect, CornerRadius::same(RADIUS_BUTTON), bg);
    } else {
        let bg = if hovered { RAISED } else { white(0x04) };
        p.rect(
            rect,
            CornerRadius::same(RADIUS_BUTTON),
            bg,
            Stroke::new(1.0, INPUT_BORDER),
            StrokeKind::Inside,
        );
    }
    let icon_center = egui::pos2(rect.left() + pad_x + icon_size / 2.0, rect.center().y);
    paint_icon(p, icon, icon_center, icon_size, fg);
    let text_pos = egui::pos2(
        icon_center.x + icon_size / 2.0 + gap,
        rect.center().y - galley.size().y / 2.0,
    );
    p.galley(text_pos, galley, fg);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn button_text(text: &str) -> RichText {
    RichText::new(text.to_uppercase())
        .family(extrabold())
        .size(11.0)
        .extra_letter_spacing(0.66)
}

/// `.status` badge: tiny text, hairline border, coloured when live.
pub fn status_badge(ui: &mut egui::Ui, text: &str, color: Option<Color32>) {
    let (fg, bg) = match color {
        Some(c) => (c, alpha(c, 6)),
        None => (Color32::from_rgb(0xa3, 0xab, 0xb6), Color32::TRANSPARENT),
    };
    // Painted as one fixed-size widget so parent layouts can't stretch or reorder it.
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::new(10.0, semibold()), fg);
    let size = Vec2::new(
        8.0 + 6.0 + 6.0 + galley.size().x + 8.0,
        galley.size().y + 8.0,
    );
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect(
        rect,
        CornerRadius::same(RADIUS_INPUT),
        bg,
        Stroke::new(1.0, white(0x14)),
        StrokeKind::Inside,
    );
    let dot = egui::pos2(rect.left() + 11.0, rect.center().y);
    if let Some(c) = color {
        p.circle_filled(dot, 4.5, alpha(c, 18));
    }
    p.circle_filled(dot, 2.5, if color.is_some() { fg } else { white(0x73) });
    p.galley(
        egui::pos2(rect.left() + 20.0, rect.center().y - galley.size().y / 2.0),
        galley,
        fg,
    );
}

/// Small uppercase label (`.eyebrow` / section headings).
pub fn eyebrow(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(9.5)
            .family(extrabold())
            .color(white(0x73))
            .extra_letter_spacing(1.3),
    );
}

/// Kicker breadcrumb, uppercase H1 and uppercase subtitle (racing-theme heading).
pub fn page_heading(ui: &mut egui::Ui, kicker: &str, title: &str, subtitle: &str) {
    eyebrow(ui, kicker);
    ui.add_space(2.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .size(22.0)
            .family(black())
            .color(TEXT)
            .extra_letter_spacing(-0.5),
    );
    ui.add_space(-2.0);
    ui.label(
        RichText::new(subtitle.to_uppercase())
            .size(10.5)
            .family(extrabold())
            .color(white(0x78))
            .extra_letter_spacing(1.05),
    );
    ui.add_space(18.0);
}

/// Icon-only sidebar entry (`.nav-item`): 44px square, white bar when active.
pub fn nav_item(ui: &mut egui::Ui, icon: Icon, tooltip: &str, selected: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::click());
    let painter = ui.painter();
    if selected || resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(7), white(0x12));
    }
    if selected {
        let bar = egui::Rect::from_center_size(
            egui::pos2(rect.left() - 9.0, rect.center().y),
            Vec2::new(2.0, 18.0),
        );
        painter.rect_filled(bar, CornerRadius::same(2), TEXT);
    }
    let fg = if selected || resp.hovered() {
        TEXT
    } else {
        white(0x80)
    };
    paint_icon(painter, icon, rect.center(), 18.0, fg);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip)
}

/// Bakery's switch: grey track, green when on, with an On/Off label.
pub fn switch(ui: &mut egui::Ui, on: &mut bool) -> Response {
    ui.spacing_mut().item_spacing.x = 10.0;
    let size = Vec2::new(38.0, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.15);
    let lerp = |a: Color32, b: Color32| a.lerp_to_gamma(b, t);
    let track = lerp(
        Color32::from_rgb(0x2b, 0x2e, 0x34),
        Color32::from_rgb(0x36, 0x5c, 0x48),
    );
    let border = lerp(
        white(0x25),
        Color32::from_rgba_unmultiplied(0x8d, 0xd4, 0xad, 0x66),
    );
    let knob = lerp(
        Color32::from_rgb(0xb7, 0xbb, 0xc3),
        Color32::from_rgb(0xc5, 0xed, 0xcf),
    );
    let p = ui.painter();
    p.rect(
        rect,
        CornerRadius::same(11),
        track,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    let x = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
    p.circle_filled(egui::pos2(x, rect.center().y), 8.0, knob);
    let label = ui.label(RichText::new(if *on { "On" } else { "Off" }).size(12.0));
    resp.union(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Inline notice, using Bakery's install-progress panel look.
pub fn notice(ui: &mut egui::Ui, text: &str, is_error: bool) {
    let (fg, border) = if is_error {
        (
            DESTRUCTIVE_TEXT,
            Color32::from_rgba_unmultiplied(0xb9, 0x6e, 0x6a, 0x50),
        )
    } else {
        (
            Color32::from_rgb(0xb5, 0xd4, 0xc3),
            Color32::from_rgba_unmultiplied(0x8d, 0xd4, 0xad, 0x40),
        )
    };
    egui::Frame::NONE
        .fill(Color32::from_rgb(0x11, 0x15, 0x1c))
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(RADIUS_BUTTON))
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).size(11.5).color(fg));
        });
}

/// Hairline divider spanning the current width.
pub fn divider(ui: &mut egui::Ui) {
    let rect = ui.available_rect_before_wrap();
    ui.painter()
        .hline(rect.x_range(), rect.top(), Stroke::new(1.0, BORDER));
    ui.add_space(1.0);
}
pub fn apply_window_chrome(cc: &eframe::CreationContext<'_>) {
    let Ok(window) = cc.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = window.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as _;
    let color_ref = |color: Color32| u32::from_le_bytes([color.r(), color.g(), color.b(), 0]);
    // Set the native caption while eframe's window is still hidden. Explicit
    // caption colors are supported on Windows 11; older builds use dark mode.
    for (attribute, value) in [
        (DWMWA_USE_IMMERSIVE_DARK_MODE, 1),
        (DWMWA_CAPTION_COLOR, color_ref(PANEL)),
        (DWMWA_TEXT_COLOR, color_ref(TEXT)),
    ] {
        // SAFETY: cc owns a live Win32 window, and each attribute takes a
        // pointer to one 32-bit value borrowed for the duration of this call.
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                attribute as u32,
                (&raw const value).cast(),
                size_of::<u32>() as u32,
            );
        }
    }
}
