//! Pure viewport layout decisions for the read-only NC shell.
//!
//! The thresholds are copied from the parent shop-floor shell and keyed by
//! width and height, never by a device name:
//!
//! - **Wide** (`>= 1120 px` wide, not portrait): program column, viewer, and
//!   move-legend column.
//! - **Compact** (`< 1120 px` wide, not portrait): program column and viewer;
//!   the legend collapses to an overlay.
//! - **Portrait** (height at least `1.15 x` width): one scrolling column and a
//!   two-row top bar. The viewer comes first and keeps 48 px targets.
//!
//! The module is free of GPUI so every decision is unit-tested without a
//! window. It is deliberately smaller than the parent's six-workspace layout.

/// Shared gap between cards and columns, in logical pixels.
pub const GAP: f32 = 12.0;

/// Width at which the landscape layout gains its third (legend) column.
pub const WIDE_MIN_WIDTH: f32 = 1120.0;

/// Width at which the program column widens for large desktops.
pub const WIDE_LEFT_MIN_WIDTH: f32 = 1600.0;

/// Height/width ratio at or above which the shell reflows to one column.
pub const PORTRAIT_ASPECT: f32 = 1.15;

/// Primary touch target, in logical pixels.
pub const TOUCH_TARGET: f32 = 48.0;

/// One 56 px row for landscape windows.
pub const LANDSCAPE_TOP_BAR_HEIGHT: f32 = 56.0;

/// Two 48 px action rows plus 4 px padding and one 4 px gap.
pub const PORTRAIT_TOP_BAR_HEIGHT: f32 = 2.0 * TOUCH_TARGET + 2.0 * 4.0 + 4.0;

/// Status strip height under the top bar.
pub const STATUS_BAR_HEIGHT: f32 = 34.0;

/// Minimum centre viewer width assumed by the three- and two-column layouts.
///
/// Kept as public layout vocabulary for tests and later panel sizing.
#[allow(dead_code)]
pub const MIN_WORKSPACE_WIDTH: f32 = 320.0;

/// Height bounds for the stacked viewer on a portrait viewport.
pub const PORTRAIT_VIEWER_MIN_HEIGHT: f32 = 380.0;
pub const PORTRAIT_VIEWER_MAX_HEIGHT: f32 = 960.0;
/// Fraction of the viewport height reserved for the viewer in portrait.
pub const PORTRAIT_VIEWER_FRACTION: f32 = 0.50;

/// Height bounds for the program card in portrait.
pub const PORTRAIT_PROGRAM_MIN_HEIGHT: f32 = 320.0;
pub const PORTRAIT_PROGRAM_MAX_HEIGHT: f32 = 560.0;
/// Fraction of the viewport height reserved for the program list in portrait.
pub const PORTRAIT_PROGRAM_FRACTION: f32 = 0.30;

/// Height bounds for the analysis dock panel.
pub const ANALYSIS_PANEL_MIN_HEIGHT: f32 = 180.0;
pub const ANALYSIS_PANEL_MAX_HEIGHT: f32 = 300.0;
/// Fraction of the content height reserved for an open analysis panel.
pub const ANALYSIS_PANEL_FRACTION: f32 = 0.38;

/// Layout family selected for one viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutClass {
    /// Landscape at or above [`WIDE_MIN_WIDTH`]; three columns.
    Wide,
    /// Landscape below [`WIDE_MIN_WIDTH`]; two columns.
    Compact,
    /// Tall viewport; one scrolling column and a two-row top bar.
    Portrait,
}

/// Column arrangement for the workspace content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnLayout {
    /// Program column, viewer, legend column.
    Three,
    /// Program column and viewer; the legend is an overlay.
    Two,
    /// One full-width scrolling stack.
    One,
}

/// Top-bar arrangement for the current viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopBarLayout {
    /// One 56 px row (landscape).
    SingleRow,
    /// Two rows: identity and mode above, actions below.
    TwoRows,
}

