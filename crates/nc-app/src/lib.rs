//! The `nca-rs` read-only NC viewer shell.
//!
//! The library owns the shell, theme, layout, source model, local file browser,
//! and the 2D/3D canvas. It never connects to a controller, never writes
//! G-code, and never sends motion.
//!
//! The binary calls [`run`] with [`LaunchOptions`]. Tests exercise the pure
//! layout, search, recent-file, legend, and render-plan models without opening
//! a window, so `cargo test -p nc-app` works without a display.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod analysis_ui;
mod analysis_worker;
mod file_browser;
mod layout;
mod program;
mod setup;
mod shell;
mod sim;
mod sim_worker;
mod state;
mod theme;
mod viewer;

use std::path::PathBuf;

pub use analysis_ui::{AnalysisPanel, AnalysisState, ReviewOptions};

use gpui_kit::{
    AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, application,
    open_window, point, px, size,
};

pub use state::{AppMode, LoadedProgram, NcaConfig, NcaState, ParseState};
pub use theme::ThemeMode;

/// Product name used by the window title and status text.
pub const PRODUCT_NAME: &str = "nca-rs";

/// Launch options assembled from command-line flags.
#[derive(Clone, Debug)]
pub struct LaunchOptions {
    /// Initial window width in logical pixels.
    pub width: f32,
    /// Initial window height in logical pixels.
    pub height: f32,
    /// One-run theme override.
    pub theme_override: Option<ThemeMode>,
    /// Optional local program to open at startup.
    pub open_path: Option<PathBuf>,
    /// Start fullscreen for screenshot review.
    pub fullscreen: bool,
    /// Start in the 3D orbit view.
    pub view_3d: bool,
    /// Open the in-app file browser at startup (review convenience).
    pub open_browser: bool,
    /// Start in SIMULATE.
    pub simulate: bool,
    /// Optional initial SIMULATE position as a fraction of the timeline.
    pub sim_at: Option<f32>,
    /// Start SIMULATE playback immediately (review convenience).
    pub autoplay: bool,
    /// Analysis review flags.
    pub review: ReviewOptions,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            width: 1280.0,
            height: 800.0,
            theme_override: None,
            open_path: None,
            fullscreen: false,
            view_3d: false,
            open_browser: false,
            simulate: false,
            sim_at: None,
            autoplay: false,
            review: ReviewOptions::default(),
        }
    }
}

impl LaunchOptions {
    /// Parses the supported flags.
    ///
    /// Supported: `--size WxH` or `--size=WxH`, `--portrait`,
    /// `--light`, `--dark`, `--high-contrast`, `--fullscreen`, `--3d`,
    /// `--browser`, `--simulate`, `--sim-at FRACTION`, `--play`,
    /// `--analyze`, `--panel diagnostics|coverage`, `--select-rule ID`,
    /// `--setup`, `--export`, and `--open PATH` or `--open=PATH`.
    ///
    /// `--sim-at` and `--play` imply `--simulate`; the fraction is clamped to
    /// `0.0..=1.0`. `--analyze` opens the diagnostics panel unless another
    /// panel is requested.
    #[must_use]
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Self {
        let args = args.into_iter().collect::<Vec<_>>();
        let (width, height) = window_size_override(&args).unwrap_or((1280.0, 800.0));
        let sim_at = sim_at_argument(&args);
        let analyze = args.iter().any(|arg| arg == "--analyze");
        let panel = panel_argument(&args).or(analyze.then_some(AnalysisPanel::Diagnostics));
        Self {
            width,
            height,
            theme_override: theme::launch_override(args.clone()),
            open_path: open_argument(&args),
            fullscreen: args.iter().any(|arg| arg == "--fullscreen"),
            view_3d: args.iter().any(|arg| arg == "--3d"),
            open_browser: args.iter().any(|arg| arg == "--browser"),
            simulate: args.iter().any(|arg| arg == "--simulate")
                || sim_at.is_some()
                || args.iter().any(|arg| arg == "--play"),
            sim_at,
            autoplay: args.iter().any(|arg| arg == "--play"),
            review: ReviewOptions {
                analyze,
                panel,
                select_rule: value_argument(&args, "--select-rule"),
                setup: args.iter().any(|arg| arg == "--setup"),
                export: args.iter().any(|arg| arg == "--export"),
            },
        }
    }
}

