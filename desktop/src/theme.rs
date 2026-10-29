//! Design tokens and the egui style built from them. One tonal surface ladder instead of lines (canvas, rail,
//! surface 1 to 3), one accent for selection and focus (BRAND), one colour for the primary action (LIME), and a
//! small motion vocabulary. Every text colour is checked against the surfaces it sits on in the tests below.

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, vec2};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Which look the app wears. Windows follows the Fluent design of Windows 11 (system accent colour, 4 px controls,
/// layered surfaces); Classic is the original indigo and lime look; Material uses the Material Design 3 palettes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    #[default]
    Windows,
    Classic,
    Material,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
}

impl Density {
    pub const ALL: [Self; 2] = [Self::Comfortable, Self::Compact];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiScale {
    Small,
    #[default]
    Default,
    Large,
    Larger,
}

impl UiScale {
    pub const ALL: [Self; 4] = [Self::Small, Self::Default, Self::Large, Self::Larger];

    pub fn label(self) -> &'static str {
        match self {
            Self::Small => "90%",
            Self::Default => "100%",
            Self::Large => "110%",
            Self::Larger => "125%",
        }
    }

    pub fn factor(self) -> f32 {
        match self {
            Self::Small => 0.9,
            Self::Default => 1.0,
            Self::Large => 1.1,
            Self::Larger => 1.25,
        }
    }
}

pub struct Appearance {
    pub dark: bool,
    pub reduce_motion: bool,
    pub style: Style,
    pub palette: Palette,
    pub accent: Color32,
    pub density: Density,
    pub scale: UiScale,
}

impl Style {
    pub const ALL: [Style; 3] = [Style::Windows, Style::Classic, Style::Material];
}

/// Colour palette is independent of control shape, so users can combine it with any style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Palette {
    #[default]
    System,
    Midnight,
    Oled,
    Graphite,
    Nebula,
    Nord,
    Arctic,
    FluentDark,
    FluentLight,
    MaterialDark,
    MaterialLight,
    Emerald,
    Crimson,
    Solarized,
    Porcelain,
    Cyberpunk,
    Custom,
}

impl Palette {
    pub const ALL: [Palette; 17] = [
        Palette::System, Palette::Midnight, Palette::Oled, Palette::Graphite, Palette::Nebula,
        Palette::Nord, Palette::Arctic, Palette::FluentDark, Palette::FluentLight,
        Palette::MaterialDark, Palette::MaterialLight, Palette::Emerald, Palette::Crimson,
        Palette::Solarized, Palette::Porcelain, Palette::Cyberpunk, Palette::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Palette::System => "System",
            Palette::Midnight => "Midnight",
            Palette::Oled => "OLED",
            Palette::Graphite => "Graphite",
            Palette::Nebula => "Nebula",
            Palette::Nord => "Nord",
            Palette::Arctic => "Arctic",
            Palette::FluentDark => "Fluent dark",
            Palette::FluentLight => "Fluent light",
            Palette::MaterialDark => "Material dark",
            Palette::MaterialLight => "Material light",
            Palette::Emerald => "Emerald",
            Palette::Crimson => "Crimson",
            Palette::Solarized => "Solarized",
            Palette::Porcelain => "Porcelain",
            Palette::Cyberpunk => "Cyberpunk",
            Palette::Custom => "Custom accent",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "system" => Some(Palette::System),
            "midnight" => Some(Palette::Midnight),
            "oled" => Some(Palette::Oled),
            "graphite" => Some(Palette::Graphite),
            "nebula" => Some(Palette::Nebula),
            "nord" => Some(Palette::Nord),
            "arctic" => Some(Palette::Arctic),
            "fluent_dark" => Some(Palette::FluentDark),
            "fluent_light" => Some(Palette::FluentLight),
            "material_dark" => Some(Palette::MaterialDark),
            "material_light" => Some(Palette::MaterialLight),
            "emerald" => Some(Palette::Emerald),
            "crimson" => Some(Palette::Crimson),
            "solarized" => Some(Palette::Solarized),
            "porcelain" => Some(Palette::Porcelain),
            "cyberpunk" => Some(Palette::Cyberpunk),
            "custom" => Some(Palette::Custom),
            _ => None,
        }
    }

    pub fn forced_dark(self) -> Option<bool> {
        match self {
            Palette::FluentDark | Palette::MaterialDark => Some(true),
            Palette::FluentLight | Palette::MaterialLight => Some(false),
            _ => None,
        }
    }
}

pub fn deserialize_palette<'de, D>(deserializer: D) -> Result<Palette, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let key = String::deserialize(deserializer)?;
    Ok(Palette::from_key(&key).unwrap_or_default())
}

/// The accent Windows ships with, used when the registry has none.
pub const DEFAULT_ACCENT: Color32 = Color32::from_rgb(0x00, 0x78, 0xD4);

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

/// Segoe Fluent Icons, the glyph font Windows 11 uses for its own navigation icons.
pub fn icons() -> FontFamily {
    FontFamily::Name("icons".into())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    /// Windows 11 Fluent shape: 4 px controls, 8 px cards.
    pub fluent: bool,
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
    /// Fill and text colour of the primary action button.
    pub action: Color32,
    pub on_action: Color32,
    /// Fully rounded controls (the Material shape) instead of the default 8 px corners.
    pub pill: bool,
}

