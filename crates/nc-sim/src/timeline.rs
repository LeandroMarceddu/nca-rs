//! Precomputed motion timeline.

use std::collections::BTreeMap;

use nc_core::math;
use nc_gcode::{Annotation, ArcRecord, MotionRecord, Plane, Program};
use nc_stock::arc_point;
use nc_viewer::{AnnotationKind, MoveKind};

use crate::config::SimConfig;

/// One motion on the timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSpan {
    /// Start time in simulated seconds.
    pub start_time_s: f64,
    /// End time in simulated seconds.
    pub end_time_s: f64,
    /// Distance at the start of the span in millimetres.
    pub start_distance_mm: f64,
    /// Distance at the end of the span in millimetres.
    pub end_distance_mm: f64,
    /// Source line that produced the motion.
    pub line: usize,
    /// Semantic motion kind.
    pub kind: MoveKind,
    /// Active tool number.
    pub tool: u32,
    /// Start pose in millimetres.
    pub start_pose_mm: [f64; 3],
    /// End pose in millimetres.
    pub end_pose_mm: [f64; 3],
    /// Programmed feed in mm/min, when known.
    pub feed_mm_min: Option<f64>,
    /// `true` when the spindle is on for this span.
    pub spindle_on: bool,
}

impl MotionSpan {
    /// Duration in seconds.
    #[must_use]
    pub fn duration_s(&self) -> f64 {
        (self.end_time_s - self.start_time_s).max(0.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
struct SpanArc {
    arc: ArcRecord,
    plane: Plane,
}

/// The ordered motion timeline with prefix sums and a line index.
///
/// Dwells and tool-change pauses are gaps between spans: the span times already
/// include them, the path distance does not. The timeline stores span metadata
/// and exact arc geometry, not copies of the program's motion records.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Timeline {
    spans: Vec<MotionSpan>,
    arcs: BTreeMap<usize, SpanArc>,
    line_to_time: BTreeMap<usize, f64>,
    motion_line_to_time: BTreeMap<usize, f64>,
    total_distance_mm: f64,
    total_time_s: f64,
    rapid_time_s: f64,
    cutting_time_s: f64,
    air_time_s: f64,
    dwell_time_s: f64,
    tool_change_time_s: f64,
}

impl Timeline {
    /// Builds a timeline from `Program::expanded_motions()`.
    ///
    /// Feed moves use `length / feed * 60`; arcs and helixes use their exact
    /// arc length. Rapids use `rapid_rate_mm_min`. Dwell and tool-change
    /// annotations insert pauses between spans. A missing or invalid feed falls
    /// back to the rapid rate so the timeline always has a finite duration.
    #[must_use]
    pub fn from_program(program: &Program, config: &SimConfig) -> Self {
        let mut timeline = Self::default();
        let mut time = 0.0_f64;
        let mut distance = 0.0_f64;
        let mut spindle_on = false;
        let mut last_tool: Option<u32> = None;
        let mut pause_for_tool: Option<u32> = None;
        let mut annotation_index = 0_usize;

        for motion in program.expanded_motions() {
            while annotation_index < program.annotations.len() {
                let annotation = &program.annotations[annotation_index];
                if annotation.next_motion_index > motion.index {
                    break;
                }
                timeline.record_annotation_line(annotation, time);
                let (dwell, pause) = apply_annotation(
                    annotation,
                    config,
                    &mut spindle_on,
                    &mut last_tool,
                    &mut pause_for_tool,
                );
                time += dwell + pause;
                timeline.dwell_time_s += dwell;
                timeline.tool_change_time_s += pause;
                annotation_index += 1;
            }

            if let Some(previous) = last_tool {
                if previous != motion.tool {
                    if pause_for_tool != Some(motion.tool) {
                        let pause = valid_pause(config.tool_change_pause_s);
                        time += pause;
                        timeline.tool_change_time_s += pause;
                    }
                    last_tool = Some(motion.tool);
                }
            } else {
                last_tool = Some(motion.tool);
            }
            pause_for_tool = None;

            if let (Some(rpm), Some(_)) = (motion.spindle_rpm, motion.spindle_cw) {
                spindle_on = rpm > 0.0;
            }

            let length = motion.path_length_mm();
            let duration = duration_for(&motion, config);
            timeline.line_to_time.entry(motion.line).or_insert(time);
            timeline
                .motion_line_to_time
                .entry(motion.line)
                .or_insert(time);
            let span_index = timeline.spans.len();
            if let Some(arc) = &motion.arc {
                timeline.arcs.insert(
                    span_index,
                    SpanArc {
                        arc: arc.as_ref().clone(),
                        plane: motion.plane,
                    },
                );
            }
            timeline.spans.push(MotionSpan {
                start_time_s: time,
                end_time_s: time + duration,
                start_distance_mm: distance,
                end_distance_mm: distance + length,
                line: motion.line,
                kind: motion.kind,
                tool: motion.tool,
                start_pose_mm: motion.start_mm,
                end_pose_mm: motion.end_mm,
                feed_mm_min: motion.feed_mm_min,
                spindle_on,
            });
            if motion.kind.is_cutting() {
                timeline.cutting_time_s += duration;
            } else if motion.kind.is_rapid() {
                timeline.rapid_time_s += duration;
            } else {
                timeline.air_time_s += duration;
            }
            time += duration;
            distance += length;
        }

        while annotation_index < program.annotations.len() {
            let annotation = &program.annotations[annotation_index];
            timeline.record_annotation_line(annotation, time);
            let (dwell, pause) = apply_annotation(
                annotation,
                config,
                &mut spindle_on,
                &mut last_tool,
                &mut pause_for_tool,
            );
            time += dwell + pause;
            timeline.dwell_time_s += dwell;
            timeline.tool_change_time_s += pause;
            annotation_index += 1;
        }

        timeline.total_time_s = time;
        timeline.total_distance_mm = distance;
        timeline
    }

    fn record_annotation_line(&mut self, annotation: &Annotation, time: f64) {
        self.line_to_time.entry(annotation.line).or_insert(time);
    }

    /// Motion spans in order.
    #[must_use]
    pub fn spans(&self) -> &[MotionSpan] {
        &self.spans
    }

    /// Total simulated duration in seconds, including pauses.
    #[must_use]
    pub fn total_time_s(&self) -> f64 {
        self.total_time_s
    }

    /// End time of the last motion, excluding any trailing pause.
    #[must_use]
    pub fn last_motion_end_s(&self) -> f64 {
        self.spans.last().map_or(0.0, |span| span.end_time_s)
    }

    /// Total path length in millimetres.
    #[must_use]
    pub fn total_distance_mm(&self) -> f64 {
        self.total_distance_mm
    }

    /// Time spent in rapid and retract spans.
    #[must_use]
    pub fn rapid_time_s(&self) -> f64 {
        self.rapid_time_s
    }

    /// Time spent in cutting spans.
    #[must_use]
    pub fn cutting_time_s(&self) -> f64 {
        self.cutting_time_s
    }

    /// Time spent in non-cutting, non-rapid spans such as probes.
    #[must_use]
    pub fn air_time_s(&self) -> f64 {
        self.air_time_s
    }

    /// Time spent in dwell annotations.
    #[must_use]
    pub fn dwell_time_s(&self) -> f64 {
        self.dwell_time_s
    }

    /// Time spent in tool-change pauses.
    #[must_use]
    pub fn tool_change_time_s(&self) -> f64 {
        self.tool_change_time_s
    }

    /// Returns `true` when the timeline has no motion.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Index of the span active at `time_s`, or the last span after the end.
    #[must_use]
    pub fn span_at_time(&self, time_s: f64) -> Option<usize> {
        if self.spans.is_empty() || !time_s.is_finite() {
            return None;
        }
        let index = self.spans.partition_point(|span| span.end_time_s <= time_s);
        Some(index.min(self.spans.len() - 1))
    }

    /// Span indexes overlapping `[start_time_s, end_time_s)`.
    #[must_use]
    pub fn overlapping_spans(&self, start_time_s: f64, end_time_s: f64) -> std::ops::Range<usize> {
        if self.spans.is_empty()
            || !start_time_s.is_finite()
            || !end_time_s.is_finite()
            || end_time_s <= start_time_s
        {
            return 0..0;
        }
        let first = self
            .spans
            .partition_point(|span| span.end_time_s <= start_time_s);
        if first >= self.spans.len() {
            return 0..0;
        }
        let count = self.spans[first..].partition_point(|span| span.start_time_s < end_time_s);
        first..first + count
    }

    /// Exact pose at `time_s`, using the arc curve for arc spans.
    #[must_use]
    pub fn pose_at_time(&self, time_s: f64) -> Option<[f64; 3]> {
        let index = self.span_at_time(time_s)?;
        let span = self.spans.get(index)?;
        if time_s <= span.start_time_s {
            Some(span.start_pose_mm)
        } else if time_s >= span.end_time_s {
            Some(span.end_pose_mm)
        } else {
            self.pose_in_span(index, time_s)
        }
    }

    /// Exact pose inside `span_index` at `time_s`.
    #[must_use]
    pub fn pose_in_span(&self, span_index: usize, time_s: f64) -> Option<[f64; 3]> {
        let span = self.spans.get(span_index)?;
        let duration = span.duration_s();
        let fraction = if duration > 0.0 {
            math::clamp((time_s - span.start_time_s) / duration, 0.0, 1.0)
        } else {
            1.0
        };
        if let Some(geometry) = self.arcs.get(&span_index) {
            Some(arc_point(
                &geometry.arc,
                geometry.plane,
                span.start_pose_mm,
                span.end_pose_mm,
                fraction,
            ))
        } else {
            Some([
                span.start_pose_mm[0] + (span.end_pose_mm[0] - span.start_pose_mm[0]) * fraction,
                span.start_pose_mm[1] + (span.end_pose_mm[1] - span.start_pose_mm[1]) * fraction,
                span.start_pose_mm[2] + (span.end_pose_mm[2] - span.start_pose_mm[2]) * fraction,
            ])
        }
    }

    /// Exact arc geometry for a span, when the span is an arc or helix.
    #[must_use]
    pub fn arc_for_span(&self, span_index: usize) -> Option<&ArcRecord> {
        self.arcs.get(&span_index).map(|geometry| &geometry.arc)
    }

    /// Arc plane for a span, when the span is an arc or helix.
    #[must_use]
    pub fn plane_for_span(&self, span_index: usize) -> Option<Plane> {
        self.arcs.get(&span_index).map(|geometry| geometry.plane)
    }

    /// First time at which `line` is reached.
    #[must_use]
    pub fn time_for_line(&self, line: usize) -> Option<f64> {
        self.line_to_time.get(&line).copied()
    }

    /// First time at or after `line` that has an indexed event.
    #[must_use]
    pub fn time_for_line_at_or_after(&self, line: usize) -> Option<f64> {
        self.line_to_time
            .range(line..)
            .next()
            .map(|(_, time)| *time)
    }

    /// First motion line strictly after `line` and its start time.
    #[must_use]
    pub fn time_for_motion_line_after(&self, line: usize) -> Option<f64> {
        self.motion_line_to_time
            .range(line.saturating_add(1)..)
            .next()
            .map(|(_, time)| *time)
    }

    /// The line-to-time index.
    #[must_use]
    pub fn line_to_time(&self) -> &BTreeMap<usize, f64> {
        &self.line_to_time
    }
}

fn duration_for(motion: &MotionRecord, config: &SimConfig) -> f64 {
    let rate = if motion.kind.is_rapid() {
        config.rapid_rate_mm_min
    } else {
        match motion.feed_mm_min {
            Some(feed) if feed.is_finite() && feed > 0.0 => feed,
            _ => config.rapid_rate_mm_min,
        }
    };
    let length = motion.path_length_mm();
    if rate > 0.0 && length > 0.0 && length.is_finite() {
        length / rate * 60.0
    } else {
        0.0
    }
}

fn valid_pause(pause_s: f64) -> f64 {
    if pause_s.is_finite() && pause_s > 0.0 {
        pause_s
    } else {
        0.0
    }
}

fn apply_annotation(
    annotation: &Annotation,
    config: &SimConfig,
    spindle_on: &mut bool,
    last_tool: &mut Option<u32>,
    pause_for_tool: &mut Option<u32>,
) -> (f64, f64) {
    match &annotation.kind {
        AnnotationKind::Dwell { seconds } => {
            let dwell = if seconds.is_finite() && *seconds > 0.0 {
                *seconds
            } else {
                0.0
            };
            (dwell, 0.0)
        }
        AnnotationKind::ToolChange { tool } => {
            *last_tool = Some(*tool);
            *pause_for_tool = Some(*tool);
            (0.0, valid_pause(config.tool_change_pause_s))
        }
        AnnotationKind::Spindle { on, .. } => {
            *spindle_on = *on;
            (0.0, 0.0)
        }
        AnnotationKind::Coolant { .. } => (0.0, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nc_core::Dialect;
    use nc_gcode::{ArcFormat, ParserConfig, ProgramBuilder};
    use nc_viewer::MoveKind;

    fn config() -> SimConfig {
        SimConfig {
            rapid_rate_mm_min: 5000.0,
            tool_change_pause_s: 2.0,
            ..SimConfig::default()
        }
    }

    fn arc(start_angle_rad: f64, sweep_rad: f64, axial_delta_mm: f64) -> ArcRecord {
        ArcRecord {
            center_mm: [0.0, 0.0, 0.0],
            radius_mm: 10.0,
            end_radius_mm: 10.0,
            start_angle_rad,
            sweep_rad,
            axial_delta_mm,
            format: ArcFormat::Center,
        }
    }

    #[test]
    fn arc_and_helix_durations_are_exact() {
        let mut arc_motion =
            MotionRecord::linear(2, 0, MoveKind::ArcCw, [10.0, 0.0, 0.0], [-10.0, 0.0, 0.0]);
        arc_motion.feed_mm_min = Some(100.0);
        arc_motion.arc = Some(Box::new(arc(0.0, math::PI, 0.0)));

        let mut helix =
            MotionRecord::linear(3, 1, MoveKind::HelixCcw, [10.0, 0.0, 0.0], [10.0, 0.0, 5.0]);
        helix.feed_mm_min = Some(200.0);
        helix.arc = Some(Box::new(arc(0.0, math::TAU, 5.0)));

        let program = ProgramBuilder::new()
            .record(arc_motion)
            .record(helix)
            .build();
        let timeline = Timeline::from_program(&program, &config());
        assert_eq!(timeline.spans().len(), 2);
        let arc_duration = math::PI * 10.0 / 100.0 * 60.0;
        assert!((timeline.spans()[0].duration_s() - arc_duration).abs() < 1e-12);
        let helix_length = math::hypot(math::TAU * 10.0, 5.0);
        let helix_duration = helix_length / 200.0 * 60.0;
        assert!((timeline.spans()[1].duration_s() - helix_duration).abs() < 1e-12);
        assert!((timeline.total_time_s() - (arc_duration + helix_duration)).abs() < 1e-12);
    }

    #[test]
    fn rapid_duration_uses_the_configured_rate() {
        let program = ProgramBuilder::new()
            .motion(1, MoveKind::Rapid, [0.0, 0.0, 0.0], [10.0, 0.0, 0.0], None)
            .build();
        let timeline = Timeline::from_program(&program, &config());
        assert!((timeline.spans()[0].duration_s() - 10.0 / 5000.0 * 60.0).abs() < 1e-12);
        assert!((timeline.rapid_time_s() - 0.12).abs() < 1e-12);
    }

    #[test]
    fn dwell_and_tool_change_insert_pauses_between_spans() {
        let program = ProgramBuilder::new()
            .motion(
                1,
                MoveKind::Feed,
                [0.0, 0.0, -1.0],
                [10.0, 0.0, -1.0],
                Some(600.0),
            )
            .annotation(Annotation {
                line: 2,
                next_motion_index: 1,
                pos_mm: [10.0, 0.0, -1.0],
                kind: AnnotationKind::Dwell { seconds: 1.5 },
            })
            .annotation(Annotation {
                line: 3,
                next_motion_index: 1,
                pos_mm: [10.0, 0.0, -1.0],
                kind: AnnotationKind::ToolChange { tool: 2 },
            })
            .record({
                let mut motion = MotionRecord::linear(
                    4,
                    1,
                    MoveKind::Feed,
                    [10.0, 0.0, -1.0],
                    [20.0, 0.0, -1.0],
                );
                motion.feed_mm_min = Some(600.0);
                motion.tool = 2;
                motion
            })
            .build();
        let timeline = Timeline::from_program(&program, &config());
        assert_eq!(timeline.spans().len(), 2);
        let first_duration = timeline.spans()[0].duration_s();
        assert!((first_duration - 1.0).abs() < 1e-12);
        assert!((timeline.spans()[1].start_time_s - (first_duration + 1.5 + 2.0)).abs() < 1e-12);
        assert!((timeline.dwell_time_s() - 1.5).abs() < 1e-12);
        assert!((timeline.tool_change_time_s() - 2.0).abs() < 1e-12);
        assert_eq!(timeline.time_for_line(2), Some(first_duration));
        assert_eq!(timeline.time_for_line(4), Some(first_duration + 1.5 + 2.0));
    }

    #[test]
    fn tool_number_change_without_an_annotation_still_pauses() {
        let program = ProgramBuilder::new()
            .motion(
                1,
                MoveKind::Feed,
                [0.0, 0.0, -1.0],
                [10.0, 0.0, -1.0],
                Some(600.0),
            )
            .record({
                let mut motion = MotionRecord::linear(
                    2,
                    1,
                    MoveKind::Feed,
                    [10.0, 0.0, -1.0],
                    [20.0, 0.0, -1.0],
                );
                motion.feed_mm_min = Some(600.0);
                motion.tool = 2;
                motion
            })
            .build();
        let timeline = Timeline::from_program(&program, &config());
        assert_eq!(timeline.tool_change_time_s(), 2.0);
    }

    #[test]
    fn dwell_seconds_come_from_the_annotation_conversion() {
        // The parser converts G4 P per dialect before building the annotation.
        // A Generic `P2` and a Fanuc `P2000` both reach the timeline as 2 s.
        for (p_value, dialect_is_milliseconds) in [(2.0, false), (2000.0, true)] {
            let seconds = if dialect_is_milliseconds {
                p_value / 1000.0
            } else {
                p_value
            };
            let program = ProgramBuilder::new()
                .motion(
                    1,
                    MoveKind::Feed,
                    [0.0, 0.0, -1.0],
                    [10.0, 0.0, -1.0],
                    Some(600.0),
                )
                .annotation(Annotation {
                    line: 2,
                    next_motion_index: 1,
                    pos_mm: [10.0, 0.0, -1.0],
                    kind: AnnotationKind::Dwell { seconds },
                })
                .build();
            let timeline = Timeline::from_program(&program, &config());
            let motion_duration = 10.0 / 600.0 * 60.0;
            assert!((timeline.total_time_s() - (motion_duration + 2.0)).abs() < 1e-12);
        }
    }

    #[test]
    fn dwell_units_follow_the_dialect_through_the_parser() {
        let motion_duration = 10.0 / 600.0 * 60.0;
        for (dialect, p) in [(Dialect::Generic, "2"), (Dialect::Fanuc, "2000")] {
            let source = format!("G21 G90\nG4 P{p}\nG1 X10 F600\n");
            let parser_config = ParserConfig {
                dialect,
                ..ParserConfig::default()
            };
            let program = nc_gcode::parse_program_with(&source, &parser_config).expect("parse");
            let timeline = Timeline::from_program(&program, &config());
            assert!(
                (timeline.total_time_s() - (motion_duration + 2.0)).abs() < 1e-9,
                "{dialect} P{p} total {}",
                timeline.total_time_s()
            );
            assert!((timeline.dwell_time_s() - 2.0).abs() < 1e-9);
        }
    }

    #[test]
    fn arc_pose_follows_the_curve_not_the_chord() {
        let mut motion =
            MotionRecord::linear(1, 0, MoveKind::ArcCcw, [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]);
        motion.feed_mm_min = Some(60.0);
        motion.arc = Some(Box::new(arc(0.0, math::FRAC_PI_2, 0.0)));
        let program = ProgramBuilder::new().record(motion).build();
        let timeline = Timeline::from_program(&program, &config());
        let span = timeline.spans()[0];
        let middle = timeline
            .pose_at_time(span.start_time_s + span.duration_s() * 0.5)
            .expect("pose");
        let radius_at_45 = 10.0 * math::sqrt(0.5);
        assert!((middle[0] - radius_at_45).abs() < 1e-9);
        assert!((middle[1] - radius_at_45).abs() < 1e-9);
    }

    #[test]
    fn zero_length_spans_keep_their_line_index() {
        let program = ProgramBuilder::new()
            .motion(5, MoveKind::Rapid, [0.0, 0.0, 0.0], [0.0, 0.0, 0.0], None)
            .build();
        let timeline = Timeline::from_program(&program, &config());
        assert_eq!(timeline.spans().len(), 1);
        assert_eq!(timeline.spans()[0].duration_s(), 0.0);
        assert_eq!(timeline.time_for_line(5), Some(0.0));
    }
}
