//! Tool shapes, sparse tool overlays, machine limits, and material presets.
//!
//! Tool data is optional. A bare NC file usually names a tool only by number,
//! so every field except [`Tool::number`] is optional and "unknown" is a first
//! class state. Rules that need a missing field are skipped; no code path
//! invents a default to make a rule run.

use std::collections::{BTreeMap, btree_map};
use std::fmt;

use serde::{Deserialize, Serialize};

/// The geometric shape of a cutter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolShape {
    /// A flat-ended cylinder.
    #[default]
    Flat,
    /// A ball-nose cutter (spherical tip).
    Ball,
    /// A bull-nose cutter (flat tip with corner radius).
    Bull,
    /// A drill (cone tip plus cylinder).
    Drill,
    /// A V-bit (cone).
    VBit,
    /// Shape not known. Rules that need a shape are skipped; simulation uses a
    /// flat cylinder and labels it as assumed.
    Unknown,
}

impl ToolShape {
    /// Short human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Ball => "ball",
            Self::Bull => "bull",
            Self::Drill => "drill",
            Self::VBit => "v-bit",
            Self::Unknown => "unknown",
        }
    }

    /// Parses a shape name from configuration or a tool comment.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "flat" | "endmill" | "end mill" | "square" => Some(Self::Flat),
            "ball" | "ballnose" | "ball-nose" | "ball nose" => Some(Self::Ball),
            "bull" | "bullnose" | "bull-nose" | "bull nose" => Some(Self::Bull),
            "drill" => Some(Self::Drill),
            "vbit" | "v-bit" | "v bit" | "engrave" | "engraving" => Some(Self::VBit),
            "unknown" | "?" => Some(Self::Unknown),
            _ => None,
        }
    }
}

impl fmt::Display for ToolShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A field of [`Tool`] with an individually tracked provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolField {
    /// Display name.
    Name,
    /// Cutter shape.
    Shape,
    /// Cutting diameter.
    Diameter,
    /// Number of cutting edges.
    Flutes,
    /// Tip-to-holder-face length.
    Stickout,
    /// Length of the fluted section.
    FluteLength,
    /// Drill or V-bit included tip angle.
    TipAngle,
    /// Bull-nose corner radius.
    CornerRadius,
    /// Shank diameter.
    ShankDiameter,
    /// Holder diameter.
    HolderDiameter,
    /// Handedness of the cutter.
    RightHand,
}

impl ToolField {
    /// Stable field name used in reports and coverage keys.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Shape => "shape",
            Self::Diameter => "diameter",
            Self::Flutes => "flutes",
            Self::Stickout => "stickout",
            Self::FluteLength => "flute_length",
            Self::TipAngle => "tip_angle",
            Self::CornerRadius => "corner_radius",
            Self::ShankDiameter => "shank_diameter",
            Self::HolderDiameter => "holder_diameter",
            Self::RightHand => "right_hand",
        }
    }

    /// Every field in a stable order.
    pub const ALL: [Self; 11] = [
        Self::Name,
        Self::Shape,
        Self::Diameter,
        Self::Flutes,
        Self::Stickout,
        Self::FluteLength,
        Self::TipAngle,
        Self::CornerRadius,
        Self::ShankDiameter,
        Self::HolderDiameter,
        Self::RightHand,
    ];
}

impl fmt::Display for ToolField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// Where a known tool field came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldSource {
    /// Entered by the user or loaded from a TOML config.
    Config,
    /// Recovered from a comment in the program.
    Comment {
        /// 1-based source line of the comment.
        line: usize,
    },
}

impl FieldSource {
    /// Stable source label used in reports (`"config"` or `"comment:L12"`).
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::Config => "config".to_string(),
            Self::Comment { line } => format!("comment:L{line}"),
        }
    }

    /// Returns `true` for a comment-derived value.
    #[must_use]
    pub const fn is_comment(self) -> bool {
        matches!(self, Self::Comment { .. })
    }
}

/// A sparse tool overlay entry.
///
/// Most programs use tools with no entry at all, and most entries know only a
/// number and maybe a diameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    /// Tool number, the `T` word value.
    pub number: u32,
    /// Display name.
    pub name: Option<String>,
    /// Cutter shape; [`ToolShape::Unknown`] when not known.
    pub shape: ToolShape,
    /// Cutting diameter in millimetres.
    pub diameter_mm: Option<f64>,
    /// Number of flutes.
    pub flutes: Option<u32>,
    /// Tip-to-holder-face length in millimetres.
    pub stickout_mm: Option<f64>,
    /// Fluted length in millimetres.
    pub flute_length_mm: Option<f64>,
    /// Drill/V-bit tip angle in degrees.
    pub tip_angle_deg: Option<f64>,
    /// Bull-nose corner radius in millimetres.
    pub corner_radius_mm: Option<f64>,
    /// Shank diameter in millimetres. `None` means the cutting diameter.
    pub shank_diameter_mm: Option<f64>,
    /// Holder diameter in millimetres. `None` skips holder checks.
    pub holder_diameter_mm: Option<f64>,
    /// Handedness. `None` skips handedness rules.
    pub right_hand: Option<bool>,
    /// Provenance of every known field.
    pub sources: BTreeMap<ToolField, FieldSource>,
}

impl Tool {
    /// Creates an unknown tool with the given number.
    #[must_use]
    pub fn new(number: u32) -> Self {
        Self {
            number,
            name: None,
            shape: ToolShape::Unknown,
            diameter_mm: None,
            flutes: None,
            stickout_mm: None,
            flute_length_mm: None,
            tip_angle_deg: None,
            corner_radius_mm: None,
            shank_diameter_mm: None,
            holder_diameter_mm: None,
            right_hand: None,
            sources: BTreeMap::new(),
        }
    }