const fn c(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

impl Tokens {
    pub const DARK: Tokens = Tokens {
        dark: true,
        fluent: false,
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
        action: LIME,
        on_action: ON_LIME,
        pill: false,
    };

    pub const LIGHT: Tokens = Tokens {
        dark: false,
        fluent: false,
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
        action: LIME,
        on_action: ON_LIME,
        pill: false,
    };

    /// Material Design 3 baseline, dark scheme: tonal surface containers, the lavender primary, pill-shaped controls.
    pub const MATERIAL_DARK: Tokens = Tokens {
        dark: true,
        fluent: false,
        canvas: c(0x14, 0x12, 0x18),
        rail: c(0x0F, 0x0D, 0x13),
        surface1: c(0x21, 0x1F, 0x26),
        surface2: c(0x2B, 0x29, 0x30),
        surface3: c(0x36, 0x34, 0x3B),
        hairline: c(0x49, 0x45, 0x4F),
        text: c(0xE6, 0xE0, 0xE9),
        muted: c(0xCA, 0xC4, 0xD0),
        subtle: c(0x9A, 0x94, 0xA6),
        brand: c(0xD0, 0xBC, 0xFF),
        brand_text: c(0xD0, 0xBC, 0xFF),
        good: c(0xA8, 0xDA, 0xB5),
        warn: c(0xF9, 0xDE, 0x8D),
        bad: c(0xF2, 0xB8, 0xB5),
        action: c(0xD0, 0xBC, 0xFF),
        on_action: c(0x38, 0x1E, 0x72),
        pill: true,
    };

    /// Material Design 3 baseline, light scheme.
    pub const MATERIAL_LIGHT: Tokens = Tokens {
        dark: false,
        fluent: false,
        canvas: c(0xFE, 0xF7, 0xFF),
        rail: c(0xF3, 0xED, 0xF7),
        surface1: c(0xF7, 0xF2, 0xFA),
        surface2: c(0xEC, 0xE6, 0xF0),
        surface3: c(0xE6, 0xE0, 0xE9),
        hairline: c(0xCA, 0xC4, 0xD0),
        text: c(0x1D, 0x1B, 0x20),
        muted: c(0x49, 0x45, 0x4F),
        subtle: c(0x62, 0x5B, 0x71),
        brand: c(0x67, 0x50, 0xA4),
        brand_text: c(0x67, 0x50, 0xA4),
        good: c(0x14, 0x6C, 0x2E),
        warn: c(0x7D, 0x57, 0x00),
        bad: c(0xB3, 0x26, 0x1E),
        action: c(0x67, 0x50, 0xA4),
        on_action: c(0xFF, 0xFF, 0xFF),
        pill: true,
    };

    pub fn of(dark: bool, style: Style, accent: Color32) -> Tokens {
        match (style, dark) {
            (Style::Windows, d) => Tokens::fluent(d, accent),
            (Style::Classic, true) => Tokens::DARK,
            (Style::Classic, false) => Tokens::LIGHT,
            (Style::Material, true) => Tokens::MATERIAL_DARK,
            (Style::Material, false) => Tokens::MATERIAL_LIGHT,
        }
    }

    pub fn with_palette(mut self, palette: Palette, accent: Color32) -> Tokens {
        let colors = match palette {
            Palette::System => return self,
            Palette::Custom => return recolor(self, accent),
            Palette::FluentDark => Tokens::fluent(true, accent),
            Palette::FluentLight => Tokens::fluent(false, accent),
            Palette::MaterialDark => Tokens::MATERIAL_DARK,
            Palette::MaterialLight => Tokens::MATERIAL_LIGHT,
            _ => {
                let colors = palette_color_set(palette, self.dark);
                self.canvas = colors.canvas;
                self.rail = colors.rail;
                self.surface1 = colors.surface1;
                self.surface2 = colors.surface2;
                self.surface3 = colors.surface3;
                self.hairline = colors.hairline;
                return recolor(self, colors.brand);
            }
        };
        self.canvas = colors.canvas;
        self.rail = colors.rail;
        self.surface1 = colors.surface1;
        self.surface2 = colors.surface2;
        self.surface3 = colors.surface3;
        self.hairline = colors.hairline;
        recolor(self, colors.brand)
    }

    /// How far the primary button moves on hover: a light state layer for Material and Windows, a stronger lift for lime.
    pub fn hover_lift(&self) -> f32 {
        if self.pill || self.fluent { 0.08 } else { 0.18 }
    }

    /// The colour hover and pressed states move the primary fill toward: away from the text colour, so the text
    /// keeps its contrast in every state.
    pub fn state_target(&self) -> Color32 {
        if luminance(self.on_action) > 0.5 { Color32::BLACK } else { Color32::WHITE }
    }

    /// Focus ring and selection indicator: the accent adjusted to read against the surfaces.
    pub fn ring(&self) -> Color32 {
        self.brand_text
    }

    /// Corner radius of buttons and navigation items.
    pub fn ctl_radius(&self) -> u8 {
        if self.pill { 17 } else if self.fluent { 4 } else { R_MD }
    }

    /// Corner radius of cards and tiles.
    pub fn card_radius(&self) -> u8 {
        if self.fluent { 8 } else { R_LG }
    }

    /// Windows 11 Fluent palette. `accent` is the user's Windows accent colour; text and fill colours derived from it
    /// are adjusted until they meet the contrast the tests demand.
    pub fn fluent(dark: bool, accent: Color32) -> Tokens {
        let (canvas, rail, surface1, surface2, surface3, hairline, text, muted, subtle) = if dark {
            (c(0x27, 0x27, 0x27), c(0x20, 0x20, 0x20), c(0x2C, 0x2C, 0x2C), c(0x32, 0x32, 0x32), c(0x3D, 0x3D, 0x3D), c(0x45, 0x45, 0x45), c(0xFF, 0xFF, 0xFF), c(0xC8, 0xC8, 0xC8), c(0x9D, 0x9D, 0x9D))
        } else {
            (c(0xF9, 0xF9, 0xF9), c(0xF3, 0xF3, 0xF3), c(0xFF, 0xFF, 0xFF), c(0xFB, 0xFB, 0xFB), c(0xEA, 0xEA, 0xEA), c(0xE0, 0xE0, 0xE0), c(0x1A, 0x1A, 0x1A), c(0x56, 0x56, 0x56), c(0x68, 0x68, 0x68))
        };
        let brand_text = readable_accent(accent, &[canvas, rail, surface1, surface2], dark);
        let (action, on_action) = action_colors(accent);
        Tokens {
            dark,
            fluent: true,
            canvas,
            rail,
            surface1,
            surface2,
            surface3,
            hairline,
            text,
            muted,
            subtle,
            brand: action,
            brand_text,
            good: if dark { c(0x6C, 0xCB, 0x5F) } else { c(0x0F, 0x6B, 0x0F) },
            warn: if dark { c(0xFC, 0xE1, 0x00) } else { c(0x8F, 0x52, 0x00) },
            bad: if dark { c(0xFF, 0x99, 0xA4) } else { c(0xB4, 0x23, 0x1B) },
            action,
            on_action,
            pill: false,
        }
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
        // the tint moves the background toward the text colour, so lift the text until it reads on the tint
        (base, readable_accent(fg, &[base], self.dark))
    }
}

fn recolor(mut t: Tokens, brand: Color32) -> Tokens {
    let surfaces = [t.canvas, t.rail, t.surface1, t.surface2, t.surface3];
    t.text = readable_color(t.text, &surfaces, t.dark, 7.0);
    t.muted = readable_accent(t.muted, &surfaces, t.dark);
    t.subtle = readable_accent(t.subtle, &[t.canvas, t.rail, t.surface1, t.surface2], t.dark);
    t.brand = brand;
    t.brand_text = readable_accent(brand, &[t.canvas, t.rail, t.surface1, t.surface2], t.dark);
    (t.action, t.on_action) = action_colors(brand);
    t
}

struct PaletteColors {
    canvas: Color32,
    rail: Color32,
    surface1: Color32,
    surface2: Color32,
    surface3: Color32,
    hairline: Color32,
    brand: Color32,
}

const fn palette_colors(
    canvas: Color32,
    rail: Color32,
    surface1: Color32,
    surface2: Color32,
    surface3: Color32,
    hairline: Color32,
    brand: Color32,
) -> PaletteColors {
    PaletteColors { canvas, rail, surface1, surface2, surface3, hairline, brand }
}

fn palette_color_set(palette: Palette, dark: bool) -> PaletteColors {
    use Palette as P;
    match (palette, dark) {
        (P::Midnight, true) => palette_colors(
            c(0x10, 0x17, 0x27), c(0x0B, 0x10, 0x1C), c(0x18, 0x22, 0x36), c(0x20, 0x2D, 0x44),
            c(0x2B, 0x3A, 0x54), c(0x35, 0x46, 0x60), c(0xA5, 0xB4, 0xFC),
        ),
        (P::Midnight, false) => palette_colors(
            c(0xF4, 0xF6, 0xFC), c(0xE9, 0xED, 0xF7), c(0xFF, 0xFF, 0xFF), c(0xF0, 0xF2, 0xFA),
            c(0xE3, 0xE7, 0xF2), c(0xD1, 0xD8, 0xE8), c(0x4F, 0x46, 0xE5),
        ),
        (P::Oled, true) => palette_colors(
            c(0, 0, 0), c(0, 0, 0), c(0x08, 0x08, 0x08), c(0x12, 0x12, 0x12), c(0x1E, 0x1E, 0x1E),
            c(0x32, 0x32, 0x32), c(0xBB, 0xA5, 0xFF),
        ),
        (P::Oled, false) => palette_colors(
            c(0xFA, 0xFA, 0xFA), c(0xF1, 0xF1, 0xF1), c(0xFF, 0xFF, 0xFF), c(0xF2, 0xF2, 0xF2),
            c(0xE6, 0xE6, 0xE6), c(0xD3, 0xD3, 0xD3), c(0x5B, 0x45, 0xB5),
        ),
        (P::Graphite, true) => palette_colors(
            c(0x18, 0x19, 0x1B), c(0x12, 0x13, 0x15), c(0x20, 0x21, 0x24), c(0x2A, 0x2C, 0x30),
            c(0x36, 0x38, 0x3D), c(0x48, 0x4A, 0x50), c(0xA6, 0xC8, 0xFF),
        ),
        (P::Graphite, false) => palette_colors(
            c(0xF2, 0xF3, 0xF4), c(0xE8, 0xE9, 0xEA), c(0xFF, 0xFF, 0xFF), c(0xF1, 0xF2, 0xF3),
            c(0xE3, 0xE5, 0xE7), c(0xD1, 0xD4, 0xD8), c(0x35, 0x5D, 0x8A),
        ),
        (P::Nebula, true) => palette_colors(
            c(0x17, 0x15, 0x24), c(0x10, 0x0F, 0x1A), c(0x20, 0x1D, 0x30), c(0x2C, 0x28, 0x3D),
            c(0x3A, 0x34, 0x4F), c(0x4B, 0x43, 0x60), c(0xC4, 0xA7, 0xFF),
        ),
        (P::Nebula, false) => palette_colors(
            c(0xF6, 0xF3, 0xFB), c(0xEC, 0xE8, 0xF4), c(0xFF, 0xFF, 0xFF), c(0xF3, 0xEF, 0xF8),
            c(0xE8, 0xE2, 0xF1), c(0xD7, 0xCF, 0xE3), c(0x68, 0x4A, 0x9C),
        ),
        (P::Nord, true) => palette_colors(
            c(0x2E, 0x34, 0x40), c(0x27, 0x2D, 0x38), c(0x35, 0x3C, 0x4A), c(0x3E, 0x46, 0x55),
            c(0x42, 0x4B, 0x5B), c(0x5A, 0x66, 0x78), c(0x88, 0xC0, 0xD0),
        ),
        (P::Nord, false) => palette_colors(
            c(0xEC, 0xEF, 0xF4), c(0xE5, 0xE9, 0xF0), c(0xFF, 0xFF, 0xFF), c(0xF3, 0xF5, 0xF8),
            c(0xE5, 0xE9, 0xF0), c(0xD1, 0xD7, 0xE2), c(0x4C, 0x73, 0x8A),
        ),
        (P::Arctic, true) => palette_colors(
            c(0x12, 0x1D, 0x28), c(0x0C, 0x15, 0x1E), c(0x1A, 0x28, 0x36), c(0x22, 0x34, 0x44),
            c(0x30, 0x45, 0x57), c(0x40, 0x57, 0x69), c(0x8F, 0xC9, 0xE8),
        ),
        (P::Arctic, false) => palette_colors(
            c(0xF1, 0xF7, 0xFA), c(0xE5, 0xEF, 0xF4), c(0xFF, 0xFF, 0xFF), c(0xEF, 0xF5, 0xF8),
            c(0xDE, 0xEA, 0xF0), c(0xC8, 0xD8, 0xE2), c(0x2D, 0x68, 0x85),
        ),
        (P::Emerald, true) => palette_colors(
            c(0x12, 0x1D, 0x18), c(0x0B, 0x15, 0x10), c(0x1A, 0x29, 0x21), c(0x23, 0x36, 0x2B),
            c(0x31, 0x48, 0x39), c(0x42, 0x5D, 0x4A), c(0x77, 0xD6, 0xA3),
        ),
        (P::Emerald, false) => palette_colors(
            c(0xF1, 0xF7, 0xF3), c(0xE5, 0xEF, 0xE8), c(0xFF, 0xFF, 0xFF), c(0xEF, 0xF5, 0xF1),
            c(0xDE, 0xEB, 0xE2), c(0xC7, 0xD9, 0xCD), c(0x24, 0x6A, 0x45),
        ),
        (P::Crimson, true) => palette_colors(
            c(0x20, 0x15, 0x18), c(0x17, 0x0D, 0x10), c(0x2B, 0x1C, 0x20), c(0x3A, 0x24, 0x29),
            c(0x4A, 0x2D, 0x33), c(0x5E, 0x3A, 0x41), c(0xF0, 0x9A, 0xA4),
        ),
        (P::Crimson, false) => palette_colors(
            c(0xFA, 0xF2, 0xF3), c(0xF1, 0xE6, 0xE8), c(0xFF, 0xFF, 0xFF), c(0xF7, 0xEC, 0xED),
            c(0xEC, 0xDD, 0xDF), c(0xD9, 0xC7, 0xCA), c(0x9B, 0x32, 0x42),
        ),
        (P::Solarized, true) => palette_colors(
            c(0x00, 0x2B, 0x36), c(0x00, 0x22, 0x2B), c(0x07, 0x36, 0x42), c(0x0B, 0x3F, 0x4B),
            c(0x16, 0x50, 0x5B), c(0x2A, 0x5B, 0x64), c(0x26, 0x8B, 0xD2),
        ),
        (P::Solarized, false) => palette_colors(
            c(0xFD, 0xF6, 0xE3), c(0xF5, 0xEF, 0xDC), c(0xFF, 0xFB, 0xEF), c(0xF5, 0xEF, 0xDC),
            c(0xEE, 0xE8, 0xD5), c(0xD8, 0xD1, 0xBF), c(0x26, 0x8B, 0xD2),
        ),
        (P::Porcelain, true) => palette_colors(
            c(0x20, 0x20, 0x20), c(0x17, 0x17, 0x17), c(0x2A, 0x2A, 0x2A), c(0x36, 0x36, 0x36),
            c(0x44, 0x44, 0x44), c(0x57, 0x57, 0x57), c(0xD8, 0xC6, 0xA4),
        ),
        (P::Porcelain, false) => palette_colors(
            c(0xF7, 0xF6, 0xF2), c(0xED, 0xEC, 0xE7), c(0xFF, 0xFF, 0xFC), c(0xF3, 0xF2, 0xED),
            c(0xE8, 0xE7, 0xE0), c(0xD5, 0xD4, 0xCD), c(0x6D, 0x5D, 0x46),
        ),
        (P::Cyberpunk, true) => palette_colors(
            c(0x1C, 0x16, 0x23), c(0x13, 0x0F, 0x19), c(0x25, 0x1C, 0x2D), c(0x32, 0x24, 0x3B),
            c(0x42, 0x2D, 0x4B), c(0x56, 0x3D, 0x60), c(0xE0, 0x9A, 0xCE),
        ),
        (P::Cyberpunk, false) => palette_colors(
            c(0xF8, 0xF2, 0xF8), c(0xEF, 0xE7, 0xF0), c(0xFF, 0xFC, 0xFF), c(0xF5, 0xEC, 0xF5),
            c(0xEA, 0xDE, 0xEB), c(0xD9, 0xC9, 0xDC), c(0x8A, 0x3F, 0x79),
        ),
        _ => unreachable!("palette has no variable color set"),
    }
}

pub fn parse_hex_color(value: &str) -> Option<Color32> {
    let hex = value.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() { return None; }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color32::from_rgb(r, g, b))
}

