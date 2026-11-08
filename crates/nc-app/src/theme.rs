//! Shop-floor visual language for the read-only NC viewer.
//!
//! The palette is adapted from the parent `ghs-app` shell: the same dark,
//! light, and high-contrast surface hierarchy, 48 px touch targets, and
//! chip/card language. `ghs_config::ThemeMode` is replaced by a local enum so
//! this crate owns no sender configuration.
//!
//! Motion colours are semantic. Every [`MoveKind`] has a distinct token, and
//! the UI always pairs colour with a label and a stroke pattern, because colour
//! is never the only signal.

use gpui_kit::component::{Theme, ThemeMode as ComponentThemeMode};
use gpui_kit::{App, Hsla, px, rgb};
use nc_viewer::{MoveKind, MoveStyle};
use serde::{Deserialize, Serialize};

/// Persisted theme selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    /// Follow the desktop preference. The shell resolves this to dark, which
    /// is the shop-floor default and does not require a platform query.
    #[default]
    System,
    /// Dark shop-floor palette.
    Dark,
    /// Light palette with the same hierarchy.
    Light,
    /// Near-black surfaces, bright borders, maximum text contrast.
    HighContrast,
}

impl ThemeMode {
    /// Every selectable mode, in selector order.
    pub const ALL: [Self; 4] = [Self::System, Self::Dark, Self::Light, Self::HighContrast];

    /// Short uppercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "SYSTEM",
            Self::Dark => "DARK",
            Self::Light => "LIGHT",
            Self::HighContrast => "HIGH CONTRAST",
        }
    }

    /// Stable config key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Dark => "dark",
            Self::Light => "light",
            Self::HighContrast => "high_contrast",
        }
    }

    /// Parses a config key. Unknown keys fall back to [`Self::default`].
    #[must_use]
    pub fn from_key(key: &str) -> Self {
        match key {
            "dark" => Self::Dark,
            "light" => Self::Light,
            "high_contrast" => Self::HighContrast,
            _ => Self::System,
        }
    }

    /// Cycles to the next mode in [`Self::ALL`].
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::System => Self::Dark,
            Self::Dark => Self::Light,
            Self::Light => Self::HighContrast,
            Self::HighContrast => Self::System,
        }
    }

    /// Whether this mode resolves to a dark-toned palette.
    #[must_use]
    pub const fn is_dark(self) -> bool {
        matches!(self, Self::System | Self::Dark | Self::HighContrast)
    }
}

/// Resolved colour palette for one frame.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// Whether this palette uses a dark-toned background.
    pub dark: bool,
    /// Whether this is the maximum-contrast variant.
    pub high_contrast: bool,
    pub bg: Hsla,
    pub panel: Hsla,
    pub panel_alt: Hsla,
    pub raised: Hsla,
    pub raised_hover: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text_dim: Hsla,
    pub text_faint: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub accent_muted: Hsla,
    pub on_accent: Hsla,
    pub danger: Hsla,
    pub warning: Hsla,
    pub success: Hsla,
    pub info: Hsla,
    pub grid_minor: Hsla,
    pub grid_major: Hsla,
    pub scrim: Hsla,
    pub move_rapid: Hsla,
    pub move_retract: Hsla,
    pub move_feed: Hsla,
    pub move_plunge: Hsla,
    pub move_ramp: Hsla,
    pub move_arc_cw: Hsla,
    pub move_arc_ccw: Hsla,
    pub move_helix_cw: Hsla,
    pub move_helix_ccw: Hsla,
    pub move_drill: Hsla,
    pub move_peck: Hsla,
    pub move_bore: Hsla,
    pub move_probe: Hsla,
    pub cut_depth: [Hsla; 5],
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn translucent(hex: u32, alpha: f32) -> Hsla {
    c(hex).alpha(alpha)
}

