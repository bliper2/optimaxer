//! Design tokens and the egui style built from them. One tonal surface ladder instead of lines (canvas, rail,
//! surface 1 to 3), one accent for selection and focus (BRAND), one colour for the primary action (LIME), and a
//! small motion vocabulary. Every text colour is checked against the surfaces it sits on in the tests below.

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, vec2};
use std::sync::Arc;

pub const BRAND: Color32 = Color32::from_rgb(0x74, 0x78, 0xF8);
pub const LIME: Color32 = Color32::from_rgb(0xA3, 0xE6, 0x35);
/// Text on a LIME fill.
pub const ON_LIME: Color32 = Color32::from_rgb(0x10, 0x1A, 0x02);

// spacing scale
pub const S1: f32 = 4.0;
pub const S2: f32 = 8.0;
pub const S3: f32 = 12.0;
pub const S4: f32 = 16.0;
pub const S5: f32 = 24.0;
pub const S6: f32 = 32.0;

// radius scale
pub const R_SM: u8 = 6;
pub const R_MD: u8 = 8;
pub const R_LG: u8 = 12;
pub const R_XL: u8 = 16;

// type scale (px)
pub const T_CAPTION: f32 = 12.0;
pub const T_SMALL: f32 = 13.0;
pub const T_BODY: f32 = 14.0;
pub const T_HEADING: f32 = 16.0;
pub const T_TITLE: f32 = 22.0;
pub const T_DISPLAY: f32 = 28.0;

/// Names of the optional font families installed by `install_fonts`.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    /// App background and the content area.
    pub canvas: Color32,
    /// Sidebar and top bar, one step away from the canvas.
    pub rail: Color32,
    pub surface1: Color32,
    pub surface2: Color32,
    pub surface3: Color32,
    pub hairline: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub subtle: Color32,
    pub brand: Color32,
    /// BRAND adjusted to read as text on this theme.
    pub brand_text: Color32,
    pub good: Color32,
    pub warn: Color32,
    pub bad: Color32,
}

const fn c(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

impl Tokens {
    pub const DARK: Tokens = Tokens {
        dark: true,
        canvas: c(0x0F, 0x10, 0x15),
        rail: c(0x0B, 0x0C, 0x10),
        surface1: c(0x17, 0x18, 0x21),
        surface2: c(0x1F, 0x20, 0x2B),
        surface3: c(0x2A, 0x2B, 0x3A),
        hairline: c(0x2B, 0x2C, 0x39),
        text: c(0xEC, 0xED, 0xF3),
        muted: c(0xA9, 0xAC, 0xBD),
        subtle: c(0x8A, 0x8D, 0xA2),
        brand: BRAND,
        brand_text: c(0xA3, 0xA6, 0xFF),
        good: LIME,
        warn: c(0xF5, 0xA6, 0x23),
        bad: c(0xFF, 0x7A, 0x7A),
    };

    pub const LIGHT: Tokens = Tokens {
        dark: false,
        canvas: c(0xF6, 0xF5, 0xFC),
        rail: c(0xEE, 0xED, 0xF8),
        surface1: c(0xFF, 0xFF, 0xFF),
        surface2: c(0xF0, 0xEF, 0xF9),
        surface3: c(0xE4, 0xE3, 0xF2),
        hairline: c(0xDD, 0xDC, 0xEC),
        text: c(0x1B, 0x1C, 0x26),
        muted: c(0x52, 0x55, 0x6C),
        subtle: c(0x66, 0x69, 0x80),
        brand: BRAND,
        brand_text: c(0x46, 0x4B, 0xCC),
        good: c(0x3F, 0x6B, 0x0B),
        warn: c(0x8F, 0x52, 0x00),
        bad: c(0xB4, 0x23, 0x23),
    };

    pub fn of(dark: bool) -> Tokens {
        if dark { Tokens::DARK } else { Tokens::LIGHT }
    }

    pub fn tone(&self, tone: Tone) -> (Color32, Color32) {
        // (background, foreground)
        let fg = match tone {
            Tone::Neutral => self.muted,
            Tone::Brand => self.brand_text,
            Tone::Good => self.good,
            Tone::Warn => self.warn,
            Tone::Bad => self.bad,
        };
        let base = if matches!(tone, Tone::Neutral) { self.surface3 } else { mix(self.surface1, fg, 0.16) };
        (base, fg)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Brand,
    Good,
    Warn,
    Bad,
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// WCAG relative luminance and contrast ratio (used by the token tests).
#[cfg(test)]
pub fn luminance(c: Color32) -> f32 {
    let f = |v: u8| {
        let s = v as f32 / 255.0;
        if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
}

#[cfg(test)]
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Tokens for the running theme.
pub fn tokens(ctx: &egui::Context) -> Tokens {
    Tokens::of(ctx.global_style().visuals.dark_mode)
}

// region motion
/// Animation lengths in seconds. Reduced motion turns every one of them into an instant change.
#[derive(Clone, Copy)]
pub struct Motion {
    pub reduce: bool,
}

impl Motion {
    pub fn fast(self) -> f32 {
        if self.reduce { 0.0 } else { 0.09 }
    }
    pub fn base(self) -> f32 {
        if self.reduce { 0.0 } else { 0.16 }
    }
    pub fn slow(self) -> f32 {
        if self.reduce { 0.0 } else { 0.26 }
    }
}

fn motion_id() -> egui::Id {
    egui::Id::unique("orion-reduce-motion")
}

pub fn motion(ctx: &egui::Context) -> Motion {
    Motion { reduce: ctx.data(|d| d.get_temp::<bool>(motion_id())).unwrap_or(false) }
}

pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// True when Windows is set to play no animations (Settings > Accessibility > Visual effects).
pub fn system_reduces_motion() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION};
    let mut on: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through pvParam.
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut on as *mut i32).cast(), 0) };
    ok != 0 && on == 0
}
// endregion

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut load = |key: &str, file: &str| {
        let path = format!(r"C:\Windows\Fonts\{file}");
        match std::fs::read(path) {
            Ok(bytes) => {
                fonts.font_data.insert(key.into(), Arc::new(egui::FontData::from_owned(bytes)));
                true
            }
            Err(_) => false,
        }
    };
    let regular = load("segoe", "segoeui.ttf");
    let semi = load("segoe-semibold", "seguisb.ttf") || load("segoe-semibold", "segoeuib.ttf");
    if regular {
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    // the semibold family falls back to the regular stack so a missing file never breaks text
    let mut stack: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if semi {
        stack.insert(0, "segoe-semibold".into());
    }
    fonts.families.insert(semibold(), stack);
    ctx.set_fonts(fonts);
}