/// Runs the desktop shell.
pub fn run(options: LaunchOptions) {
    let cached = NcaConfig::load();
    let theme = options.theme_override.unwrap_or(cached.theme);
    let open_path = options.open_path.clone();
    let view_3d = options.view_3d;
    let open_browser = options.open_browser;
    let simulate = options.simulate;
    let sim_at = options.sim_at;
    let autoplay = options.autoplay;
    let review = options.review.clone();
    let initial_theme = theme;
    application().run(move |cx| {
        gpui_kit::init(cx);
        theme::apply(theme, cx);
        let bounds = Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(options.width), px(options.height)),
        };
        let window_bounds = if options.fullscreen {
            WindowBounds::Fullscreen(bounds)
        } else {
            WindowBounds::Windowed(Bounds {
                origin: point(px(80.0), px(60.0)),
                size: size(px(options.width), px(options.height)),
            })
        };
        let window_options = WindowOptions {
            window_bounds: Some(window_bounds),
            window_min_size: Some(size(px(480.0), px(360.0))),
            titlebar: Some(TitlebarOptions {
                title: Some(format!("{PRODUCT_NAME} - read-only NC viewer").into()),
                ..Default::default()
            }),
            app_id: Some(PRODUCT_NAME.into()),
            is_resizable: true,
            ..Default::default()
        };
        if let Err(error) = open_window(window_options, cx, |window, cx| {
            cx.new(|cx| {
                shell::NcaShell::new(
                    window,
                    open_path.clone(),
                    view_3d,
                    open_browser,
                    simulate,
                    sim_at,
                    autoplay,
                    review.clone(),
                    initial_theme,
                    cx,
                )
            })
        }) {
            eprintln!("failed to open {PRODUCT_NAME} window: {error}");
            cx.quit();
        }
    });
}

fn window_size_override(args: &[String]) -> Option<(f32, f32)> {
    if let Some(size) = size_argument(args) {
        return Some(size);
    }
    if args.iter().any(|arg| arg == "--portrait") {
        return Some((1080.0, 1920.0));
    }
    None
}

fn size_argument(args: &[String]) -> Option<(f32, f32)> {
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let raw = if arg == "--size" {
            index += 1;
            args.get(index)?
        } else if let Some(value) = arg.strip_prefix("--size=") {
            value
        } else {
            index += 1;
            continue;
        };
        let (width, height) = raw.split_once(['x', 'X'])?;
        let width = width.parse::<f32>().ok()?;
        let height = height.parse::<f32>().ok()?;
        if width >= 320.0 && height >= 320.0 {
            return Some((width, height));
        }
        return None;
    }
    None
}

fn open_argument(args: &[String]) -> Option<PathBuf> {
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--open" {
            index += 1;
            return args.get(index).map(PathBuf::from);
        }
        if let Some(value) = arg.strip_prefix("--open=") {
            return Some(PathBuf::from(value));
        }
        index += 1;
    }
    None
}

/// Reads one `--name VALUE` or `--name=VALUE` string.
fn value_argument(args: &[String], name: &str) -> Option<String> {
    let mut index = 0;
    let prefix = format!("{name}=");
    while index < args.len() {
        let arg = &args[index];
        if arg == name {
            index += 1;
            return args.get(index).cloned();
        }
        if let Some(value) = arg.strip_prefix(&prefix) {
            return Some(value.to_string());
        }
        index += 1;
    }
    None
}

fn panel_argument(args: &[String]) -> Option<AnalysisPanel> {
    value_argument(args, "--panel").and_then(|value| AnalysisPanel::parse(&value))
}