pub fn hex_color(color: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b())
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

/// WCAG relative luminance and contrast ratio.
pub fn luminance(c: Color32) -> f32 {
    let f = |v: u8| {
        let s = v as f32 / 255.0;
        if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
}

pub fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Moves an accent colour toward white (dark theme) or black (light theme) until it reads as text on every surface in `bgs`.
pub fn readable_accent(accent: Color32, bgs: &[Color32], dark: bool) -> Color32 {
    readable_color(accent, bgs, dark, 4.5)
}

fn readable_color(accent: Color32, bgs: &[Color32], dark: bool, minimum: f32) -> Color32 {
    let toward = if dark { Color32::WHITE } else { Color32::BLACK };
    (0..=20).map(|i| mix(accent, toward, i as f32 * 0.05)).find(|c| bgs.iter().all(|bg| contrast_ratio(*c, *bg) >= minimum)).unwrap_or(toward)
}

/// Fill and text colour for a button in the accent colour: white or near-black text, whichever reads at 4.5:1, and
/// when neither does (mid-tone accents) the fill is darkened until white text does.
pub fn action_colors(accent: Color32) -> (Color32, Color32) {
    let ink = c(0x10, 0x10, 0x10);
    if contrast_ratio(Color32::WHITE, accent) >= 4.5 {
        (accent, Color32::WHITE)
    } else if contrast_ratio(ink, accent) >= 4.5 {
        (accent, ink)
    } else {
        let fill = (1..=20).map(|i| mix(accent, Color32::BLACK, i as f32 * 0.03)).find(|c| contrast_ratio(Color32::WHITE, *c) >= 4.5).unwrap_or(Color32::BLACK);
        (fill, Color32::WHITE)
    }
}

fn style_id() -> egui::Id {
    egui::Id::unique("orion-style")
}

fn accent_id() -> egui::Id {
    egui::Id::unique("orion-accent")
}

fn palette_id() -> egui::Id {
    egui::Id::unique("orion-palette")
}

/// Tokens for the running theme.
pub fn tokens(ctx: &egui::Context) -> Tokens {
    let style = ctx.data(|d| d.get_temp::<Style>(style_id())).unwrap_or_default();
    let accent = ctx.data(|d| d.get_temp::<Color32>(accent_id())).unwrap_or(DEFAULT_ACCENT);
    let palette = ctx.data(|d| d.get_temp::<Palette>(palette_id())).unwrap_or_default();
    Tokens::of(ctx.global_style().visuals.dark_mode, style, accent).with_palette(palette, accent)
}

// region Windows settings
/// True when Windows is set to dark mode for apps.
pub fn system_dark() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.get_value::<u32, _>("AppsUseLightTheme"))
        .map(|v| v == 0)
        .unwrap_or(true)
}

