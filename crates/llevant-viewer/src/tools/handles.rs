//! The geometry of handles dragged over either view, shared by the tools:
//! dragging along a direction, turning by a ring about an axis, and drawing
//! model segments.

use llevant::vec3::{add, cross, dot, norm, normalize, scale, sub};
use llevant::Vec3;

use crate::viewport::ViewPort;

/// How far (model units along `dir`) a drag from `from` to `pos` moves a
/// handle at `at`: the drag projected on the image of `dir` there.
pub fn along(
    view: &ViewPort,
    at: Vec3,
    dir: Vec3,
    from: egui::Pos2,
    pos: egui::Pos2,
) -> Option<f64> {
    let a = view.to_screen(at)?.pos;
    let b = view.to_screen(add(at, dir))?.pos;
    let s = b - a;
    let len2 = s.length_sq();
    (len2 > 1e-6).then(|| ((pos - from).dot(s) / len2) as f64)
}

/// Model length of a screen point at `at` along `dir` (unit), for steps
/// that look the same at any zoom.
pub fn length_per_point(view: &ViewPort, at: Vec3, dir: Vec3) -> Option<f64> {
    let a = view.to_screen(at)?.pos;
    let b = view.to_screen(add(at, dir))?.pos;
    let l = (b - a).length() as f64;
    (l > 1e-9).then(|| 1.0 / l)
}

/// `v` turned by `angle` (radians, right-handed) about the unit vector `axis`.
pub fn turn(v: Vec3, axis: Vec3, angle: f64) -> Vec3 {
    let (s, c) = angle.sin_cos();
    add(
        add(scale(v, c), scale(cross(axis, v), s)),
        scale(axis, dot(axis, v) * (1.0 - c)),
    )
}

/// The points of the ring about axis `k` through `c`, of radius `r`.
pub fn ring_points(c: Vec3, k: usize, r: f64) -> Vec<Vec3> {
    const N: usize = 48;
    let (u, v) = ((k + 1) % 3, (k + 2) % 3);
    (0..=N)
        .map(|i| {
            let (s, co) = (std::f64::consts::TAU * i as f64 / N as f64).sin_cos();
            let mut p = c;
            p[u] += r * co;
            p[v] += r * s;
            p
        })
        .collect()
}

/// The point of the ring about axis `k` through `c` (radius `r`) that shows
/// nearest `pos`, and how far (screen points).
pub fn nearest_on_ring(
    view: &ViewPort,
    c: Vec3,
    k: usize,
    r: f64,
    pos: egui::Pos2,
) -> Option<(Vec3, f32)> {
    ring_points(c, k, r)
        .into_iter()
        .filter_map(|p| Some((p, (view.to_screen(p)?.pos - pos).length())))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// How far `pos` is from the ring about axis `k` through `c` on screen.
pub fn ring_distance(view: &ViewPort, c: Vec3, k: usize, r: f64, pos: egui::Pos2) -> Option<f32> {
    let pts: Vec<egui::Pos2> = ring_points(c, k, r)
        .into_iter()
        .filter_map(|p| view.to_screen(p).map(|s| s.pos))
        .collect();
    pts.windows(2)
        .map(|w| {
            // Distance to the segment.
            let (a, b) = (w[0], w[1]);
            let ab = b - a;
            let t = if ab.length_sq() > 0.0 {
                ((pos - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (a + ab * t - pos).length()
        })
        .min_by(|a, b| a.total_cmp(b))
}

/// The angle (radians, about axis `k`) a ring through `c` of radius `rho`
/// is turned by dragging it from where it was pressed (`from`, at `pressed`)
/// to `pos`: the length dragged along the ring there, over its radius.
pub fn ring_angle(
    view: &ViewPort,
    c: Vec3,
    k: usize,
    rho: f64,
    pressed: Vec3,
    from: egui::Pos2,
    pos: egui::Pos2,
) -> Option<f64> {
    let mut axis = [0.0; 3];
    axis[k] = 1.0;
    let tangent = normalize(cross(axis, sub(pressed, c)));
    // How the ring runs on screen where it was pressed.
    let eps = 0.1 * rho;
    let run = view.to_screen(add(pressed, scale(tangent, eps)))?.pos - view.to_screen(pressed)?.pos;
    let per_unit = run.length() as f64 / eps;
    if per_unit < 1e-6 || !(norm(sub(pressed, c)) > 0.0) {
        return None;
    }
    let dir = run / run.length();
    Some((pos - from).dot(dir) as f64 / (per_unit * rho))
}

/// The segment from `a` to `b` (model points), where both show.
pub fn seg(view: &ViewPort, painter: &egui::Painter, a: Vec3, b: Vec3, stroke: egui::Stroke) {
    if let (Some(p), Some(q)) = (view.to_screen(a), view.to_screen(b)) {
        painter.line_segment([p.pos, q.pos], stroke);
    }
}
