//! The reference the fast paths are tested against: classification and
//! tracing by brute force (every cell of each universe a candidate, no
//! boxes, BVH or octrees), and when two tracings agree.
//!
//! The fast path gives exactly the brute-force labels for points
//! ([`classify`]); for lines it agrees with brute force as
//! [`traces_agree`] says (`docs/contracts.md`). Slow: for tests, tools
//! and checking a model.

use crate::classify::{Classification, Depth, Label, Status};
use crate::error::Result;
use crate::model::Model;
use crate::trace::{check_lines, Line, Segment, Tracing};
use crate::vec3::Vec3;

/// Classify `points` as [`Model::classify`], by brute force.
pub fn classify(model: &Model, points: &[Vec3], depth: Depth) -> Classification {
    model.classify_impl(points, true, depth.level())
}

/// Trace `lines` as [`Model::trace`], by brute force: every cell of each
/// universe is a candidate over the whole line.
pub fn trace(model: &Model, lines: &[Line], depth: Depth) -> Result<Tracing> {
    check_lines(lines)?;
    Ok(model.trace_unchecked(lines, true, depth.level()))
}

/// Whether two tracings of `line` (fast and brute force, say) agree:
/// segment by segment, the same labels and ends within the tolerance
/// (where several surfaces lie within it, which of them is named may
/// differ), once on-surface slivers up to 1e-6 cm are passed over in both.
/// Segments are decided at midpoints between crossings, and brute force
/// intersects surfaces that cannot bound a cell where they cross the line;
/// such a crossing within the tolerance of a cell's internal surface shows
/// a sliver of the band around it that the fast path need not show. Gaps
/// and overlaps (wider than the tolerance) are the same either way.
pub fn traces_agree(model: &Model, line: &Line, a: &[Segment], b: &[Segment]) -> bool {
    model.traces_agree(line, a, b)
}

/// [`traces_agree`] for one pair of segments (no slivers passed over).
pub fn segments_agree(model: &Model, line: &Line, a: &Segment, b: &Segment) -> bool {
    model.segments_agree(line, a, b)
}

/// Whether `point`, the label of a point on the segment labelled `segment`,
/// is the on-surface band next to the segment's cell that a line at grazing
/// incidence passes through (`tol / sin θ` long; `docs/internals/engine.md`,
/// "Deliberate semantics"): on-surface at some level of the segment's path,
/// that path down to there, and the segment's cell at that level among the
/// cells involved. The band can be at any level the segment goes through,
/// not only its last: a plane of a level-0 cell the line runs almost inside.
pub fn on_surface_next_to(point: &Label, segment: &Label) -> bool {
    let k = point.path.len();
    point.status == Status::OnSurface
        && segment.path.len() > k
        && segment.path[..k] == point.path[..]
        && point.involved.contains(&segment.path[k].cell)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::Step;

    fn step(cell: u32) -> Step {
        Step {
            cell,
            lattice: None,
        }
    }

    #[test]
    fn on_surface_next_to_any_level_of_the_path() {
        let segment = Label {
            status: Status::Inside,
            path: vec![step(7), step(40)],
            involved: vec![],
        };
        let at = |path: Vec<Step>, cells: Vec<u32>| Label {
            status: Status::OnSurface,
            path,
            involved: cells,
        };
        // Between the segment's cells at level 1, or at level 0.
        assert!(on_surface_next_to(
            &at(vec![step(7)], vec![40, 41]),
            &segment
        ));
        assert!(on_surface_next_to(&at(vec![], vec![7, 9]), &segment));
        // Not next to the segment's cell, or on another path.
        assert!(!on_surface_next_to(&at(vec![], vec![8, 9]), &segment));
        assert!(!on_surface_next_to(&at(vec![step(8)], vec![40]), &segment));
        // Only on-surface points.
        let gap = Label {
            status: Status::Gap,
            ..at(vec![], vec![7])
        };
        assert!(!on_surface_next_to(&gap, &segment));
    }
}