/// The accent colour from Windows personalization, or the stock blue when the registry has none that is usable.
/// Some PCs store 0 here (accent never set, or stripped by a debloat tool); that falls back to the personalization
/// palette and then to the stock blue.
pub fn system_accent() -> Color32 {
    use winreg::enums::HKEY_CURRENT_USER;
    let root = winreg::RegKey::predef(HKEY_CURRENT_USER);
    let direct = root.open_subkey(r"Software\Microsoft\Windows\DWM").and_then(|k| k.get_value::<u32, _>("AccentColor")).ok().and_then(accent_from_abgr);
    let palette = || {
        let raw = root.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent").and_then(|k| k.get_raw_value("AccentPalette")).ok()?;
        accent_from_palette(&raw.bytes)
    };
    direct.or_else(palette).unwrap_or(DEFAULT_ACCENT)
}

/// The registry stores the accent as 0xAABBGGRR. A zero alpha, or a colour that is nearly black or white, means
/// "not set" and gives `None`.
pub fn accent_from_abgr(v: u32) -> Option<Color32> {
    let col = Color32::from_rgb((v & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, ((v >> 16) & 0xFF) as u8);
    usable_accent(col, (v >> 24) as u8)
}

/// AccentPalette is eight RGBA colours; the fourth (index 3) is the base accent.
pub fn accent_from_palette(bytes: &[u8]) -> Option<Color32> {
    let e = bytes.get(12..16)?;
    usable_accent(Color32::from_rgb(e[0], e[1], e[2]), e[3])
}

fn usable_accent(col: Color32, alpha: u8) -> Option<Color32> {
    let l = luminance(col);
    (alpha != 0 && (0.02..0.85).contains(&l)).then_some(col)
}

/// Whether the Segoe Fluent Icons font is installed (Windows 11).
pub fn icons_available() -> bool {
    std::path::Path::new(r"C:\Windows\Fonts\SegoeIcons.ttf").exists()
}
// endregion

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
    if load("icons", "SegoeIcons.ttf") {
        fonts.families.insert(icons(), vec!["icons".into()]);
    }
    ctx.set_fonts(fonts);
}

