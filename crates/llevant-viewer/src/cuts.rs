//! New cuts of the 3D view, made from what is being looked at: a plane
//! through a point cutting away the side facing the eye, a quarter wedge
//! about z turned towards the eye, a section box around the selection.

use llevant::Vec3;

use crate::document::Document;
use crate::selection::selection_box;
use crate::state::CutState;

/// The plane through `p` cutting away the side the eye is on.
pub fn plane_facing(doc: &Document, p: Vec3) -> CutState {
    let f = doc.orbit.forward();
    CutState::Plane {
        point: p,
        normal: f.map(|x| -x),
    }
}

/// A quarter wedge about z through `axis_point` (x, y; z ignored), on the
/// side of the eye: what it shows is the inside, cut open towards the
/// viewer.
pub fn wedge_facing(doc: &Document, axis_point: Vec3) -> CutState {
    let eye = doc.orbit.eye();
    let towards = (eye[1] - axis_point[1])
        .atan2(eye[0] - axis_point[0])
        .to_degrees();
    CutState::Wedge {
        point: axis_point,
        axis: [0.0, 0.0, 1.0],
        zero: [1.0, 0.0, 0.0],
        from: towards - 45.0,
        to: towards + 45.0,
    }
}

/// A box keeping only the selection (with a margin), or the middle half
/// of the model.
pub fn section_box(doc: &Document) -> CutState {
    let r = &doc.region;
    let b = selection_box(&doc.model, &doc.index, &doc.selection.cells, r).unwrap_or_else(|| {
        let c = r.center();
        llevant::Aabb {
            lo: [0, 1, 2].map(|k| c[k] - 0.25 * (r.hi[k] - r.lo[k])),
            hi: [0, 1, 2].map(|k| c[k] + 0.25 * (r.hi[k] - r.lo[k])),
        }
    });
    let pad = 0.02 * (0..3).map(|k| b.hi[k] - b.lo[k]).fold(0.0, f64::max);
    CutState::Box {
        lo: b.lo.map(|x| x - pad),
        hi: b.hi.map(|x| x + pad),
        keep: true,
    }
}

/// The axis of the radial planes about x, y and z, and where their angles
/// start (as the slice has them).
pub const RADIAL: [(Vec3, Vec3); 3] = [
    ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
    ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
];

/// The unit normal of the plane through axis `k` at `phi` degrees.
pub fn radial_normal(k: usize, phi: f64) -> Vec3 {
    use llevant::vec3::{add, cross, scale};
    let (a, zero) = RADIAL[k];
    let e90 = cross(a, zero);
    let (s, c) = phi.to_radians().sin_cos();
    cross(add(scale(zero, c), scale(e90, s)), a)
}

/// The angle (degrees) of the plane with unit normal `n` about axis `k`,
/// if it goes through that axis.
pub fn radial_angle(k: usize, n: Vec3) -> Option<f64> {
    use llevant::vec3::{cross, dot};
    if n[k].abs() > 1e-6 {
        return None;
    }
    let (a, zero) = RADIAL[k];
    let u = cross(a, n);
    Some(dot(u, cross(a, zero)).atan2(dot(u, zero)).to_degrees())
}

/// The axis (0, 1, 2) a unit vector lies along, either way, if it does.
pub fn along_axis(n: Vec3) -> Option<usize> {
    (0..3).find(|&k| n[k].abs() > 1.0 - 1e-9)
}
