//! Small component kit on top of the theme tokens: buttons, chips, cards, navigation items, a sliding segmented
//! control, hover rows, empty and loading states, and the page-enter transition. Every state change animates
//! through `Motion`, so reduced motion makes them instant.

use crate::theme::{self, ease_out, mix, motion, semibold, tokens, Tone, R_SM, S1, S2, S3, S4, T_BODY, T_CAPTION, T_HEADING, T_SMALL, T_TITLE};
use egui::{Align2, Color32, CornerRadius, FontId, Margin, Pos2, Response, RichText, Role, Sense, Shape, Stroke, StrokeKind, Ui, WidgetInfo, vec2};
use std::fmt::Debug;
use std::hash::Hash;

pub fn muted(ui: &Ui, text: impl Into<String>) -> RichText {
    RichText::new(text).size(T_SMALL).color(tokens(ui.ctx()).muted)
}

// region buttons
#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    Primary,
    Secondary,
    Ghost,
}

fn button(ui: &mut Ui, label: &str, enabled: bool, look: Look, min_w: f32) -> Response {
    let ctx = ui.ctx().clone();
    let (t, m) = (tokens(&ctx), motion(&ctx));
    let font = FontId::new(T_BODY, semibold());
    let probe = ui.painter().layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE);
    let size = vec2((probe.size().x + 2.0 * S4).max(min_w), if t.fluent { 32.0 } else { 34.0 });
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    let hover = ctx.animate_bool_with_time_and_easing(resp.id.with("hover"), enabled && resp.hovered(), m.fast(), ease_out);
    let pressed = enabled && resp.is_pointer_button_down_on();
    let (fill, fg) = match (look, enabled) {
        (Look::Primary, true) => {
            let base = mix(t.action, t.state_target(), t.hover_lift() * hover);
            (if pressed { mix(t.action, t.state_target(), 0.14) } else { base }, t.on_action)
        }
        (Look::Secondary, true) => (if pressed { t.surface3 } else { mix(t.surface2, t.surface3, hover) }, t.text),
        (Look::Ghost, true) => {
            // fade a surface in over nothing: mixing with transparent black would darken the colour
            let a = (255.0 * hover).round() as u8;
            (if pressed { t.surface3 } else { Color32::from_rgba_unmultiplied(t.surface3.r(), t.surface3.g(), t.surface3.b(), a) }, t.text)
        }
        (_, false) => (t.surface2, t.subtle),
    };
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let r = CornerRadius::same(t.ctl_radius());
        p.rect_filled(rect, r, fill);
        if look == Look::Secondary && enabled {
            p.rect_stroke(rect, r, Stroke::new(1.0, mix(t.hairline, t.brand, 0.6 * hover)), StrokeKind::Inside);
        }
        if resp.has_focus() {
            let ring = t.ring();
            p.rect_stroke(rect.expand(2.0), CornerRadius::same(t.ctl_radius() + 2), Stroke::new(2.0, ring), StrokeKind::Outside);
        }
        p.text(rect.center(), Align2::CENTER_CENTER, label, font, fg);
    }
    if enabled {
        ui.ctx().set_cursor_icon(if resp.hovered() { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default });
    }
    resp
}

/// The one "go" action of a view: lime fill, dark text.
pub fn primary(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    button(ui, label, enabled, Look::Primary, 96.0)
}

pub fn secondary(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    button(ui, label, enabled, Look::Secondary, 0.0)
}

pub fn ghost(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    button(ui, label, enabled, Look::Ghost, 0.0)
}
// endregion

pub fn chip(ui: &mut Ui, text: &str, tone: Tone) {
    let (bg, fg) = tokens(ui.ctx()).tone(tone);
    egui::Frame::new().fill(bg).corner_radius(R_SM + 2).inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
        ui.add(egui::Label::new(RichText::new(text).size(T_CAPTION).color(fg)).extend());
    });
}

/// A raised group of content: surface fill, quiet border, no shadow at rest.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    let t = tokens(ui.ctx());
    egui::Frame::new().fill(t.surface1).stroke(Stroke::new(1.0, t.hairline)).corner_radius(t.card_radius()).inner_margin(S4 as i8).show(ui, add)
}