/// Resolved layout for one viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellLayout {
    /// Viewport width in logical pixels (at least 1.0).
    pub width: f32,
    /// Viewport height in logical pixels (at least 1.0).
    pub height: f32,
    /// Selected layout family.
    pub class: LayoutClass,
    /// Column arrangement for the workspace.
    pub columns: ColumnLayout,
    /// Top-bar arrangement.
    pub top_bar: TopBarLayout,
    /// Program column width for the two- and three-column layouts.
    pub left_width: f32,
    /// Legend column width for the three-column layout.
    pub right_width: f32,
    /// Top-bar height used by the content area and sheet offset.
    pub top_bar_height: f32,
    /// Fixed viewer height for [`ColumnLayout::One`].
    pub viewer_height: f32,
    /// Fixed program height for [`ColumnLayout::One`].
    pub program_height: f32,
    /// Whether the workspace content lives in a vertical scroller.
    pub content_scroll: bool,
}

impl ShellLayout {
    /// Resolves the layout for a viewport measured in logical pixels.
    #[must_use]
    pub fn for_viewport(width: f32, height: f32) -> Self {
        let width = width.max(1.0);
        let height = height.max(1.0);
        let portrait = height >= width * PORTRAIT_ASPECT;
        let class = if portrait {
            LayoutClass::Portrait
        } else if width >= WIDE_MIN_WIDTH {
            LayoutClass::Wide
        } else {
            LayoutClass::Compact
        };
        let columns = match class {
            LayoutClass::Wide => ColumnLayout::Three,
            LayoutClass::Compact => ColumnLayout::Two,
            LayoutClass::Portrait => ColumnLayout::One,
        };
        let (left_width, right_width) = if width >= WIDE_LEFT_MIN_WIDTH {
            (400.0, 280.0)
        } else {
            (340.0, 240.0)
        };
        let top_bar = if portrait {
            TopBarLayout::TwoRows
        } else {
            TopBarLayout::SingleRow
        };
        let top_bar_height = if portrait {
            PORTRAIT_TOP_BAR_HEIGHT
        } else {
            LANDSCAPE_TOP_BAR_HEIGHT
        };
        Self {
            width,
            height,
            class,
            columns,
            top_bar,
            left_width,
            right_width,
            top_bar_height,
            viewer_height: (height * PORTRAIT_VIEWER_FRACTION)
                .clamp(PORTRAIT_VIEWER_MIN_HEIGHT, PORTRAIT_VIEWER_MAX_HEIGHT),
            program_height: (height * PORTRAIT_PROGRAM_FRACTION)
                .clamp(PORTRAIT_PROGRAM_MIN_HEIGHT, PORTRAIT_PROGRAM_MAX_HEIGHT),
            content_scroll: portrait,
        }
    }

    #[allow(dead_code)]
    /// Width available after the outer page padding.
    #[must_use]
    pub fn content_width(&self) -> f32 {
        (self.width - 2.0 * GAP).max(0.0)
    }

    /// Top offset used by every modal sheet.
    #[must_use]
    pub fn overlay_top(&self) -> f32 {
        self.top_bar_height
    }

    /// Whether the move legend is inline (wide) or collapsed (compact and
    /// portrait).
    #[must_use]
    pub fn legend_inline(&self) -> bool {
        self.class == LayoutClass::Wide
    }

    #[allow(dead_code)]
    /// Width of the centre viewer after the fixed columns and gaps.
    #[must_use]
    pub fn centre_width(&self) -> f32 {
        let rails = match self.columns {
            ColumnLayout::Three => self.left_width + self.right_width + 2.0 * GAP,
            ColumnLayout::Two => self.left_width + GAP,
            ColumnLayout::One => 0.0,
        };
        (self.content_width() - rails).max(0.0)
    }

    #[allow(dead_code)]
    /// True when the centre viewer keeps at least [`MIN_WORKSPACE_WIDTH`].
    #[must_use]
    pub fn fits_without_horizontal_overflow(&self) -> bool {
        self.centre_width() >= MIN_WORKSPACE_WIDTH
    }

    /// Primary action height: 48 px in every layout.
    #[must_use]
    pub const fn touch_target(&self) -> f32 {
        TOUCH_TARGET
    }