impl Palette {
    /// Picks the palette for a persisted mode.
    #[must_use]
    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::System | ThemeMode::Dark => Self::dark(),
            ThemeMode::Light => Self::light(),
            ThemeMode::HighContrast => Self::high_contrast(),
        }
    }

    /// Dark shop-floor palette.
    #[must_use]
    pub fn dark() -> Self {
        Self {
            dark: true,
            high_contrast: false,
            bg: c(0x080b0e),
            panel: c(0x10161b),
            panel_alt: c(0x151d24),
            raised: c(0x1b252e),
            raised_hover: c(0x24313c),
            border: c(0x283640),
            border_strong: c(0x3a4c59),
            text: c(0xe9f1f6),
            text_dim: c(0x9baebb),
            text_faint: c(0x657a87),
            accent: c(0x3dd6c0),
            accent_hover: c(0x5ae5d1),
            accent_muted: c(0x123d39),
            on_accent: c(0x04211d),
            danger: c(0xe5484d),
            warning: c(0xf5a524),
            success: c(0x46d17d),
            info: c(0x5aa9ff),
            grid_minor: c(0x1b262e),
            grid_major: c(0x2a3943),
            scrim: translucent(0x000000, 0.62),
            move_rapid: c(0x6c8291),
            move_retract: c(0xa9bac4),
            move_feed: c(0x3dd6c0),
            move_plunge: c(0xf5a524),
            move_ramp: c(0xe0b341),
            move_arc_cw: c(0x5aa9ff),
            move_arc_ccw: c(0xa07cff),
            move_helix_cw: c(0xff6ad5),
            move_helix_ccw: c(0x9b6bff),
            move_drill: c(0x2fd9e8),
            move_peck: c(0x7fd7ff),
            move_bore: c(0x6d8fd1),
            move_probe: c(0xf2d64b),
            cut_depth: [
                c(0x3dd6c0),
                c(0x2fd9e8),
                c(0x5aa9ff),
                c(0xa07cff),
                c(0x9b6bff),
            ],
        }
    }

    /// Light palette with the same contrast hierarchy.
    #[must_use]
    pub fn light() -> Self {
        Self {
            dark: false,
            high_contrast: false,
            bg: c(0xe9eef1),
            panel: c(0xffffff),
            panel_alt: c(0xf2f6f8),
            raised: c(0xf7fafb),
            raised_hover: c(0xe7eef2),
            border: c(0xd3dde3),
            border_strong: c(0xaebec8),
            text: c(0x0c171e),
            text_dim: c(0x4e626f),
            text_faint: c(0x84949d),
            accent: c(0x0c8f7f),
            accent_hover: c(0x0aa394),
            accent_muted: c(0xd5f2ed),
            on_accent: c(0xffffff),
            danger: c(0xc7353a),
            warning: c(0xb87505),
            success: c(0x1d8a4b),
            info: c(0x1868b8),
            grid_minor: c(0xdee7ec),
            grid_major: c(0xc5d2da),
            scrim: translucent(0x000000, 0.38),
            move_rapid: c(0x5d6f79),
            move_retract: c(0x7a8b95),
            move_feed: c(0x0c8f7f),
            move_plunge: c(0xb87505),
            move_ramp: c(0x9a6a00),
            move_arc_cw: c(0x1868b8),
            move_arc_ccw: c(0x6a3fd1),
            move_helix_cw: c(0xb02a83),
            move_helix_ccw: c(0x5b32b8),
            move_drill: c(0x007f8f),
            move_peck: c(0x176c96),
            move_bore: c(0x3f5f9b),
            move_probe: c(0x8a6a00),
            cut_depth: [
                c(0x0c8f7f),
                c(0x007f8f),
                c(0x1868b8),
                c(0x6a3fd1),
                c(0x5b32b8),
            ],
        }
    }

    /// Maximum-contrast dark palette.
    #[must_use]
    pub fn high_contrast() -> Self {
        Self {
            dark: true,
            high_contrast: true,
            bg: c(0x050607),
            panel: c(0x0d1114),
            panel_alt: c(0x141a1f),
            raised: c(0x1b2329),
            raised_hover: c(0x26313a),
            border: c(0x7f9aab),
            border_strong: c(0xcfe0ea),
            text: c(0xffffff),
            text_dim: c(0xe2edf3),
            text_faint: c(0xc3d3dd),
            accent: c(0x00ffd1),
            accent_hover: c(0x66ffe3),
            accent_muted: c(0x003b33),
            on_accent: c(0x00130f),
            danger: c(0xff7878),
            warning: c(0xffd400),
            success: c(0x3dff7a),
            info: c(0x7ccbff),
            grid_minor: c(0x22313a),
            grid_major: c(0x3c5563),
            scrim: translucent(0x000000, 0.78),
            move_rapid: c(0xd7e2e8),
            move_retract: c(0xffffff),
            move_feed: c(0x00ffd1),
            move_plunge: c(0xffd400),
            move_ramp: c(0xffb000),
            move_arc_cw: c(0x8fd0ff),
            move_arc_ccw: c(0xc8a8ff),
            move_helix_cw: c(0xff8ae0),
            move_helix_ccw: c(0xb99cff),
            move_drill: c(0x66f5ff),
            move_peck: c(0xb3e9ff),
            move_bore: c(0x9db8ff),
            move_probe: c(0xfff06a),
            cut_depth: [
                c(0x00ffd1),
                c(0x66f5ff),
                c(0x8fd0ff),
                c(0xc8a8ff),
                c(0xb99cff),
            ],
        }
    }

    /// Applies the GPUI Kit mode and every semantic colour.
    pub fn apply(self, cx: &mut App) {
        Theme::change(
            if self.dark {
                ComponentThemeMode::Dark
            } else {
                ComponentThemeMode::Light
            },
            None,
            cx,
        );
        Theme::update(cx, |theme| {
            theme.font_size = px(15.);
            theme.mono_font_size = px(13.);
            theme.radius = px(10.);
            theme.radius_lg = px(14.);
            theme.colors.background = self.bg;
            theme.colors.foreground = self.text;
            theme.colors.border = self.border;
            theme.colors.input = self.panel_alt;
            theme.colors.primary = self.accent;
            theme.colors.primary_hover = self.accent_hover;
            theme.colors.primary_active = self.accent;
            theme.colors.primary_foreground = self.on_accent;
            theme.colors.button = self.raised;
            theme.colors.button_hover = self.raised_hover;
            theme.colors.button_active = self.raised;
            theme.colors.button_foreground = self.text;
            theme.colors.button_primary = self.accent;
            theme.colors.button_primary_hover = self.accent_hover;
            theme.colors.button_primary_active = self.accent_hover;
            theme.colors.button_primary_foreground = self.on_accent;
            theme.colors.button_secondary = self.raised;
            theme.colors.button_secondary_hover = self.raised_hover;
            theme.colors.button_secondary_active = self.raised;
            theme.colors.button_secondary_foreground = self.text;
            theme.colors.button_danger = self.danger;
            theme.colors.button_danger_hover = self.danger;
            theme.colors.button_danger_active = self.danger;
            theme.colors.button_danger_foreground = if self.high_contrast {
                c(0x050607)
            } else {
                c(0xffffff)
            };
            theme.colors.button_warning = self.warning;
            theme.colors.button_warning_hover = self.warning;
            theme.colors.button_warning_active = self.warning;
            theme.colors.button_warning_foreground = c(0x111111);
            theme.colors.accent = self.accent_muted;
            theme.colors.accent_foreground = self.text;
            theme.colors.secondary = self.raised;
            theme.colors.secondary_hover = self.raised_hover;
            theme.colors.secondary_foreground = self.text;
            theme.colors.muted = self.panel_alt;
            theme.colors.muted_foreground = self.text_dim;
            theme.colors.popover = self.raised;
            theme.colors.popover_foreground = self.text;
            theme.colors.title_bar = self.panel;
            theme.colors.title_bar_border = self.border;
            theme.colors.sidebar = self.panel;
            theme.colors.sidebar_foreground = self.text;
            theme.colors.sidebar_border = self.border;
            theme.colors.danger = self.danger;
            theme.colors.warning = self.warning;
            theme.colors.success = self.success;
            theme.colors.info = self.info;
        });
    }
}