fn sim_at_argument(args: &[String]) -> Option<f32> {
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let raw = if arg == "--sim-at" {
            index += 1;
            args.get(index)?
        } else if let Some(value) = arg.strip_prefix("--sim-at=") {
            value
        } else {
            index += 1;
            continue;
        };
        let fraction = raw.parse::<f32>().ok()?;
        if !fraction.is_finite() {
            return None;
        }
        return Some(fraction.clamp(0.0, 1.0));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_flag_accepts_both_spellings() {
        let args = ["--size", "1280x800"].map(str::to_string);
        assert_eq!(size_argument(&args), Some((1280.0, 800.0)));
        let args = ["--size=1920X1080"].map(str::to_string);
        assert_eq!(size_argument(&args), Some((1920.0, 1080.0)));
    }

    #[test]
    fn size_flag_rejects_tiny_or_malformed_values() {
        let args = ["--size", "200x200"].map(str::to_string);
        assert_eq!(size_argument(&args), None);
        let args = ["--size", "wide"].map(str::to_string);
        assert_eq!(size_argument(&args), None);
        let args = ["--size"].map(str::to_string);
        assert_eq!(size_argument(&args), None);
    }

    #[test]
    fn portrait_shorthand_is_overridden_by_size() {
        let args = ["--portrait"].map(str::to_string);
        assert_eq!(window_size_override(&args), Some((1080.0, 1920.0)));
        let args = ["--portrait", "--size", "1024x600"].map(str::to_string);
        assert_eq!(window_size_override(&args), Some((1024.0, 600.0)));
    }

    #[test]
    fn open_flag_accepts_both_spellings() {
        let args = ["--open", "/tmp/part.nc"].map(str::to_string);
        assert_eq!(open_argument(&args), Some(PathBuf::from("/tmp/part.nc")));
        let args = ["--open=/tmp/a.gcode"].map(str::to_string);
        assert_eq!(open_argument(&args), Some(PathBuf::from("/tmp/a.gcode")));
    }

    #[test]
    fn simulate_flags_parse_and_clamp() {
        let options = LaunchOptions::from_args(["--simulate"].map(str::to_string));
        assert!(options.simulate);
        assert_eq!(options.sim_at, None);

        let options = LaunchOptions::from_args(["--sim-at", "0.25"].map(str::to_string));
        assert!(options.simulate, "--sim-at implies --simulate");
        assert_eq!(options.sim_at, Some(0.25));

        let options = LaunchOptions::from_args(["--sim-at=2.0"].map(str::to_string));
        assert_eq!(options.sim_at, Some(1.0));

        let options = LaunchOptions::from_args(["--simulate", "--3d"].map(str::to_string));
        assert!(options.simulate && options.view_3d);

        let options = LaunchOptions::from_args(["--sim-at", "wide"].map(str::to_string));
        assert!(!options.simulate);
        assert_eq!(options.sim_at, None);

        let options = LaunchOptions::from_args(["--play"].map(str::to_string));
        assert!(options.simulate);
        assert!(options.autoplay);

        let options = LaunchOptions::from_args(["--setup", "--export"].map(str::to_string));
        assert!(options.review.setup);
        assert!(options.review.export);
    }

    #[test]
    fn launch_options_default_to_the_review_size() {
        let options = LaunchOptions::from_args(Vec::new());
        assert_eq!((options.width, options.height), (1280.0, 800.0));
        assert!(options.open_path.is_none());
        assert!(!options.fullscreen);
        assert!(!options.simulate);
        assert_eq!(options.sim_at, None);
        assert_eq!(options.review, ReviewOptions::default());
    }

    #[test]
    fn analysis_review_flags_parse() {
        let options = LaunchOptions::from_args(["--analyze"].map(str::to_string));
        assert!(options.review.analyze);
        assert_eq!(
            options.review.panel,
            Some(AnalysisPanel::Diagnostics),
            "--analyze opens diagnostics when no panel is named"
        );

        let options = LaunchOptions::from_args(
            [
                "--panel",
                "coverage",
                "--select-rule",
                "clearance.rapid_at_depth",
            ]
            .map(str::to_string),
        );
        assert_eq!(options.review.panel, Some(AnalysisPanel::Coverage));
        assert_eq!(
            options.review.select_rule.as_deref(),
            Some("clearance.rapid_at_depth")
        );

        let options = LaunchOptions::from_args(
            ["--panel=diagnostics", "--select-rule=x.id"].map(str::to_string),
        );
        assert_eq!(options.review.panel, Some(AnalysisPanel::Diagnostics));
        assert_eq!(options.review.select_rule.as_deref(), Some("x.id"));

        let options = LaunchOptions::from_args(["--panel", "nope"].map(str::to_string));
        assert_eq!(options.review.panel, None);
    }
}