/// Builds the whole egui style from the tokens: tonal surfaces, quiet strokes, rounded controls, a visible
/// focus ring, floating scroll bars and the type scale.
pub fn apply(ctx: &egui::Context, appearance: Appearance) {
    let Appearance { dark, reduce_motion, style, palette, accent, density, scale } = appearance;
    let t = Tokens::of(dark, style, accent).with_palette(palette, accent);
    ctx.data_mut(|d| {
        d.insert_temp(motion_id(), reduce_motion);
        d.insert_temp(style_id(), style);
        d.insert_temp(accent_id(), accent);
        d.insert_temp(palette_id(), palette);
    });
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = if t.fluent { t.rail } else { t.canvas };
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

    let r = CornerRadius::same(t.ctl_radius());
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = t.surface1;
    w.noninteractive.weak_bg_fill = t.surface1;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.hairline);
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = t.surface2;
    w.inactive.weak_bg_fill = t.surface2;
    w.inactive.bg_stroke = if t.fluent { Stroke::new(1.0, t.hairline) } else { Stroke::NONE };
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
        match density {
            Density::Comfortable => {
                s.spacing.item_spacing = vec2(S2, S2);
                s.spacing.button_padding = vec2(S3, 7.0);
                s.spacing.interact_size = vec2(36.0, 32.0);
                s.spacing.window_margin = egui::Margin::same(S4 as i8);
                s.spacing.menu_margin = egui::Margin::same(S2 as i8);
            }
            Density::Compact => {
                s.spacing.item_spacing = vec2(S1, S1);
                s.spacing.button_padding = vec2(S2, 5.0);
                s.spacing.interact_size = vec2(32.0, 28.0);
                s.spacing.window_margin = egui::Margin::same(S3 as i8);
                s.spacing.menu_margin = egui::Margin::same(S1 as i8);
            }
        }
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
    ctx.set_zoom_factor(scale.factor());
}