/// Builds the whole egui style from the tokens: tonal surfaces, quiet strokes, rounded controls, a visible
/// focus ring, floating scroll bars and the type scale.
pub fn apply(ctx: &egui::Context, dark: bool, reduce_motion: bool) {
    let t = Tokens::of(dark);
    ctx.data_mut(|d| d.insert_temp(motion_id(), reduce_motion));
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = t.canvas;
    v.window_fill = t.surface1;
    v.window_stroke = Stroke::new(1.0, t.hairline);
    v.window_corner_radius = CornerRadius::same(R_XL);
    v.menu_corner_radius = CornerRadius::same(R_LG);
    // inputs: one step lighter than the bar they sit on in the light theme, one step raised in the dark theme
    v.extreme_bg_color = if dark { t.surface2 } else { t.surface1 };
    v.faint_bg_color = t.surface1;
    v.code_bg_color = t.surface2;
    v.hyperlink_color = t.brand_text;
    v.warn_fg_color = t.warn;
    v.error_fg_color = t.bad;
    v.override_text_color = None;
    v.indent_has_left_vline = false;
    v.selection.bg_fill = mix(t.surface2, t.brand, 0.45);
    v.selection.stroke = Stroke::new(2.0, t.brand);
    v.window_shadow = egui::Shadow { offset: [0, 10], blur: 32, spread: 0, color: Color32::from_black_alpha(if dark { 140 } else { 40 }) };
    v.popup_shadow = v.window_shadow;

    let r = CornerRadius::same(R_MD);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = t.surface1;
    w.noninteractive.weak_bg_fill = t.surface1;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.hairline);
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = t.surface2;
    w.inactive.weak_bg_fill = t.surface2;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, t.text);
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = t.surface3;
    w.hovered.weak_bg_fill = t.surface3;
    w.hovered.bg_stroke = Stroke::new(1.0, mix(t.surface3, t.brand, 0.55));
    w.hovered.fg_stroke = Stroke::new(1.0, t.text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = mix(t.surface3, t.brand, 0.25);
    w.active.weak_bg_fill = mix(t.surface3, t.brand, 0.25);
    w.active.bg_stroke = Stroke::new(1.5, t.brand);
    w.active.fg_stroke = Stroke::new(1.0, t.text);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;
    w.open.bg_fill = t.surface2;
    w.open.weak_bg_fill = t.surface2;
    w.open.bg_stroke = Stroke::new(1.0, t.hairline);
    w.open.corner_radius = r;

    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = vec2(S2, S2);
        s.spacing.button_padding = vec2(S3, 7.0);
        s.spacing.interact_size = vec2(36.0, 32.0);
        s.spacing.window_margin = egui::Margin::same(S4 as i8);
        s.spacing.menu_margin = egui::Margin::same(S2 as i8);
        s.spacing.scroll = egui::style::ScrollStyle::floating();
        s.animation_time = Motion { reduce: reduce_motion }.base();
        s.text_styles = [
            (TextStyle::Small, FontId::new(T_CAPTION, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(T_BODY, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(T_BODY, semibold())),
            (TextStyle::Heading, FontId::new(T_TITLE, semibold())),
            (TextStyle::Monospace, FontId::new(T_SMALL, FontFamily::Monospace)),
        ]
        .into();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const AA: f32 = 4.5;

    fn surfaces(t: &Tokens) -> [(&'static str, Color32); 5] {
        [("canvas", t.canvas), ("rail", t.rail), ("surface1", t.surface1), ("surface2", t.surface2), ("surface3", t.surface3)]
    }

    #[test]
    fn body_and_muted_text_meet_wcag_aa_on_every_surface() {
        for t in [Tokens::DARK, Tokens::LIGHT] {
            for (name, s) in surfaces(&t) {
                assert!(contrast(t.text, s) >= 7.0, "text on {name} (dark={}): {:.2}", t.dark, contrast(t.text, s));
                assert!(contrast(t.muted, s) >= AA, "muted on {name} (dark={}): {:.2}", t.dark, contrast(t.muted, s));
            }
        }
    }

    #[test]
    fn subtle_text_meets_aa_on_the_surfaces_it_is_used_on() {
        for t in [Tokens::DARK, Tokens::LIGHT] {
            for (name, s) in [("canvas", t.canvas), ("rail", t.rail), ("surface1", t.surface1), ("surface2", t.surface2)] {
                assert!(contrast(t.subtle, s) >= AA, "subtle on {name} (dark={}): {:.2}", t.dark, contrast(t.subtle, s));
            }
        }
    }

    #[test]
    fn brand_text_and_tones_are_readable() {
        for t in [Tokens::DARK, Tokens::LIGHT] {
            for (name, s) in [("canvas", t.canvas), ("surface1", t.surface1), ("surface2", t.surface2)] {
                assert!(contrast(t.brand_text, s) >= AA, "brand text on {name} (dark={}): {:.2}", t.dark, contrast(t.brand_text, s));
            }
            for tone in [Tone::Neutral, Tone::Brand, Tone::Good, Tone::Warn, Tone::Bad] {
                let (bg, fg) = t.tone(tone);
                assert!(contrast(fg, bg) >= AA, "{tone:?} chip (dark={}): {:.2}", t.dark, contrast(fg, bg));
            }
        }
    }

    #[test]
    fn primary_button_text_is_readable_on_lime() {
        assert!(contrast(ON_LIME, LIME) >= 7.0, "{:.2}", contrast(ON_LIME, LIME));
        assert!(contrast(ON_LIME, mix(LIME, Color32::WHITE, 0.2)) >= 7.0, "hover state");
        assert!(contrast(ON_LIME, mix(LIME, Color32::BLACK, 0.18)) >= 7.0, "pressed state");
    }

    #[test]
    fn focus_and_selection_accent_is_visible_against_surfaces() {
        // WCAG 1.4.11: non-text indicators need 3:1
        for t in [Tokens::DARK, Tokens::LIGHT] {
            for (name, s) in [("canvas", t.canvas), ("surface1", t.surface1)] {
                let ring = if t.dark { t.brand } else { t.brand_text };
                assert!(contrast(ring, s) >= 3.0, "focus ring on {name} (dark={}): {:.2}", t.dark, contrast(ring, s));
            }
        }
    }

    #[test]
    fn surfaces_step_in_one_direction() {
        let d = Tokens::DARK;
        assert!(luminance(d.rail) < luminance(d.canvas) && luminance(d.canvas) < luminance(d.surface1) && luminance(d.surface1) < luminance(d.surface2) && luminance(d.surface2) < luminance(d.surface3));
        let l = Tokens::LIGHT;
        assert!(luminance(l.surface1) > luminance(l.canvas) && luminance(l.canvas) > luminance(l.rail) && luminance(l.rail) > luminance(l.surface3));
    }

    #[test]
    fn mix_and_contrast_math() {
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 0.5), Color32::from_rgb(128, 128, 128));
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(ease_out(0.5) > 0.5, "ease-out leads");
    }
}
