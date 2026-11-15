//! The B-snap logo, rendered from assets/logo/B-snap-animated.svg with resvg.
//!
//! The SVG animates with CSS (two halves of the B split along the diagonal and
//! snap back), which resvg doesn't run. So each half is rendered to its own
//! texture and the keyframes from the SVG's <style> are replayed here. The
//! animation plays on launch and on hover, so idle frames cost nothing.

use eframe::egui::{self, Color32, Rect, Sense, TextureHandle, pos2, vec2};

const SVG: &str = include_str!("../assets/logo/B-snap-animated.svg");
/// Crop of the 512x512 artboard around the B (same crop Bakery's build uses).
const VIEW: (f32, f32, f32, f32) = (90.0, 65.0, 330.0, 380.0);
const CYCLE: f64 = 3.0;

pub struct Logo {
    top: Option<TextureHandle>,
    bottom: Option<TextureHandle>,
    rendered_px: f32,
    anim_start: Option<f64>,
    was_hovered: bool,
}

impl Logo {
    pub fn new() -> Self {
        Self {
            top: None,
            bottom: None,
            rendered_px: 0.0,
            anim_start: None,
            was_hovered: false,
        }
    }

    /// Draws the logo `height` points tall.
    pub fn show(&mut self, ui: &mut egui::Ui, height: f32) -> egui::Response {
        let size = vec2(height * VIEW.2 / VIEW.3, height);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);

        // Re-rasterise when the display scale changes so it stays sharp.
        let px = height * ctx.pixels_per_point() * 2.0;
        if self.top.is_none() || (px - self.rendered_px).abs() > 0.5 {
            self.top = render_half(&ctx, "top", px);
            self.bottom = render_half(&ctx, "bottom", px);
            self.rendered_px = px;
        }

        // Play once on launch and whenever the pointer enters the logo.
        let hovered = resp.hovered();
        if self.anim_start.is_none() || (hovered && !self.was_hovered && self.finished(now)) {
            self.anim_start = Some(now);
        }
        self.was_hovered = hovered;

        let t = self.anim_start.map_or(CYCLE, |s| now - s);
        if t < CYCLE {
            ctx.request_repaint();
        }
        let (dx, dy) = snap_offset((t / CYCLE).min(1.0) as f32);
        // Offsets are in SVG units inside the skew matrix(1 0 -.13 1 ..).
        let unit = height / VIEW.3;
        let off = vec2((dx - 0.13 * dy) * unit, dy * unit);

        let p = ui.painter();
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        if let Some(top) = &self.top {
            p.image(top.id(), rect.translate(off), uv, Color32::WHITE);
        }
        if let Some(bottom) = &self.bottom {
            p.image(bottom.id(), rect.translate(-off), uv, Color32::WHITE);
        }
        resp
    }

    fn finished(&self, now: f64) -> bool {
        self.anim_start.is_none_or(|s| now - s >= CYCLE)
    }
}

/// Top-half translation at progress `p` of the 3s cycle, from @keyframes snapTop.
/// The bottom half moves by the negative.
fn snap_offset(p: f32) -> (f32, f32) {
    let a = (-22.0f32, -16.0f32);
    let at = |k: f32| (a.0 * k, a.1 * k);
    // cubic-bezier(.16,1,.3,1) ~ strong ease-out; cubic-bezier(.7,0,1,.3) ~ ease-in.
    let out = |t: f32| 1.0 - (1.0 - t).powi(4);
    let inn = |t: f32| t * t * t;
    let seg = |p: f32, s: f32, e: f32| ((p - s) / (e - s)).clamp(0.0, 1.0);
    let lerp = |k0: f32, k1: f32, t: f32| at(k0 + (k1 - k0) * t);
    match p {
        p if p < 0.15 => (0.0, 0.0),
        p if p < 0.35 => lerp(0.0, 1.0, out(seg(p, 0.15, 0.35))),
        p if p < 0.45 => at(1.0),
        p if p < 0.52 => lerp(1.0, -0.12, inn(seg(p, 0.45, 0.52))),
        p if p < 0.59 => lerp(-0.12, 0.035, out(seg(p, 0.52, 0.59))),
        p if p < 0.67 => lerp(0.035, 0.0, out(seg(p, 0.59, 0.67))),
        _ => (0.0, 0.0),
    }
}

/// Builds a static SVG with just one half of the B, reusing the source's <defs>.
fn half_svg(clip: &str) -> Option<String> {
    let defs_start = SVG.find("<defs>")?;
    let defs_end = SVG.find("</defs>")? + "</defs>".len();
    let defs = &SVG[defs_start..defs_end];
    Some(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{} {} {} {}">{defs}<g transform="matrix(1 0 -.13 1 33.28 0)" fill="#f6f6f5"><g clip-path="url(#{clip})"><use href="#b" mask="url(#stripes)"/></g></g></svg>"##,
        VIEW.0, VIEW.1, VIEW.2, VIEW.3
    ))
}

fn render_half(ctx: &egui::Context, clip: &str, height_px: f32) -> Option<TextureHandle> {
    use resvg::{tiny_skia, usvg};
    let svg = half_svg(clip)?;
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).ok()?;
    let scale = height_px / VIEW.3;
    let (w, h) = (
        (VIEW.2 * scale).ceil() as u32,
        (VIEW.3 * scale).ceil() as u32,
    );
    let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let image = egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data());
    Some(ctx.load_texture(
        format!("b-snap-{clip}"),
        image,
        egui::TextureOptions::LINEAR,
    ))
}
