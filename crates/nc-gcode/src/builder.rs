//! A hand-built [`Program`] for tests before the real parser emits records.
//!
//! Available under `#[cfg(test)]` and the `test-util` feature. The builder
//! synthesizes a display [`Toolpath`](nc_viewer::Toolpath) from the motion
//! records so both the viewer-facing and record-facing paths can be tested.

use std::collections::BTreeMap;

use nc_core::Position;
use nc_viewer::{MoveKind, Point3, SegmentKind, Toolpath};

use crate::motion::{Annotation, MotionRecord};
use crate::program::{Program, ProgramLine, ProgramStats};
use crate::state::ParserState;

/// Builds a [`Program`] from hand-written motion records.
#[derive(Debug, Default)]
pub struct ProgramBuilder {
    lines: BTreeMap<usize, String>,
    motions: Vec<MotionRecord>,
    annotations: Vec<Annotation>,
    geometry_uncertain_after_line: Option<usize>,
}

impl ProgramBuilder {
    /// Creates an empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a source line.
    #[must_use]
    pub fn source(mut self, line: usize, text: impl Into<String>) -> Self {
        self.lines.insert(line, text.into());
        self
    }

    /// Adds a motion record. A placeholder source line is created when the
    /// record's line has no text yet.
    #[must_use]
    pub fn record(mut self, motion: MotionRecord) -> Self {
        self.lines
            .entry(motion.line)
            .or_insert_with(|| format!("; motion {}", motion.index));
        self.motions.push(motion);
        self
    }

    /// Adds a straight motion on `line`, auto-numbering its index.
    #[must_use]
    pub fn motion(
        self,
        line: usize,
        kind: MoveKind,
        start_mm: [f64; 3],
        end_mm: [f64; 3],
        feed_mm_min: Option<f64>,
    ) -> Self {
        let index = self.motions.len();
        let mut motion = MotionRecord::linear(line, index, kind, start_mm, end_mm);
        motion.feed_mm_min = feed_mm_min;
        self.record(motion)
    }

    /// Adds an annotation at the current end of the motion list.
    #[must_use]
    pub fn annotation(mut self, annotation: Annotation) -> Self {
        self.annotations.push(annotation);
        self
    }

    /// Marks geometry as untrusted after `line`.
    #[must_use]
    pub fn uncertain_after(mut self, line: usize) -> Self {
        self.geometry_uncertain_after_line = Some(line);
        self
    }

    /// Builds the program.
    #[must_use]
    pub fn build(self) -> Program {
        let mut toolpath = Toolpath::new();
        let mut stats = ProgramStats::default();
        let mut motions = Vec::with_capacity(self.motions.len());
        let mut per_line: BTreeMap<usize, (usize, usize)> = BTreeMap::new();

        for (index, mut motion) in self.motions.into_iter().enumerate() {
            motion.index = index;
            let start = toolpath.len();
            let start_point = Point3::new(
                motion.start_mm[0] as f32,
                motion.start_mm[1] as f32,
                motion.start_mm[2] as f32,
            );
            let end_point = Point3::new(
                motion.end_mm[0] as f32,
                motion.end_mm[1] as f32,
                motion.end_mm[2] as f32,
            );
            let kind = display_kind(motion.kind);
            let tool = motion.tool;
            toolpath.append_line(start_point, end_point, kind, 0, display_tool(tool));
            let end = toolpath.len();
            per_line
                .entry(motion.line)
                .and_modify(|range| range.1 = end)
                .or_insert((start, end));
            stats.path_length_mm += motion.path_length_mm();
            match kind {
                SegmentKind::Rapid => stats.rapid_segments += 1,
                SegmentKind::Feed => stats.feed_segments += 1,
                SegmentKind::Plunge => stats.plunge_segments += 1,
                SegmentKind::Arc => stats.arc_segments += 1,
                SegmentKind::Probe => stats.probe_segments += 1,
            }
            stats.observe_tool(tool);
            motions.push(motion);
        }

        let mut lines = Vec::with_capacity(self.lines.len());
        for (line, text) in self.lines {
            let (start, end) = per_line.get(&line).copied().unwrap_or((0, 0));
            lines.push(ProgramLine {
                line,
                number: None,
                source: text,
                words: Vec::new(),
                comments: Vec::new(),
                block_delete: false,
                checksum: None,
                segments: start..end,
            });
        }
        stats.lines = lines.len();
        stats.segments = toolpath.len();
        stats.blocks = lines.iter().filter(|line| !line.source.is_empty()).count();
        let modal_table_len = motions
            .iter()
            .map(|motion| motion.modal.0 as usize + 1)
            .max()
            .unwrap_or(1);
        let bounds = toolpath.bounds();
        Program {
            lines,
            toolpath,
            bounds,
            stats,
            state: ParserState::default(),
            parameters: crate::expression::Parameters::new(),
            offsets: [Position::zero(); 9],
            warnings: Vec::new(),
            motions,
            annotations: self.annotations,
            comments: Vec::new(),
            tool_hints: Vec::new(),
            modal_table: vec![crate::motion::ModalState::default(); modal_table_len],
            geometry_uncertain_after_line: self.geometry_uncertain_after_line,
        }
    }
}

fn display_tool(tool: u32) -> u16 {
    u16::try_from(tool).unwrap_or(u16::MAX)
}

fn display_kind(kind: MoveKind) -> SegmentKind {
    match kind {
        MoveKind::Rapid | MoveKind::Retract => SegmentKind::Rapid,
        MoveKind::Plunge => SegmentKind::Plunge,
        MoveKind::ArcCw | MoveKind::ArcCcw | MoveKind::HelixCw | MoveKind::HelixCcw => {
            SegmentKind::Arc
        }
        MoveKind::Probe => SegmentKind::Probe,
        MoveKind::Feed | MoveKind::Ramp | MoveKind::Drill | MoveKind::Peck | MoveKind::Bore => {
            SegmentKind::Feed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_builds_lines_and_motions() {
        let program = ProgramBuilder::new()
            .source(1, "G21 G90")
            .motion(2, MoveKind::Rapid, [0.0; 3], [10.0, 0.0, 5.0], None)
            .motion(
                3,
                MoveKind::Plunge,
                [10.0, 0.0, 5.0],
                [10.0, 0.0, -1.0],
                Some(100.0),
            )
            .build();
        assert_eq!(program.motions.len(), 2);
        assert_eq!(program.lines.len(), 3);
        assert_eq!(program.segment_count(), 2);
        assert_eq!(program.motions[1].feed_mm_min, Some(100.0));
        assert!(program.cutting_bounds().min().z < 0.0);
    }

    #[test]
    fn expanded_motions_passes_plain_records_through() {
        let program = ProgramBuilder::new()
            .motion(1, MoveKind::Feed, [0.0; 3], [1.0, 0.0, 0.0], Some(200.0))
            .build();
        assert_eq!(program.expanded_motions().count(), 1);
    }
}