/// Applies a persisted mode.
pub fn apply(mode: ThemeMode, cx: &mut App) {
    Palette::for_mode(mode).apply(cx);
}

/// The palette colour token for one semantic motion kind.
#[must_use]
pub fn move_color(kind: MoveKind, palette: Palette) -> Hsla {
    match kind {
        MoveKind::Rapid => palette.move_rapid,
        MoveKind::Retract => palette.move_retract,
        MoveKind::Feed => palette.move_feed,
        MoveKind::Plunge => palette.move_plunge,
        MoveKind::Ramp => palette.move_ramp,
        MoveKind::ArcCw => palette.move_arc_cw,
        MoveKind::ArcCcw => palette.move_arc_ccw,
        MoveKind::HelixCw => palette.move_helix_cw,
        MoveKind::HelixCcw => palette.move_helix_ccw,
        MoveKind::Drill => palette.move_drill,
        MoveKind::Peck => palette.move_peck,
        MoveKind::Bore => palette.move_bore,
        MoveKind::Probe => palette.move_probe,
    }
}

/// The stable style for one semantic motion kind.
///
/// Thin wrapper over [`MoveKind::style`] so the shell has one import surface
/// for legend rows and painter decisions.
#[must_use]
pub fn move_style(kind: MoveKind) -> MoveStyle {
    kind.style()
}