// region navigation
/// Sidebar item: hover tint, selection with an indicator that grows in, optional count on the right.
pub fn nav_item(ui: &mut Ui, selected: bool, label: &str, trailing: Option<&str>) -> Response {
    nav_item_icon(ui, selected, None, label, trailing)
}

/// Same, with a Segoe Fluent Icons glyph in front of the label (skipped when the font is not installed).
pub fn nav_item_icon(ui: &mut Ui, selected: bool, icon: Option<char>, label: &str, trailing: Option<&str>) -> Response {
    let ctx = ui.ctx().clone();
    let (t, m) = (tokens(&ctx), motion(&ctx));
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(Role::Tab, true, selected, label));
    let hover = ctx.animate_bool_with_time_and_easing(resp.id.with("hover"), resp.hovered(), m.fast(), ease_out);
    let sel = ctx.animate_bool_with_time_and_easing(resp.id.with("sel"), selected, m.base(), ease_out);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let r = CornerRadius::same(t.ctl_radius());
        let selected_fill = if t.fluent { t.surface2 } else { mix(t.rail, t.brand, 0.2) };
        p.rect_filled(rect, r, mix(mix(t.rail, t.surface2, hover), selected_fill, sel));
        if sel > 0.01 {
            let h = 16.0 * sel;
            let bar = egui::Rect::from_center_size(egui::pos2(rect.left() + 3.0, rect.center().y), vec2(3.0, h));
            p.rect_filled(bar, CornerRadius::same(2), t.ring());
        }
        let fg = mix(t.muted, t.text, (hover * 0.6).max(sel));
        let font = if selected { FontId::new(T_BODY, semibold()) } else { FontId::proportional(T_BODY) };
        let mut x = rect.left() + S3 + 4.0;
        if let Some(g) = icon.filter(|_| theme::icons_available()) {
            let glyph_color = if selected { t.ring() } else { fg };
            p.text(egui::pos2(x + 8.0, rect.center().y), Align2::CENTER_CENTER, g, FontId::new(16.0, theme::icons()), glyph_color);
            x += 28.0;
        }
        p.text(egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, label, font, fg);
        if let Some(tr) = trailing {
            p.text(egui::pos2(rect.right() - S3, rect.center().y), Align2::RIGHT_CENTER, tr, FontId::proportional(T_CAPTION), t.subtle);
        }
        if resp.has_focus() {
            let ring = t.ring();
            p.rect_stroke(rect, r, Stroke::new(2.0, ring), StrokeKind::Inside);
        }
    }
    if resp.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Two or more exclusive choices on one rail. The selected thumb slides to its new place.
/// Returns true when the choice changed.
pub fn segmented(ui: &mut Ui, current: &mut usize, labels: &[&str]) -> bool {
    let ctx = ui.ctx().clone();
    let (t, m) = (tokens(&ctx), motion(&ctx));
    let font = FontId::new(T_BODY, semibold());
    let widths: Vec<f32> = labels.iter().map(|l| ui.painter().layout_no_wrap(l.to_string(), font.clone(), Color32::WHITE).size().x + 2.0 * S4).collect();
    let pad = 3.0;
    let total: f32 = widths.iter().sum::<f32>() + pad * 2.0;
    let (rect, mut resp) = ui.allocate_exact_size(vec2(total, 36.0), Sense::hover());
    let mut changed = false;
    let mut x = rect.left() + pad;
    let mut slots = Vec::new();
    for w in &widths {
        slots.push(egui::Rect::from_min_size(egui::pos2(x, rect.top() + pad), vec2(*w, 30.0)));
        x += w;
    }
    let id = resp.id.with("thumb");
    let target = slots.get(*current).map_or(rect.left(), |s| s.left());
    let thumb_x = ctx.animate_value_with_time(id, target, m.base());
    let thumb_w = ctx.animate_value_with_time(id.with("w"), widths.get(*current).copied().unwrap_or(40.0), m.base());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(t.ctl_radius() + 2), t.surface1);
        p.rect_filled(egui::Rect::from_min_size(egui::pos2(thumb_x, rect.top() + pad), vec2(thumb_w, 30.0)), CornerRadius::same(t.ctl_radius()), t.surface3);
    }
    for (i, (slot, label)) in slots.iter().zip(labels).enumerate() {
        let r = ui.interact(*slot, resp.id.with(i), Sense::click());
        r.widget_info(|| WidgetInfo::selected(Role::Tab, true, *current == i, *label));
        let hover = ctx.animate_bool_with_time_and_easing(r.id.with("h"), r.hovered(), m.fast(), ease_out);
        let fg = if *current == i { t.text } else { mix(t.muted, t.text, hover) };
        ui.painter().text(slot.center(), Align2::CENTER_CENTER, *label, font.clone(), fg);
        if r.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if r.has_focus() {
            let ring = t.ring();
            ui.painter().rect_stroke(*slot, CornerRadius::same(t.ctl_radius()), Stroke::new(2.0, ring), StrokeKind::Inside);
        }
        if r.clicked() && *current != i {
            *current = i;
            changed = true;
        }
    }
    if changed {
        resp.mark_changed();
    }
    changed
}
// endregion

