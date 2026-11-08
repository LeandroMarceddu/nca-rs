//! Semantic motion taxonomy shared by the parser, viewer, simulator, and rules.
//!
//! [`MoveKind`] is assigned by the parser from the modal command and geometry,
//! never guessed by the renderer. Color is never the only signal: every kind
//! also carries a label, a stroke width, and an optional dash pattern.

use std::fmt;

/// The semantic kind of one motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum MoveKind {
    /// G0 travel.
    #[default]
    Rapid = 0,
    /// Vertical or upward motion that does not cut.
    Retract = 1,
    /// G1 cutting move in XY or 3D.
    Feed = 2,
    /// Vertical downward cut.
    Plunge = 3,
    /// G1 with simultaneous XY and downward Z motion.
    Ramp = 4,
    /// G2 planar arc.
    ArcCw = 5,
    /// G3 planar arc.
    ArcCcw = 6,
    /// G2 arc with travel along the plane normal.
    HelixCw = 7,
    /// G3 arc with travel along the plane normal.
    HelixCcw = 8,
    /// G81/G82 drill cycle.
    Drill = 9,
    /// G73/G83 peck drill cycle.
    Peck = 10,
    /// G85/G86/G89 boring cycle.
    Bore = 11,
    /// G38.x probing move.
    Probe = 12,
}

/// A legend grouping for motion kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MoveFamily {
    /// Rapid and retract travel.
    Travel,
    /// Feed, plunge, and ramp cutting.
    Cutting,
    /// Arc and helix cutting.
    Arcs,
    /// Canned drilling and boring.
    Holes,
    /// Probing.
    Special,
}

impl MoveFamily {
    /// Every family in legend order.
    pub const ALL: [Self; 5] = [
        Self::Travel,
        Self::Cutting,
        Self::Arcs,
        Self::Holes,
        Self::Special,
    ];

    /// Human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Travel => "Travel",
            Self::Cutting => "Cutting",
            Self::Arcs => "Arcs",
            Self::Holes => "Holes",
            Self::Special => "Special",
        }
    }
}

/// Marker drawn at a hole position, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HoleMarker {
    /// A single ring (drill).
    Ring,
    /// A dashed ring (peck).
    DashedRing,
    /// A double ring (bore).
    DoubleRing,
}

/// The visual defaults for one motion kind.
///
/// `color_token` names a theme token rather than carrying a literal color, so
/// dark, light, and high-contrast palettes stay consistent. `dash` is
/// `[on, off]` in screen pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveStyle {
    /// Legend label.
    pub label: &'static str,
    /// Theme color token name.
    pub color_token: &'static str,
    /// Stroke width in screen pixels.
    pub width: f32,
    /// Dash pattern, or `None` for a solid stroke.
    pub dash: Option<[f32; 2]>,
    /// Optional hole marker.
    pub marker: Option<HoleMarker>,
}

impl MoveKind {
    /// Every kind in display/legend order.
    pub const ALL: [Self; 13] = [
        Self::Rapid,
        Self::Retract,
        Self::Feed,
        Self::Plunge,
        Self::Ramp,
        Self::ArcCw,
        Self::ArcCcw,
        Self::HelixCw,
        Self::HelixCcw,
        Self::Drill,
        Self::Peck,
        Self::Bore,
        Self::Probe,
    ];

    /// Returns `true` for moves that remove material by default.
    #[must_use]
    pub const fn is_cutting(self) -> bool {
        matches!(
            self,
            Self::Feed
                | Self::Plunge
                | Self::Ramp
                | Self::ArcCw
                | Self::ArcCcw
                | Self::HelixCw
                | Self::HelixCcw
                | Self::Drill
                | Self::Peck
                | Self::Bore
        )
    }

    /// Returns `true` for G0 travel and retracts.
    #[must_use]
    pub const fn is_rapid(self) -> bool {
        matches!(self, Self::Rapid | Self::Retract)
    }

