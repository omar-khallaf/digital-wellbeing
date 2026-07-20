//! Design-system tokens for the Digital Wellbeing GUI.
//!
//! All colors are sourced from the active [`gpui_component`] theme
//! (`cx.theme()`) so the shell stays consistent with the component library's
//! dark/light modes. We expose a thin ergonomic layer + a spacing/radius scale
//! so every screen shares one visual language instead of ad-hoc `hsla()` calls.

use gpui::*;
use gpui_component::theme::Theme;

/// Spacing scale (px). Use these instead of magic numbers.
/// Returned as `Pixels` because gpui's length methods take `impl Into<DefiniteLength>`.
pub mod sp {
    use gpui::px;

    pub const XS: gpui::Pixels = px(4.0);
    pub const SM: gpui::Pixels = px(8.0);
    pub const MD: gpui::Pixels = px(12.0);
    pub const LG: gpui::Pixels = px(16.0);
    pub const XL: gpui::Pixels = px(24.0);
    pub const X2: gpui::Pixels = px(32.0);
}

/// Radius scale (px). Mirrors the component library's `Theme.radius`.
/// Returned as `Pixels` because gpui's `rounded()` expects `AbsoluteLength`.
pub mod rad {
    use gpui::px;

    pub fn sm() -> gpui::Pixels {
        px(4.0)
    }
    pub fn md() -> gpui::Pixels {
        px(8.0)
    }
    pub fn lg() -> gpui::Pixels {
        px(12.0)
    }
    pub fn full() -> gpui::Pixels {
        px(9999.0)
    }
}

/// Read the live theme from any context that exposes `theme()`.
///
/// `cx` here is anything implementing `gpui_component::theme::Theme`
/// (e.g. `&App`, `&mut App`, `&Window`).
pub fn theme_of(cx: &App) -> &Theme {
    Theme::global(cx)
}

/// Semantic surface used for cards / panels.
pub fn surface(cx: &App) -> Hsla {
    // Slightly lifted from the window background for depth.
    let bg = Theme::global(cx).background;
    lift(bg, 0.04)
}

/// Card border color.
pub fn border(cx: &App) -> Hsla {
    Theme::global(cx).border
}

/// Primary text (headings, values).
pub fn text_primary(cx: &App) -> Hsla {
    Theme::global(cx).foreground
}

/// Secondary text (labels, captions).
pub fn text_secondary(cx: &App) -> Hsla {
    Theme::global(cx).muted
}

/// Muted / tertiary text (hints, disabled).
pub fn text_muted(cx: &App) -> Hsla {
    let fg = Theme::global(cx).foreground;
    with_alpha(fg, 0.45)
}

/// Brand accent (links, active indicators, highlights).
pub fn accent(cx: &App) -> Hsla {
    Theme::global(cx).accent
}

/// Destructive / blocked state.
pub fn danger(cx: &App) -> Hsla {
    Theme::global(cx).danger
}

/// Positive / allowed state.
pub fn success(cx: &App) -> Hsla {
    Theme::global(cx).success
}

/// Warning / caution state.
pub fn warning(cx: &App) -> Hsla {
    Theme::global(cx).warning
}

/// Info / neutral state.
pub fn info(cx: &App) -> Hsla {
    Theme::global(cx).info
}

/// Lift a color's lightness by `amount` (clamped to [0,1]).
fn lift(mut c: Hsla, amount: f32) -> Hsla {
    c.l = (c.l + amount).clamp(0.0, 1.0);
    c
}

/// Scale a color's alpha.
fn with_alpha(mut c: Hsla, a: f32) -> Hsla {
    c.a = a.clamp(0.0, 1.0);
    c
}

/// Build a deterministic, pleasant Hsla from a string seed.
///
/// Used for per-app / per-category series colors where no explicit color is
/// provided. Produces saturated, mid-lightness hues.
fn color_from_str(seed: &str) -> Hsla {
    let hash: u32 = seed
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    let hue = (hash % 360) as f32;
    hsla(hue, 0.62, 0.58, 1.0)
}

/// Resolve a color: explicit hex wins, otherwise a deterministic seed color.
pub fn resolve_color(hex: &str, fallback_seed: &str) -> Hsla {
    if hex.is_empty() {
        return color_from_str(fallback_seed);
    }
    parse_hex(hex).unwrap_or_else(|| color_from_str(fallback_seed))
}

/// Parse `#rrggbb` into `Hsla`. Returns `None` on malformed input.
pub fn parse_hex(hex: &str) -> Option<Hsla> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(rgb_to_hsla(r, g, b))
}

fn rgb_to_hsla(r: u8, g: u8, b: u8) -> Hsla {
    let (rf, gf, bf) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) / 2.0;
    let delta = max - min;
    let h = if delta == 0.0 {
        0.0
    } else if max == rf {
        60.0 * (((gf - bf) / delta) % 6.0)
    } else if max == gf {
        60.0 * (((bf - rf) / delta) + 2.0)
    } else {
        60.0 * (((rf - gf) / delta) + 4.0)
    };
    let s = if delta == 0.0 {
        0.0
    } else if l <= 0.5 {
        delta / (max + min)
    } else {
        delta / (2.0 - max - min)
    };
    hsla(h.rem_euclid(360.0), s, l, 1.0)
}