// region rows and states
/// A list row with an animated hover tint. `selected` rows keep a faint brand tint.
pub fn row<R>(ui: &mut Ui, id: impl Hash + Debug, selected: bool, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    let ctx = ui.ctx().clone();
    let (t, m) = (tokens(&ctx), motion(&ctx));
    let slot = ui.painter().add(Shape::Noop);
    let inner = egui::Frame::new().inner_margin(Margin::symmetric(S3 as i8, S2 as i8)).corner_radius(t.ctl_radius().min(14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    });
    let rect = inner.response.rect;
    let hovered = ui.rect_contains_pointer(rect);
    let h = ctx.animate_bool_with_time_and_easing(ui.scope_id().with(&id), hovered, m.fast(), ease_out);
    let base = if selected { mix(t.canvas, t.brand, 0.10) } else { t.canvas };
    let fill = mix(base, mix(t.surface1, t.surface2, 0.5), h * if selected { 0.4 } else { 1.0 });
    ui.painter().set(slot, Shape::rect_filled(rect, CornerRadius::same(t.ctl_radius().min(14)), fill));
    inner
}

/// Check box drawn by hand so a whole row can be the click target. Purely visual: the row owns the click.
pub fn check(ui: &mut Ui, checked: bool) {
    let ctx = ui.ctx().clone();
    let (t, m) = (tokens(&ctx), motion(&ctx));
    let (rect, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
    let k = ctx.animate_bool_with_time_and_easing(resp.id.with("c"), checked, m.fast(), ease_out);
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = ui.painter();
    let r = CornerRadius::same(5);
    p.rect_filled(rect, r, mix(t.surface2, t.brand, k));
    p.rect_stroke(rect, r, Stroke::new(1.5, mix(t.subtle, t.brand, k)), StrokeKind::Inside);
    if k > 0.02 {
        let at = |x: f32, y: f32| egui::pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height());
        p.add(Shape::line(vec![at(0.26, 0.53), at(0.44, 0.70), at(0.76, 0.32)], Stroke::new(2.0, Color32::WHITE.gamma_multiply(k))));
    }
}

/// A clickable tile with a title, a wrapped description and an optional right-aligned note.
pub fn tile(ui: &mut Ui, size: egui::Vec2, title: &str, desc: &str, meta: Option<&str>, enabled: bool, danger: bool) -> Response {
    let ctx = ui.ctx().clone();
    let (t, m) = (tokens(&ctx), motion(&ctx));
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, title));
    let h = ctx.animate_bool_with_time_and_easing(resp.id.with("h"), enabled && (resp.hovered() || resp.has_focus()), m.fast(), ease_out);
    if ui.is_rect_visible(rect) {
        let r = CornerRadius::same(t.card_radius());
        let accent = if danger { t.bad } else { t.ring() };
        let pressed = enabled && resp.is_pointer_button_down_on();
        let fill = if pressed { t.surface3 } else { mix(t.surface1, t.surface2, h) };
        ui.painter().rect_filled(rect, r, fill);
        let (w, color) = if resp.has_focus() { (2.0, accent) } else { (1.0, mix(t.hairline, accent, h * 0.85)) };
        ui.painter().rect_stroke(rect, r, Stroke::new(w, color), StrokeKind::Inside);
        let p = ui.painter().with_clip_rect(rect.shrink(1.0));
        let title_color = if !enabled { t.subtle } else if danger { t.bad } else { t.text };
        let tg = p.layout_no_wrap(title.to_string(), FontId::new(T_BODY, semibold()), title_color);
        let tpos = rect.min + vec2(S4, S3 + 2.0);
        let th = tg.size().y;
        p.galley(tpos, tg, title_color);
        let dg = p.layout(desc.to_string(), FontId::proportional(T_SMALL), if enabled { t.muted } else { t.subtle }, rect.width() - 2.0 * S4);
        p.galley(tpos + vec2(0.0, th + S1), dg, t.muted);
        if let Some(note) = meta {
            let ng = p.layout_no_wrap(note.to_string(), FontId::new(T_CAPTION, semibold()), t.brand_text);
            let pos = egui::pos2(rect.right() - S4 - ng.size().x, rect.top() + S3 + 2.0);
            p.galley(pos, ng, t.brand_text);
        }
    }
    if enabled && resp.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