#[cfg(test)]
mod tests {
    use super::*;

    const AA: f32 = 4.5;
    const ACCENTS: [Color32; 8] = [
        DEFAULT_ACCENT,
        Color32::from_rgb(0xFF, 0xB9, 0x00), // gold
        Color32::from_rgb(0xFF, 0x8C, 0x00), // orange
        Color32::from_rgb(0xE7, 0x48, 0x56), // red
        Color32::from_rgb(0xE3, 0x00, 0x8C), // magenta
        Color32::from_rgb(0x88, 0x17, 0x98), // purple
        Color32::from_rgb(0x00, 0xB2, 0x94), // teal
        Color32::from_rgb(0x10, 0x7C, 0x10), // green
    ];

    fn all() -> Vec<Tokens> {
        let mut v = vec![Tokens::DARK, Tokens::LIGHT, Tokens::MATERIAL_DARK, Tokens::MATERIAL_LIGHT];
        for a in ACCENTS {
            v.push(Tokens::fluent(true, a));
            v.push(Tokens::fluent(false, a));
        }
        v
    }

    #[test]
    fn density_and_scale_update_the_global_egui_style() {
        let ctx = egui::Context::default();
        apply(
            &ctx,
            Appearance {
                dark: true,
                reduce_motion: false,
                style: Style::Windows,
                palette: Palette::Midnight,
                accent: BRAND,
                density: Density::Comfortable,
                scale: UiScale::Default,
            },
        );
        let comfortable_spacing = ctx.global_style().spacing.item_spacing;
        apply(
            &ctx,
            Appearance {
                dark: true,
                reduce_motion: false,
                style: Style::Windows,
                palette: Palette::Midnight,
                accent: BRAND,
                density: Density::Compact,
                scale: UiScale::Larger,
            },
        );
        let compact_spacing = ctx.global_style().spacing.item_spacing;
        assert!(compact_spacing.x < comfortable_spacing.x);
        assert!(compact_spacing.y < comfortable_spacing.y);
        assert_eq!(UiScale::Small.factor(), 0.9);
        assert_eq!(UiScale::Default.factor(), 1.0);
        assert_eq!(UiScale::Large.factor(), 1.1);
        assert_eq!(UiScale::Larger.factor(), 1.25);
    }

