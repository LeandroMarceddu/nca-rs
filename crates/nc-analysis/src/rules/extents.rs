//! Exact per-axis motion extents shared by the travel rules.
//!
//! A travel check must inspect the whole swept envelope, not only the motion
//! endpoints. A full-circle arc whose start and end coincide still spans the
//! whole circle, so an endpoint-only check can miss a violation that the
//! machine will actually execute.

use nc_gcode::MotionRecord;
use nc_stock::arc_bounds;

/// Returns the exact axis-aligned extents of one motion in machine millimetres.
///
/// The extents always include `start_mm` and `end_mm`. When the motion carries
/// an arc, [`arc_bounds`] contributes the swept cardinal extrema, including a
/// full circle whose endpoints are identical.
///
/// Returns `None` when an endpoint coordinate is non-finite: the motion cannot
/// be ordered against a travel or stock box, so a rule must not report it. A
/// malformed arc extremum that is non-finite is ignored per axis and leaves the
/// endpoint extent in place.
#[must_use]
pub(crate) fn motion_extents(motion: &MotionRecord) -> Option<([f64; 3], [f64; 3])> {
    let mut min = [0.0; 3];
    let mut max = [0.0; 3];
    for axis in 0..3 {
        let start = motion.start_mm[axis];
        let end = motion.end_mm[axis];
        if !start.is_finite() || !end.is_finite() {
            return None;
        }
        min[axis] = start.min(end);
        max[axis] = start.max(end);
    }

    if let Some(arc) = &motion.arc {
        let (arc_min, arc_max) = arc_bounds(arc, motion.plane, motion.start_mm, motion.end_mm);
        for axis in 0..3 {
            if arc_min[axis].is_finite() {
                min[axis] = min[axis].min(arc_min[axis]);
            }
            if arc_max[axis].is_finite() {
                max[axis] = max[axis].max(arc_max[axis]);
            }
        }
    }

    Some((min, max))
}