pub fn page_header(ui: &mut Ui, title: &str, sub: &str) {
    ui.add_space(S1);
    ui.label(RichText::new(title).font(FontId::new(T_TITLE, semibold())).color(tokens(ui.ctx()).text));
    if !sub.is_empty() {
        ui.add(egui::Label::new(muted(ui, sub)).wrap());
    }
    ui.add_space(S3);
}

pub fn group_title(ui: &mut Ui, text: &str) {
    let t = tokens(ui.ctx());
    ui.add_space(S3);
    ui.label(RichText::new(text).font(FontId::new(T_HEADING - 2.0, semibold())).color(t.brand_text));
    ui.add_space(S1);
}

/// Empty or not-yet-loaded view: what is missing and what to do about it.
pub fn empty_state(ui: &mut Ui, title: &str, hint: &str) {
    let t = tokens(ui.ctx());
    ui.add_space(theme::S6 * 1.5);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).font(FontId::new(T_HEADING, semibold())).color(t.text));
        ui.add_space(S1);
        ui.add(egui::Label::new(RichText::new(hint).size(T_SMALL).color(t.muted)).wrap());
    });
}

/// Pulsing placeholder rows for content that is loading.
pub fn skeleton(ui: &mut Ui, rows: usize) {
    let ctx = ui.ctx().clone();
    let (t, m) = (tokens(&ctx), motion(&ctx));
    let phase = if m.reduce { 0.5 } else { (ctx.input(|i| i.time) as f32 * 2.2).sin() * 0.5 + 0.5 };
    if !m.reduce {
        ctx.request_repaint_after(std::time::Duration::from_millis(40));
    }
    for i in 0..rows {
        let w = ui.available_width() * [0.92, 0.78, 0.86, 0.64][i % 4];
        let (rect, _) = ui.allocate_exact_size(vec2(w, 14.0), Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::same(R_SM), mix(t.surface1, t.surface3, 0.25 + 0.5 * phase));
        ui.add_space(S2);
    }
}