    /// Returns `true` for a retract.
    #[must_use]
    pub const fn is_retract(self) -> bool {
        matches!(self, Self::Retract)
    }

    /// Returns `true` for any arc or helix.
    #[must_use]
    pub const fn is_arc(self) -> bool {
        matches!(
            self,
            Self::ArcCw | Self::ArcCcw | Self::HelixCw | Self::HelixCcw
        )
    }

    /// Returns `true` for a helical arc.
    #[must_use]
    pub const fn is_helix(self) -> bool {
        matches!(self, Self::HelixCw | Self::HelixCcw)
    }

    /// Returns `true` for a canned-cycle hole kind.
    #[must_use]
    pub const fn is_hole(self) -> bool {
        matches!(self, Self::Drill | Self::Peck | Self::Bore)
    }

    /// Legend family.
    #[must_use]
    pub const fn family(self) -> MoveFamily {
        match self {
            Self::Rapid | Self::Retract => MoveFamily::Travel,
            Self::Feed | Self::Plunge | Self::Ramp => MoveFamily::Cutting,
            Self::ArcCw | Self::ArcCcw | Self::HelixCw | Self::HelixCcw => MoveFamily::Arcs,
            Self::Drill | Self::Peck | Self::Bore => MoveFamily::Holes,
            Self::Probe => MoveFamily::Special,
        }
    }

    /// Stable lowercase label used in reports and legend text.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rapid => "rapid",
            Self::Retract => "retract",
            Self::Feed => "feed",
            Self::Plunge => "plunge",
            Self::Ramp => "ramp",
            Self::ArcCw => "arc cw",
            Self::ArcCcw => "arc ccw",
            Self::HelixCw => "helix cw",
            Self::HelixCcw => "helix ccw",
            Self::Drill => "drill",
            Self::Peck => "peck",
            Self::Bore => "bore",
            Self::Probe => "probe",
        }
    }

    /// Visual defaults.
    #[must_use]
    pub const fn style(self) -> MoveStyle {
        match self {
            Self::Rapid => MoveStyle {
                label: "Rapid",
                color_token: "move.rapid",
                width: 1.2,
                dash: Some([5.0, 4.0]),
                marker: None,
            },
            Self::Retract => MoveStyle {
                label: "Retract",
                color_token: "move.retract",
                width: 1.2,
                dash: Some([2.0, 3.0]),
                marker: None,
            },
            Self::Feed => MoveStyle {
                label: "Feed",
                color_token: "move.feed",
                width: 1.6,
                dash: None,
                marker: None,
            },
            Self::Plunge => MoveStyle {
                label: "Plunge",
                color_token: "move.plunge",
                width: 1.9,
                dash: None,
                marker: None,
            },
            Self::Ramp => MoveStyle {
                label: "Ramp",
                color_token: "move.ramp",
                width: 1.8,
                dash: None,
                marker: None,
            },
            Self::ArcCw => MoveStyle {
                label: "Arc CW",
                color_token: "move.arc_cw",
                width: 1.6,
                dash: None,
                marker: None,
            },
            Self::ArcCcw => MoveStyle {
                label: "Arc CCW",
                color_token: "move.arc_ccw",
                width: 1.6,
                dash: None,
                marker: None,
            },
            Self::HelixCw => MoveStyle {
                label: "Helix CW",
                color_token: "move.helix_cw",
                width: 1.8,
                dash: None,
                marker: None,
            },
            Self::HelixCcw => MoveStyle {
                label: "Helix CCW",
                color_token: "move.helix_ccw",
                width: 1.8,
                dash: None,
                marker: None,
            },
            Self::Drill => MoveStyle {
                label: "Drill",
                color_token: "move.drill",
                width: 2.0,
                dash: None,
                marker: Some(HoleMarker::Ring),
            },
            Self::Peck => MoveStyle {
                label: "Peck",
                color_token: "move.peck",
                width: 2.0,
                dash: None,
                marker: Some(HoleMarker::DashedRing),
            },
            Self::Bore => MoveStyle {
                label: "Bore",
                color_token: "move.bore",
                width: 2.0,
                dash: None,
                marker: Some(HoleMarker::DoubleRing),
            },
            Self::Probe => MoveStyle {
                label: "Probe",
                color_token: "move.probe",
                width: 2.0,
                dash: Some([2.0, 3.0]),
                marker: None,
            },
        }
    }

    /// Short uppercase label for chips.
    #[must_use]
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Rapid => "RAPID",
            Self::Retract => "RETRACT",
            Self::Feed => "FEED",
            Self::Plunge => "PLUNGE",
            Self::Ramp => "RAMP",
            Self::ArcCw => "ARC CW",
            Self::ArcCcw => "ARC CCW",
            Self::HelixCw => "HELIX CW",
            Self::HelixCcw => "HELIX CCW",
            Self::Drill => "DRILL",
            Self::Peck => "PECK",
            Self::Bore => "BORE",
            Self::Probe => "PROBE",
        }
    }
}