    /// Height remaining below the top bar and status strip.
    #[must_use]
    pub fn content_height(&self) -> f32 {
        (self.height - self.top_bar_height - STATUS_BAR_HEIGHT).max(0.0)
    }

    /// Height of an open diagnostics/coverage dock.
    #[must_use]
    pub fn analysis_panel_height(&self) -> f32 {
        (self.content_height() * ANALYSIS_PANEL_FRACTION)
            .clamp(ANALYSIS_PANEL_MIN_HEIGHT, ANALYSIS_PANEL_MAX_HEIGHT)
    }

    /// Number of top-bar action rows.
    #[must_use]
    pub const fn top_bar_rows(&self) -> usize {
        match self.top_bar {
            TopBarLayout::SingleRow => 1,
            TopBarLayout::TwoRows => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_1280x800_is_three_columns_with_a_dominant_viewer() {
        let layout = ShellLayout::for_viewport(1280.0, 800.0);
        assert_eq!(layout.class, LayoutClass::Wide);
        assert_eq!(layout.columns, ColumnLayout::Three);
        assert_eq!(layout.top_bar, TopBarLayout::SingleRow);
        assert_eq!(layout.left_width, 340.0);
        assert_eq!(layout.right_width, 240.0);
        assert!(layout.legend_inline());
        assert!(layout.centre_width() > layout.left_width);
        assert!(layout.fits_without_horizontal_overflow());
    }

    #[test]
    fn compact_1024x600_is_two_columns_and_collapses_the_legend() {
        let layout = ShellLayout::for_viewport(1024.0, 600.0);
        assert_eq!(layout.class, LayoutClass::Compact);
        assert_eq!(layout.columns, ColumnLayout::Two);
        assert!(!layout.legend_inline());
        assert!(layout.centre_width() >= MIN_WORKSPACE_WIDTH);
    }

    #[test]
    fn portrait_1080x1920_is_one_scrolling_column() {
        let layout = ShellLayout::for_viewport(1080.0, 1920.0);
        assert_eq!(layout.class, LayoutClass::Portrait);
        assert_eq!(layout.columns, ColumnLayout::One);
        assert_eq!(layout.top_bar, TopBarLayout::TwoRows);
        assert_eq!(layout.top_bar_height, PORTRAIT_TOP_BAR_HEIGHT);
        assert_eq!(layout.top_bar_rows(), 2);
        assert!(layout.content_scroll);
        assert!(!layout.legend_inline());
        assert_eq!(layout.touch_target(), 48.0);
        assert!(layout.viewer_height >= PORTRAIT_VIEWER_MIN_HEIGHT);
    }

    #[test]
    fn large_desktop_widens_both_rails() {
        let layout = ShellLayout::for_viewport(1920.0, 1080.0);
        assert_eq!(layout.class, LayoutClass::Wide);
        assert_eq!(layout.left_width, 400.0);
        assert_eq!(layout.right_width, 280.0);
    }

    #[test]
    fn analysis_panel_height_stays_bounded() {
        let layout = ShellLayout::for_viewport(1280.0, 800.0);
        let height = layout.analysis_panel_height();
        assert!((ANALYSIS_PANEL_MIN_HEIGHT..=ANALYSIS_PANEL_MAX_HEIGHT).contains(&height));
        assert!(height < layout.content_height());
    }

    #[test]
    fn degenerate_viewport_is_clamped() {
        let layout = ShellLayout::for_viewport(-10.0, 0.0);
        assert_eq!(layout.width, 1.0);
        assert_eq!(layout.height, 1.0);
        assert!(layout.content_width() >= 0.0);
    }

    #[test]
    fn breakpoint_boundary_is_exact() {
        assert_eq!(
            ShellLayout::for_viewport(WIDE_MIN_WIDTH - 0.1, 600.0).class,
            LayoutClass::Compact
        );
        assert_eq!(
            ShellLayout::for_viewport(WIDE_MIN_WIDTH, 600.0).class,
            LayoutClass::Wide
        );
    }
}