/// Page-enter transition: content eases in with a short rise. `started` is the `ctx.input(|i| i.time)` of the switch.
pub fn enter<R>(ui: &mut Ui, started: f64, add: impl FnOnce(&mut Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    let m = motion(&ctx);
    let now = ctx.input(|i| i.time);
    let t = if m.reduce { 1.0 } else { ((now - started) / m.slow() as f64).clamp(0.0, 1.0) as f32 };
    if t < 1.0 {
        ctx.request_repaint();
    }
    let e = ease_out(t);
    ui.scope(|ui| {
        ui.set_opacity(e);
        ui.add_space((1.0 - e) * 10.0);
        add(ui)
    })
    .inner
}
// endregion

// region readouts
/// Points on a circle from `start_deg` sweeping `sweep_deg` clockwise (egui's y axis points down).
pub fn arc_points(center: Pos2, radius: f32, start_deg: f32, sweep_deg: f32, steps: usize) -> Vec<Pos2> {
    (0..=steps.max(1))
        .map(|i| {
            let a = (start_deg + sweep_deg * i as f32 / steps.max(1) as f32).to_radians();
            Pos2::new(center.x + radius * a.cos(), center.y + radius * a.sin())
        })
        .collect()
}

/// Colour of a load reading: the accent while it is comfortable, amber above 75, red above 90.
pub fn load_color(t: &theme::Tokens, percent: f32) -> Color32 {
    if percent >= 90.0 {
        t.bad
    } else if percent >= 75.0 {
        t.warn
    } else if t.dark {
        t.brand
    } else {
        t.brand_text
    }
}

/// A 270 degree ring gauge with the value in the middle and a caption underneath. The arc eases to new readings.
pub fn gauge(ui: &mut Ui, size: f32, fraction: f32, color: Color32, value: &str, caption: &str) {
    let ctx = ui.ctx().clone();
    let (t, m) = (tokens(&ctx), motion(&ctx));
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let shown = ctx.animate_value_with_time(resp.id.with("v"), fraction.clamp(0.0, 1.0), m.slow());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let stroke_w = (size * 0.085).max(6.0);
    let radius = size / 2.0 - stroke_w;
    let center = rect.center();
    let p = ui.painter();
    p.add(Shape::line(arc_points(center, radius, 135.0, 270.0, 64), Stroke::new(stroke_w, t.surface3)));
    if shown > 0.004 {
        let pts = arc_points(center, radius, 135.0, 270.0 * shown, ((64.0 * shown) as usize).max(2));
        p.circle_filled(pts[0], stroke_w / 2.0, color);
        p.circle_filled(*pts.last().unwrap_or(&pts[0]), stroke_w / 2.0, color);
        p.add(Shape::line(pts, Stroke::new(stroke_w, color)));
    }
    p.text(center - vec2(0.0, size * 0.04), Align2::CENTER_CENTER, value, FontId::new(size * 0.2, semibold()), t.text);
    p.text(center + vec2(0.0, size * 0.17), Align2::CENTER_CENTER, caption, FontId::proportional(T_SMALL), t.muted);
}

/// Line chart of the last readings (0 to 100), newest on the right, with a faint fill under the line.
pub fn sparkline(ui: &mut Ui, size: egui::Vec2, data: &std::collections::VecDeque<f32>, color: Color32) {
    let t = tokens(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(t.ctl_radius().min(8)), t.surface2);
    for q in [0.25, 0.5, 0.75] {
        let y = rect.bottom() - rect.height() * q;
        p.line_segment([egui::pos2(rect.left() + 6.0, y), egui::pos2(rect.right() - 6.0, y)], Stroke::new(1.0, mix(t.surface2, t.hairline, 0.7)));
    }
    if data.len() < 2 {
        return;
    }
    let inner = rect.shrink2(vec2(6.0, 6.0));
    let step = inner.width() / (crate::opti::sysmon::HISTORY as f32 - 1.0);
    let x0 = inner.right() - step * (data.len() as f32 - 1.0);
    let pts: Vec<Pos2> = data.iter().enumerate().map(|(i, v)| egui::pos2(x0 + step * i as f32, inner.bottom() - inner.height() * (v / 100.0).clamp(0.0, 1.0))).collect();
    let mut mesh = egui::Mesh::default();
    let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 46);
    for w in pts.windows(2) {
        let base = mesh.vertices.len() as u32;
        for (pos, _) in [(w[0], 0), (w[1], 0)] {
            mesh.colored_vertex(pos, fill);
        }
        mesh.colored_vertex(egui::pos2(w[1].x, inner.bottom()), fill);
        mesh.colored_vertex(egui::pos2(w[0].x, inner.bottom()), fill);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    p.add(Shape::mesh(mesh));
    p.add(Shape::line(pts, Stroke::new(2.0, color)));
}
// endregion

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_starts_and_ends_where_expected() {
        let c = Pos2::new(100.0, 100.0);
        let pts = arc_points(c, 50.0, 135.0, 270.0, 27);
        assert_eq!(pts.len(), 28);
        // start bottom-left, end bottom-right, both on the circle
        assert!(pts[0].x < c.x && pts[0].y > c.y);
        assert!(pts[27].x > c.x && pts[27].y > c.y);
        for p in &pts {
            assert!((((p.x - c.x).powi(2) + (p.y - c.y).powi(2)).sqrt() - 50.0).abs() < 0.01);
        }
        // the top of the ring is passed halfway
        assert!(pts[14].y < c.y - 49.0 || pts[13].y < c.y - 49.0);
    }

    #[test]
    fn load_colour_steps_up_with_load() {
        let t = theme::Tokens::DARK;
        assert_eq!(load_color(&t, 10.0), t.brand);
        assert_eq!(load_color(&t, 80.0), t.warn);
        assert_eq!(load_color(&t, 95.0), t.bad);
    }
}