    /// Effective shank diameter: explicit, else the cutting diameter.
    #[must_use]
    pub fn effective_shank_diameter_mm(&self) -> Option<f64> {
        self.shank_diameter_mm.or(self.diameter_mm)
    }
}

/// Best-effort tool data a post wrote into a comment. Never authoritative.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolHint {
    /// Tool number the comment refers to.
    pub tool: u32,
    /// 1-based source line of the comment.
    pub line: usize,
    /// Tool name, when recognized.
    pub name: Option<String>,
    /// Tool shape, when recognized.
    pub shape: Option<ToolShape>,
    /// Diameter in millimetres, only when the unit was unambiguous.
    pub diameter_mm: Option<f64>,
    /// Flute count, when recognized.
    pub flutes: Option<u32>,
}

impl ToolHint {
    /// Creates an empty hint for a tool number and source line.
    #[must_use]
    pub fn new(tool: u32, line: usize) -> Self {
        Self {
            tool,
            line,
            name: None,
            shape: None,
            diameter_mm: None,
            flutes: None,
        }
    }
}

/// A sparse tool table sorted by tool number.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolTable {
    entries: BTreeMap<u32, Tool>,
}

impl ToolTable {
    /// Creates an empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` when no tool is known.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Looks up a tool by number.
    #[must_use]
    pub fn get(&self, number: u32) -> Option<&Tool> {
        self.entries.get(&number)
    }

    /// Inserts or replaces a tool.
    pub fn insert(&mut self, tool: Tool) {
        self.entries.insert(tool.number, tool);
    }

    /// Removes a tool, returning it when present.
    pub fn remove(&mut self, number: u32) -> Option<Tool> {
        self.entries.remove(&number)
    }

    /// Iterates entries in tool-number order.
    pub fn iter(&self) -> btree_map::Iter<'_, u32, Tool> {
        self.entries.iter()
    }

    /// Iterates tool numbers in ascending order.
    pub fn numbers(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.keys().copied()
    }

    /// Returns the underlying map.
    #[must_use]
    pub fn as_map(&self) -> &BTreeMap<u32, Tool> {
        &self.entries
    }

    /// Builds a resolved table: user configuration wins per field, comment
    /// hints fill only fields the configuration leaves unknown.
    #[must_use]
    pub fn resolve(config: &Self, hints: &[ToolHint]) -> Self {
        let mut resolved = config.clone();
        for hint in hints {
            let entry = resolved
                .entries
                .entry(hint.tool)
                .or_insert_with(|| Tool::new(hint.tool));
            let source = FieldSource::Comment { line: hint.line };
            if entry.name.is_none()
                && let Some(name) = &hint.name
            {
                entry.name = Some(name.clone());
                entry.sources.insert(ToolField::Name, source);
            }
            if entry.shape == ToolShape::Unknown
                && let Some(shape) = hint.shape
            {
                entry.shape = shape;
                entry.sources.insert(ToolField::Shape, source);
            }
            if entry.diameter_mm.is_none()
                && let Some(diameter) = hint.diameter_mm
            {
                entry.diameter_mm = Some(diameter);
                entry.sources.insert(ToolField::Diameter, source);
            }
            if entry.flutes.is_none()
                && let Some(flutes) = hint.flutes
            {
                entry.flutes = Some(flutes);
                entry.sources.insert(ToolField::Flutes, source);
            }
        }
        resolved
    }

    /// Returns the provenance of one field, when known.
    #[must_use]
    pub fn source_of(&self, number: u32, field: ToolField) -> Option<FieldSource> {
        self.entries
            .get(&number)
            .and_then(|tool| tool.sources.get(&field).copied())
    }
}

impl FromIterator<Tool> for ToolTable {
    fn from_iter<I: IntoIterator<Item = Tool>>(iter: I) -> Self {
        let mut table = Self::new();
        for tool in iter {
            table.insert(tool);
        }
        table
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_wins_over_comment_hint() {
        let mut config = ToolTable::new();
        let mut tool = Tool::new(3);
        tool.diameter_mm = Some(6.0);
        tool.sources
            .insert(ToolField::Diameter, FieldSource::Config);
        config.insert(tool);

        let hint = ToolHint {
            tool: 3,
            line: 4,
            name: Some("6mm flat".into()),
            shape: Some(ToolShape::Flat),
            diameter_mm: Some(3.175),
            flutes: Some(2),
        };
        let resolved = ToolTable::resolve(&config, &[hint]);
        let tool = resolved.get(3).expect("tool");
        assert_eq!(tool.diameter_mm, Some(6.0));
        assert_eq!(
            tool.sources.get(&ToolField::Diameter),
            Some(&FieldSource::Config)
        );
        assert_eq!(tool.flutes, Some(2));
        assert_eq!(
            tool.sources.get(&ToolField::Flutes),
            Some(&FieldSource::Comment { line: 4 })
        );
        assert_eq!(
            resolved.source_of(3, ToolField::Flutes),
            Some(FieldSource::Comment { line: 4 })
        );
    }

    #[test]
    fn empty_table_is_normal() {
        let table = ToolTable::new();
        assert!(table.is_empty());
        assert_eq!(table.len(), 0);
        assert!(table.get(1).is_none());
    }

    #[test]
    fn tools_are_ordered_by_number() {
        let table: ToolTable = [Tool::new(9), Tool::new(2), Tool::new(5)]
            .into_iter()
            .collect();
        assert_eq!(table.numbers().collect::<Vec<_>>(), vec![2, 5, 9]);
    }
}