/// A colour from the cut-depth ramp.
///
/// `fraction` is clamped to `0.0..=1.0`; the ramp runs shallow teal to deep
/// violet. Red is deliberately absent: it stays reserved for errors and unsafe
/// motion.
#[must_use]
pub fn cut_depth_color(fraction: f32, palette: Palette) -> Hsla {
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let last = palette.cut_depth.len() - 1;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let index = (fraction * last as f32).round() as usize;
    palette.cut_depth[index.min(last)]
}

/// Resolves a launch flag that overrides the persisted theme for one run.
#[must_use]
pub fn launch_override(args: impl IntoIterator<Item = String>) -> Option<ThemeMode> {
    let mut selected = None;
    for arg in args {
        match arg.as_str() {
            "--high-contrast" => selected = Some(ThemeMode::HighContrast),
            "--light" if selected != Some(ThemeMode::HighContrast) => {
                selected = Some(ThemeMode::Light);
            }
            "--dark" if selected != Some(ThemeMode::HighContrast) => {
                selected = Some(ThemeMode::Dark);
            }
            _ => {}
        }
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_move_kind_has_a_distinct_colour_in_every_palette() {
        for palette in [Palette::dark(), Palette::light(), Palette::high_contrast()] {
            let mut seen = std::collections::BTreeSet::new();
            for kind in MoveKind::ALL {
                let color = move_color(kind, palette);
                let bits = (
                    color.h.to_bits(),
                    color.s.to_bits(),
                    color.l.to_bits(),
                    color.a.to_bits(),
                );
                assert!(seen.insert(bits), "{kind:?} shares a colour token");
                assert!(!kind.style().label.is_empty());
            }
            assert_eq!(seen.len(), MoveKind::ALL.len());
        }
    }

    #[test]
    fn cut_depth_ramp_is_clamped_and_never_uses_danger() {
        let palette = Palette::dark();
        let shallow = cut_depth_color(-5.0, palette);
        let deep = cut_depth_color(9.0, palette);
        assert_eq!(shallow, palette.cut_depth[0]);
        assert_eq!(deep, palette.cut_depth[4]);
        for color in palette.cut_depth {
            assert_ne!(color, palette.danger);
        }
    }

    #[test]
    fn theme_keys_round_trip() {
        for mode in ThemeMode::ALL {
            assert_eq!(ThemeMode::from_key(mode.key()), mode);
            assert!(!mode.label().is_empty());
        }
        assert_eq!(ThemeMode::from_key("unknown"), ThemeMode::System);
    }

    #[test]
    fn launch_flags_prefer_high_contrast() {
        let args = ["--light", "--high-contrast"].map(str::to_string);
        assert_eq!(launch_override(args), Some(ThemeMode::HighContrast));
        assert_eq!(launch_override(Vec::new()), None);
    }
}