impl fmt::Display for MoveKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A non-motion event at a position in the program.
#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationKind {
    /// G4 dwell.
    Dwell {
        /// Dwell duration in seconds after dialect conversion.
        seconds: f64,
    },
    /// M6 tool change.
    ToolChange {
        /// Tool number selected.
        tool: u32,
    },
    /// M3/M4/M5 spindle state.
    Spindle {
        /// `true` when the spindle is on.
        on: bool,
        /// Programmed speed in rpm.
        rpm: f64,
        /// `true` for clockwise (M3).
        clockwise: bool,
    },
    /// M7/M8/M9 coolant state.
    Coolant {
        /// `true` when coolant is on.
        on: bool,
    },
}

impl AnnotationKind {
    /// Stable kind label.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Dwell { .. } => "dwell",
            Self::ToolChange { .. } => "tool change",
            Self::Spindle { .. } => "spindle",
            Self::Coolant { .. } => "coolant",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_complete_style_and_legend_entry() {
        let mut tokens = std::collections::BTreeSet::new();
        for kind in MoveKind::ALL {
            let style = kind.style();
            assert!(!style.label.is_empty(), "{kind:?} label");
            assert!(!style.color_token.is_empty(), "{kind:?} color token");
            assert!(style.width > 0.0, "{kind:?} width");
            assert!(!kind.label().is_empty(), "{kind:?} report label");
            assert!(!kind.short_label().is_empty(), "{kind:?} chip label");
            assert!(tokens.insert(style.color_token), "duplicate color token");
        }
        assert_eq!(MoveKind::ALL.len(), 13);
    }

    #[test]
    fn families_cover_every_kind() {
        let mut covered = 0;
        for family in MoveFamily::ALL {
            for kind in MoveKind::ALL {
                if kind.family() == family {
                    covered += 1;
                }
            }
            assert!(!family.label().is_empty());
        }
        assert_eq!(covered, MoveKind::ALL.len());
    }

    #[test]
    fn cutting_and_travel_sets_are_exclusive() {
        for kind in MoveKind::ALL {
            assert!(!(kind.is_cutting() && kind.is_rapid()), "{kind:?}");
            assert_eq!(kind.is_retract(), kind == MoveKind::Retract);
        }
        assert!(MoveKind::Drill.is_cutting());
        assert!(!MoveKind::Probe.is_cutting());
    }

    #[test]
    fn annotations_have_labels() {
        for kind in [
            AnnotationKind::Dwell { seconds: 1.0 },
            AnnotationKind::ToolChange { tool: 2 },
            AnnotationKind::Spindle {
                on: true,
                rpm: 12000.0,
                clockwise: true,
            },
            AnnotationKind::Coolant { on: true },
        ] {
            assert!(!kind.label().is_empty());
        }
    }
}