    fn surfaces(t: &Tokens) -> [(&'static str, Color32); 5] {
        [("canvas", t.canvas), ("rail", t.rail), ("surface1", t.surface1), ("surface2", t.surface2), ("surface3", t.surface3)]
    }

    #[test]
    fn body_and_muted_text_meet_wcag_aa_on_every_surface() {
        for t in all() {
            for (name, s) in surfaces(&t) {
                assert!(contrast_ratio(t.text, s) >= 7.0, "text on {name} (dark={}): {:.2}", t.dark, contrast_ratio(t.text, s));
                assert!(contrast_ratio(t.muted, s) >= AA, "muted on {name} (dark={}): {:.2}", t.dark, contrast_ratio(t.muted, s));
            }
        }
    }

    #[test]
    fn subtle_text_meets_aa_on_the_surfaces_it_is_used_on() {
        for t in all() {
            for (name, s) in [("canvas", t.canvas), ("rail", t.rail), ("surface1", t.surface1), ("surface2", t.surface2)] {
                assert!(contrast_ratio(t.subtle, s) >= AA, "subtle on {name} (dark={}): {:.2}", t.dark, contrast_ratio(t.subtle, s));
            }
        }
    }

    #[test]
    fn brand_text_and_tones_are_readable() {
        for t in all() {
            for (name, s) in [("canvas", t.canvas), ("surface1", t.surface1), ("surface2", t.surface2)] {
                assert!(contrast_ratio(t.brand_text, s) >= AA, "brand text on {name} (dark={}): {:.2}", t.dark, contrast_ratio(t.brand_text, s));
            }
            for tone in [Tone::Neutral, Tone::Brand, Tone::Good, Tone::Warn, Tone::Bad] {
                let (bg, fg) = t.tone(tone);
                assert!(contrast_ratio(fg, bg) >= AA, "{tone:?} chip (dark={}): {:.2}", t.dark, contrast_ratio(fg, bg));
            }
        }
    }

    #[test]
    fn primary_button_text_is_readable_in_every_state() {
        for t in all() {
            let on = t.on_action;
            assert!(contrast_ratio(on, t.action) >= 4.5, "rest (material={}, dark={}): {:.2}", t.pill, t.dark, contrast_ratio(on, t.action));
            assert!(contrast_ratio(on, mix(t.action, t.state_target(), t.hover_lift())) >= 4.5, "hover (material={}, dark={})", t.pill, t.dark);
            assert!(contrast_ratio(on, mix(t.action, t.state_target(), 0.14)) >= 4.5, "pressed (material={}, dark={})", t.pill, t.dark);
        }
    }

    #[test]
    fn focus_and_selection_accent_is_visible_against_surfaces() {
        // WCAG 1.4.11: non-text indicators need 3:1
        for t in all() {
            for (name, s) in [("canvas", t.canvas), ("surface1", t.surface1)] {
                let ring = t.ring();
                assert!(contrast_ratio(ring, s) >= 3.0, "focus ring on {name} (dark={}): {:.2}", t.dark, contrast_ratio(ring, s));
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
    fn the_right_palette_is_chosen() {
        assert_eq!(Tokens::of(true, Style::Classic, DEFAULT_ACCENT), Tokens::DARK);
        assert_eq!(Tokens::of(false, Style::Material, DEFAULT_ACCENT), Tokens::MATERIAL_LIGHT);
        assert!(Tokens::of(true, Style::Material, DEFAULT_ACCENT).pill && !Tokens::of(true, Style::Classic, DEFAULT_ACCENT).pill);
        assert!(Tokens::MATERIAL_DARK.ctl_radius() > Tokens::DARK.ctl_radius());
        let f = Tokens::of(true, Style::Windows, DEFAULT_ACCENT);
        assert!(f.fluent && f.ctl_radius() == 4 && f.card_radius() == 8 && f.brand == DEFAULT_ACCENT);
    }

    #[test]
    fn every_named_palette_keeps_text_and_focus_contrast() {
        for palette in Palette::ALL {
            for requested_dark in [true, false] {
                let dark = palette.forced_dark().unwrap_or(requested_dark);
                for style in Style::ALL {
                    let shape = Tokens::of(dark, style, DEFAULT_ACCENT);
                    let t = shape.with_palette(palette, DEFAULT_ACCENT);
                    assert_eq!(t.fluent, shape.fluent, "{} changed Fluent shape", palette.label());
                    assert_eq!(t.pill, shape.pill, "{} changed pill shape", palette.label());
                    for (name, surface) in surfaces(&t) {
                        assert!(contrast_ratio(t.text, surface) >= 7.0, "{} text on {name}: {:.2}", palette.label(), contrast_ratio(t.text, surface));
                        assert!(contrast_ratio(t.muted, surface) >= AA, "{} muted on {name}: {:.2}", palette.label(), contrast_ratio(t.muted, surface));
                    }
                    for surface in [t.canvas, t.surface1, t.surface2] {
                        assert!(contrast_ratio(t.brand_text, surface) >= AA, "{} brand text: {:.2}", palette.label(), contrast_ratio(t.brand_text, surface));
                        assert!(contrast_ratio(t.ring(), surface) >= 3.0, "{} focus ring: {:.2}", palette.label(), contrast_ratio(t.ring(), surface));
                    }
                    assert!(contrast_ratio(t.on_action, t.action) >= AA, "{} action text: {:.2}", palette.label(), contrast_ratio(t.on_action, t.action));
                }
            }
        }
    }

    #[test]
    fn named_palettes_keep_tonal_surface_order() {
        for palette in Palette::ALL {
            for requested_dark in [true, false] {
                let dark = palette.forced_dark().unwrap_or(requested_dark);
                let t = Tokens::of(dark, Style::Classic, DEFAULT_ACCENT).with_palette(palette, DEFAULT_ACCENT);
                let (rail, canvas, s1, s2, s3) = (luminance(t.rail), luminance(t.canvas), luminance(t.surface1), luminance(t.surface2), luminance(t.surface3));
                if palette == Palette::MaterialLight {
                    assert!(rail < s1 && s1 < canvas && s1 > s2 && s2 > s3, "{} tonal containers", palette.label());
                } else if dark {
                    assert!(rail <= canvas && canvas < s1 && s1 < s2 && s2 < s3, "{} dark surfaces", palette.label());
                } else {
                    assert!(rail < canvas && canvas < s1 && s1 > s2 && s2 > s3, "{} light surfaces", palette.label());
                }
            }
        }
    }

    #[test]
    fn fixed_palette_variants_select_their_named_mode() {
        assert_eq!(Palette::FluentDark.forced_dark(), Some(true));
        assert_eq!(Palette::FluentLight.forced_dark(), Some(false));
        assert_eq!(Palette::MaterialDark.forced_dark(), Some(true));
        assert_eq!(Palette::MaterialLight.forced_dark(), Some(false));
        assert_eq!(Palette::Nord.forced_dark(), None);
    }

    #[test]
    fn custom_accent_hex_round_trips_and_rejects_invalid_input() {
        let color = Color32::from_rgb(0x12, 0xAB, 0xEF);
        assert_eq!(hex_color(color), "#12ABEF");
        assert_eq!(parse_hex_color("#12abef"), Some(color));
        assert_eq!(parse_hex_color("12abef"), None);
        assert_eq!(parse_hex_color("#12ab"), None);
        assert_eq!(parse_hex_color("#12aéef"), None);
    }

    #[test]
    fn windows_accent_is_read_from_the_registry_layout() {
        // 0xAABBGGRR: the stock blue #0078D4 is stored as 0xFFD47800
        assert_eq!(accent_from_abgr(0xFFD4_7800), Some(DEFAULT_ACCENT));
        assert_eq!(accent_from_abgr(0xFF00_00FF), Some(Color32::from_rgb(0xFF, 0, 0)));
    }

    #[test]
    fn an_unset_or_extreme_accent_is_ignored() {
        assert_eq!(accent_from_abgr(0), None, "0 is what a PC with no accent stores");
        assert_eq!(accent_from_abgr(0xFF00_0000), None, "black");
        assert_eq!(accent_from_abgr(0xFFFF_FFFF), None, "white");
        assert_eq!(accent_from_abgr(0x00D4_7800), None, "alpha 0");
        let mut palette = vec![0u8; 32];
        palette[12..16].copy_from_slice(&[0x00, 0x78, 0xD4, 0xFF]);
        assert_eq!(accent_from_palette(&palette), Some(DEFAULT_ACCENT));
        assert_eq!(accent_from_palette(&[0u8; 32]), None);
        assert_eq!(accent_from_palette(&[1, 2, 3]), None, "too short");
    }

    #[test]
    fn readable_accent_meets_contrast_for_every_windows_accent() {
        for a in ACCENTS {
            for dark in [true, false] {
                let t = Tokens::fluent(dark, a);
                for bg in [t.canvas, t.surface1, t.surface2] {
                    assert!(contrast_ratio(t.brand_text, bg) >= 4.5, "accent {a:?} dark={dark}: {:.2}", contrast_ratio(t.brand_text, bg));
                }
                assert!(contrast_ratio(t.on_action, t.action) >= 4.5, "button text on accent {a:?}: {:.2}", contrast_ratio(t.on_action, t.action));
            }
        }
    }

    #[test]
    fn mix_and_contrast_math() {
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 0.5), Color32::from_rgb(128, 128, 128));
        assert!((contrast_ratio(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(ease_out(0.5) > 0.5, "ease-out leads");
    }
}
