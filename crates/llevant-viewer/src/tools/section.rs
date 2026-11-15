//! The cuts of the 3D view, and the slice plane, drawn and handled in place,
//! whatever the tool ([`super::Layer`]). A plane is a square: its centre
//! handle slides it along its normal, and, once it is selected, three rings
//! about the x, y and z axes (in the gizmo's colours) turn it about the one
//! dragged, in steps of 15° that include the axes, or freely with Shift or
//! Alt held. A wedge's two handles turn its sides about its axis; a box's six
//! face handles move its faces. The slice plane, when shown in 3D, has the
//! same handles, beside its middle: turning it puts the slice along an axis
//! or, between them, through one (a radial slice).
//!
//! Cuts turned off are not drawn; with "show cuts in 3D" off (the Section
//! menu) only the selected cut is. Cuts not selected are faint outlines with their handles; a click on an
//! outline or a handle selects the cut (the slice plane: a handle; the 3D
//! view's Section menu shows a cut in detail) and is not a click on the cells; a click anywhere else
//! deselects it and goes on to the tool. Each drag is one step of the
//! history. Cuts are added from the Section menu or the right-click menu
//! ("Cut here").

use llevant::vec3::{add, cross, dot, norm, normalize, scale, sub};
use llevant::Vec3;

use crate::command::{Command, Edit};
use crate::cuts;
use crate::document::Document;
use crate::state::{AxisName, CutState, PlaneState, Radial};
use crate::theme;
use crate::tools::{handles, Layer};
use crate::viewport::{Projection, ViewKind, ViewPort};

/// Pick distance of a handle (screen points).
const GRAB: f32 = 9.0;

/// The colours of the x, y and z rings (as the axes gizmo has them).
const AXIS_COLOURS: [egui::Color32; 3] = [
    egui::Color32::from_rgb(230, 80, 80),
    egui::Color32::from_rgb(110, 200, 90),
    egui::Color32::from_rgb(90, 150, 255),
];

/// The `cut` of the slice plane's handle, which is not one of the scene's cuts.
const SLICE: usize = usize::MAX;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Grip {
    /// A plane's centre: slides it along its normal.
    Plane,
    /// The slice plane's centre (of cut `SLICE`): slides the slice along its normal.
    Slice,
    /// One of the three rings (about x, y, z) that turn the slice.
    SliceRing(usize),
    /// One of the three rings (about x, y, z) that turn a plane.
    Ring(usize),
    /// A wedge's side: `false` its `from` angle, `true` its `to`.
    Side(bool),
    /// A box's face: axis and which side (`true`: the high one).
    Face(usize, bool),
}

#[derive(Clone, Copy, Debug)]
struct Handle {
    cut: usize,
    grip: Grip,
    /// Where it is drawn (for a ring, one of its points).
    at: Vec3,
    /// Where it is drawn from (the plane's point, the wedge's axis, a ring's
    /// centre).
    base: Vec3,
}

struct Drag {
    handle: Handle,
    start: CutState,
    /// Which plane the slice was on, for the slice's handles.
    slice: Option<(PlaneState, Vec3)>,
    from: egui::Pos2,
    /// For a ring: the point of it that was pressed.
    pressed: Vec3,
}

#[derive(Default)]
pub struct CutHandles {
    drag: Option<Drag>,
}

/// Length of a plane's arrow and radius of a wedge's handles and of the
/// rings, on screen at the point looked at (points).
const HANDLE_POINTS: f64 = 90.0;

/// Length of a plane's arrow and radius of a wedge's handles and of the
/// rings (cm): 90 points on screen at the point looked at, whatever
/// the zoom.
pub fn handle_size(doc: &Document) -> f64 {
    let s = doc.orbit.state;
    let half = s.distance * (0.5 * s.fov_deg).to_radians().tan();
    HANDLE_POINTS * 2.0 * half / doc.view3d_height.max(1.0) as f64
}

fn size(doc: &Document) -> f64 {
    handle_size(doc)
}

/// Where the slice plane's handles are: beside the middle of the plane as shown
/// in 3D (where the cuts usually are), within the plane and the view.
pub fn slice_handle_base(doc: &Document) -> Vec3 {
    slice_geometry(doc).0
}

/// Where the slice plane's handles are, the middle of the plane, and its unit
/// normal.
fn slice_geometry(doc: &Document) -> (Vec3, Vec3, Vec3) {
    let plane = doc.slice.plane();
    let c = plane.corners();
    let mid = [0, 1, 2].map(|k| 0.25 * (c[0][k] + c[1][k] + c[2][k] + c[3][k]));
    let f = doc.slice.frame();
    let off = (3.0 * size(doc)).min(0.3 * plane.width);
    (add(mid, scale(normalize(f.u), off)), mid, normalize(f.n))
}

/// The three rings about the axes through `centre`, of radius `r`, as handles.
fn rings(out: &mut Vec<Handle>, cut: usize, slice: bool, centre: Vec3, r: f64) {
    for k in 0..3 {
        let mut at = centre;
        at[(k + 1) % 3] += r;
        out.push(Handle {
            cut,
            grip: if slice {
                Grip::SliceRing(k)
            } else {
                Grip::Ring(k)
            },
            at,
            base: centre,
        });
    }
}

/// Every handle of the scene's cuts.
fn handles(doc: &Document) -> Vec<Handle> {
    let mut out = Vec::new();
    let r = size(doc);
    for (k, c) in doc.scene.cuts.iter().enumerate() {
        if !shown(doc, k) {
            continue;
        }
        match *c {
            CutState::Plane { point, .. } => {
                // The rings that turn the selected one.
                if doc.editing.cut_selected == Some(k) {
                    rings(&mut out, k, false, point, r);
                }
                out.push(Handle {
                    cut: k,
                    grip: Grip::Plane,
                    at: point,
                    base: point,
                });
            }
            CutState::Wedge { .. } => {
                let base = wedge_centre(doc, c);
                for side in [false, true] {
                    out.push(Handle {
                        cut: k,
                        grip: Grip::Side(side),
                        at: wedge_handle(c, base, r, side),
                        base,
                    });
                }
            }
            CutState::Box { lo, hi, .. } => {
                let c = [0, 1, 2].map(|d| 0.5 * (lo[d] + hi[d]));
                for d in 0..3 {
                    for high in [false, true] {
                        let mut at = c;
                        at[d] = if high { hi[d] } else { lo[d] };
                        out.push(Handle {
                            cut: k,
                            grip: Grip::Face(d, high),
                            at,
                            base: c,
                        });
                    }
                }
            }
        }
    }
    // Last: where a cut's handle is under one of the slice's, the cut's wins.
    if doc.slice_shown_in_3d() {
        let (edge, _, _) = slice_geometry(doc);
        if doc.editing.slice_selected {
            rings(&mut out, SLICE, true, edge, r);
        }
        out.push(Handle {
            cut: SLICE,
            grip: Grip::Slice,
            at: edge,
            base: edge,
        });
    }
    out
}

/// The wedge's axis at the height (along it) of what the view is centred
/// on.
fn wedge_centre(doc: &Document, c: &CutState) -> Vec3 {
    let CutState::Wedge { point, axis, .. } = *c else {
        unreachable!("a wedge")
    };
    let a = normalize(axis);
    let h = dot(sub(doc.orbit.state.target, point), a);
    add(point, scale(a, h))
}

/// The unit vectors normal to a wedge's axis at angles 0 and 90°.
fn wedge_basis(c: &CutState) -> (Vec3, Vec3, Vec3) {
    let CutState::Wedge { axis, zero, .. } = *c else {
        unreachable!("a wedge")
    };
    let a = normalize(axis);
    let e0 = normalize(sub(zero, scale(a, dot(zero, a))));
    (a, e0, cross(a, e0))
}

fn wedge_handle(c: &CutState, base: Vec3, r: f64, to_side: bool) -> Vec3 {
    let CutState::Wedge { from, to, .. } = *c else {
        unreachable!("a wedge")
    };
    let (_, e0, e90) = wedge_basis(c);
    let deg = if to_side { to } else { from };
    let (s, co) = deg.to_radians().sin_cos();
    add(base, scale(add(scale(e0, co), scale(e90, s)), r))
}

/// The cut after dragging `h` from `from` to `pos`, as it was at `start`.
fn dragged(view: &ViewPort, d: &Drag, pos: egui::Pos2, free: bool) -> Option<CutState> {
    let h = &d.handle;
    // Along a direction: the drag projected on the direction's image.
    let along = |dir: Vec3| handles::along(view, h.at, dir, d.from, pos);
    Some(match (d.start, h.grip) {
        (CutState::Plane { point, normal }, Grip::Plane | Grip::Slice) => {
            let n = normalize(normal);
            let t = along(n)?;
            CutState::Plane {
                point: add(point, scale(n, t)),
                normal,
            }
        }
        (CutState::Plane { point, normal }, Grip::Ring(k)) => {
            let n = ring_turned(view, d, k, normal, pos, free)?;
            CutState::Plane { point, normal: n }
        }
        (c @ CutState::Wedge { .. }, Grip::Side(to_side)) => {
            // Where the pointer's ray meets the plane normal to the axis
            // through the handles: the side's new angle.
            let Projection::Camera(cam) = &view.projection else {
                return None;
            };
            let (i, j) = view.pixel(pos)?;
            let (o, dir) = (cam.origin(i, j), cam.direction(i, j));
            let (a, e0, e90) = wedge_basis(&c);
            let ds = dot(dir, a);
            if ds.abs() < 1e-9 {
                return None;
            }
            let t = dot(sub(h.base, o), a) / ds;
            let q = sub(add(o, scale(dir, t)), h.base);
            let deg = dot(q, e90).atan2(dot(q, e0)).to_degrees();
            let CutState::Wedge {
                point,
                axis,
                zero,
                from,
                to,
            } = c
            else {
                unreachable!()
            };
            // Keep the angle near its old value (no jump across ±180°).
            let near = |old: f64| old + (deg - old + 180.0).rem_euclid(360.0) - 180.0;
            let (from, to) = if to_side {
                (from, near(to))
            } else {
                (near(from), to)
            };
            CutState::Wedge {
                point,
                axis,
                zero,
                from,
                to,
            }
        }
        (
            CutState::Box {
                mut lo,
                mut hi,
                keep,
            },
            Grip::Face(k, high),
        ) => {
            let mut e = [0.0; 3];
            e[k] = 1.0;
            let t = along(e)?;
            if high {
                hi[k] = (hi[k] + t).max(lo[k] + 1e-6);
            } else {
                lo[k] = (lo[k] + t).min(hi[k] - 1e-6);
            }
            CutState::Box { lo, hi, keep }
        }
        _ => return None,
    })
}

/// Granularity of a turn by a handle, in degrees: the axes and what is
/// between them (Shift or Alt turns freely).
const STEP: f64 = 15.0;

/// The unit vector nearest `d` whose direction is a multiple of [`STEP`] in
/// azimuth and in elevation (so the axes are among them).
fn snap_direction(d: Vec3) -> Vec3 {
    let d = normalize(d);
    let round = |deg: f64| (deg / STEP).round() * STEP;
    let el = round(d[2].clamp(-1.0, 1.0).asin().to_degrees());
    let az = round(d[1].atan2(d[0]).to_degrees());
    let ((se, ce), (sa, ca)) = (el.to_radians().sin_cos(), az.to_radians().sin_cos());
    // No rounding noise: zeros and ones are exact.
    [ce * ca, ce * sa, se].map(|x| {
        if x.abs() < 1e-9 {
            0.0
        } else if (x.abs() - 1.0).abs() < 1e-9 {
            x.signum()
        } else {
            x
        }
    })
}

/// The normal `n0` of a plane turned about axis `k` by dragging its ring
/// (centre `c`, radius `rho`) from where it was pressed (`from`, at
/// `pressed`) to `pos`: by the length dragged along the ring there, over the
/// ring's radius. Snapped by [`STEP`], unless `free`.
fn ring_turned(
    view: &ViewPort,
    d: &Drag,
    k: usize,
    n0: Vec3,
    pos: egui::Pos2,
    free: bool,
) -> Option<Vec3> {
    let c = d.handle.base;
    let rho = norm(sub(d.handle.at, c));
    let angle = handles::ring_angle(view, c, k, rho, d.pressed, d.from, pos)?;
    let mut axis = [0.0; 3];
    axis[k] = 1.0;
    let n = handles::turn(normalize(n0), axis, angle);
    Some(if free {
        normalize(n)
    } else {
        snap_direction(n)
    })
}

/// The plane the slice goes to when the tip of its arrow is turned to the
/// direction `d`, from the plane `now` whose middle is `mid`: along an axis,
/// or through the axis `d` is most nearly across (radial).
fn slice_turned(d: Vec3, mid: Vec3, now: PlaneState) -> PlaneState {
    if let Some(k) = cuts::along_axis(d) {
        return PlaneState {
            axis: [AxisName::X, AxisName::Y, AxisName::Z][k],
            radial: None,
            at: mid[k],
        };
    }
    let k = (0..3)
        .min_by(|&a, &b| d[a].abs().total_cmp(&d[b].abs()))
        .unwrap_or(2);
    let (axis, zero) = cuts::RADIAL[k];
    let mut n = d;
    n[k] = 0.0;
    let angle = cuts::radial_angle(k, normalize(n)).unwrap_or(0.0);
    // Turning a radial plane about its axis keeps the axis; any other
    // plane gets its axis through its middle.
    let (point, at) = match now.radial {
        Some(r) if r.axis == axis => (r.point, now.at),
        _ => (mid, 0.0),
    };
    PlaneState {
        axis: now.axis,
        radial: Some(Radial {
            point,
            axis,
            zero,
            phi: angle,
        }),
        at,
    }
}

/// The slice's plane after dragging one of its handles to `pos`.
fn slice_dragged(view: &ViewPort, d: &Drag, pos: egui::Pos2, free: bool) -> Option<PlaneState> {
    let (now, mid) = d.slice?;
    let CutState::Plane {
        point: edge,
        normal: n,
    } = d.start
    else {
        return None;
    };
    match d.handle.grip {
        Grip::Slice => {
            let CutState::Plane { point, .. } = dragged(view, d, pos, free)? else {
                return None;
            };
            Some(PlaneState {
                at: now.at + dot(sub(point, edge), n),
                ..now
            })
        }
        Grip::SliceRing(k) => Some(slice_turned(
            ring_turned(view, d, k, n, pos, free)?,
            mid,
            now,
        )),
        _ => None,
    }
}

/// What of a cut shows as a surface in the view: polygons in world space
/// (each with whether to outline it): the plane's square, a wedge's sector
/// and sides, a box's faces.
fn surfaces(doc: &Document, k: usize) -> Vec<(Vec<Vec3>, bool)> {
    let r = size(doc);
    let c = &doc.scene.cuts[k];
    match *c {
        CutState::Plane { point, normal } => {
            let n = normalize(normal);
            let helper = if n[2].abs() < 0.9 {
                [0.0, 0.0, 1.0]
            } else {
                [1.0, 0.0, 0.0]
            };
            let u = normalize(cross(n, helper));
            let v = cross(n, u);
            let h = 2.0 * r;
            let at = |a: f64, b: f64| add(point, add(scale(u, a * h), scale(v, b * h)));
            vec![(
                vec![at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0)],
                true,
            )]
        }
        CutState::Wedge { from, to, .. } => {
            let base = wedge_centre(doc, c);
            let (a, e0, e90) = wedge_basis(c);
            let on = |deg: f64, rad: f64| {
                let (s, co) = deg.to_radians().sin_cos();
                add(base, scale(add(scale(e0, co), scale(e90, s)), rad))
            };
            let rad = 1.6 * r;
            let steps = (((to - from).abs() / 6.0).ceil() as usize).clamp(2, 60);
            let mut sector = vec![base];
            for i in 0..=steps {
                sector.push(on(from + (to - from) * i as f64 / steps as f64, rad));
            }
            let side = |deg: f64| {
                vec![
                    add(base, scale(a, -rad)),
                    add(on(deg, rad), scale(a, -rad)),
                    add(on(deg, rad), scale(a, rad)),
                    add(base, scale(a, rad)),
                ]
            };
            vec![(sector, false), (side(from), true), (side(to), true)]
        }
        CutState::Box { lo, hi, .. } => {
            let mut faces = Vec::new();
            for d in 0..3 {
                let (e1, e2) = ((d + 1) % 3, (d + 2) % 3);
                for high in [false, true] {
                    let corner = |s1: bool, s2: bool| {
                        let mut p = lo;
                        p[d] = if high { hi[d] } else { lo[d] };
                        p[e1] = if s1 { hi[e1] } else { lo[e1] };
                        p[e2] = if s2 { hi[e2] } else { lo[e2] };
                        p
                    };
                    faces.push((
                        vec![
                            corner(false, false),
                            corner(true, false),
                            corner(true, true),
                            corner(false, true),
                        ],
                        false,
                    ));
                }
            }
            faces
        }
    }
}

/// The polygon in screen points, if all its corners are in front of the eye.
fn on_screen(view: &ViewPort, poly: &[Vec3]) -> Option<Vec<egui::Pos2>> {
    poly.iter()
        .map(|&p| view.to_screen(p).map(|s| s.pos))
        .collect()
}

impl CutHandles {
    fn under(view: &ViewPort, doc: &Document, pos: egui::Pos2) -> Option<Handle> {
        handles(doc)
            .into_iter()
            .filter_map(|h| {
                let d = match h.grip {
                    Grip::Ring(k) | Grip::SliceRing(k) => {
                        handles::ring_distance(view, h.base, k, norm(sub(h.at, h.base)), pos)?
                    }
                    _ => (view.to_screen(h.at)?.pos - pos).length(),
                };
                (d < GRAB).then_some((h, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(h, _)| h)
    }

    /// The cut whose outline is under `pos`: the last drawn wins. (Not the
    /// slice plane: its outline runs along the model's edges, where clicks
    /// are for the cells; its handles select it.)
    fn outline_under(view: &ViewPort, doc: &Document, pos: egui::Pos2) -> Option<usize> {
        let near = |segs: &[(Vec3, Vec3)]| {
            segs.iter().any(|&(a, b)| {
                let (Some(a), Some(b)) = (view.to_screen(a), view.to_screen(b)) else {
                    return false;
                };
                segment_distance(pos, a.pos, b.pos) < GRAB
            })
        };
        (0..doc.scene.cuts.len())
            .rev()
            .find(|&k| shown(doc, k) && near(&outline(doc, k)))
    }

    /// The cut (or [`SLICE`]) under `pos`: one of its handles or its outline.
    fn cut_under(view: &ViewPort, doc: &Document, pos: egui::Pos2) -> Option<usize> {
        Self::under(view, doc, pos)
            .map(|h| h.cut)
            .or_else(|| Self::outline_under(view, doc, pos))
    }
}

/// Whether cut `k` is drawn and handled in 3D: it is on, and cuts are shown
/// (the Section menu's "show cuts in 3D") or it is the one selected.
fn shown(doc: &Document, k: usize) -> bool {
    doc.scene.cut_on(k) && (doc.show_cuts || doc.editing.cut_selected == Some(k))
}

/// Select the cut `k` (or the slice plane), if it is not already.
fn select(doc: &Document, k: usize, out: &mut Vec<Command>) {
    if k == SLICE {
        if !doc.editing.slice_selected {
            out.push(Command::SelectSlice(true));
        }
    } else if doc.editing.cut_selected != Some(k) {
        out.push(Command::SelectCut(Some(k)));
    }
}

/// Distance (screen points) from `p` to the segment `a`–`b`.
fn segment_distance(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + t * ab - p).length()
}

/// The lines a cut is drawn with (and picked by): its outlined polygons'
/// sides, a box's twelve edges.
fn outline(doc: &Document, k: usize) -> Vec<(Vec3, Vec3)> {
    let mut segs = Vec::new();
    for (poly, outlined) in surfaces(doc, k) {
        if outlined {
            for i in 0..poly.len() {
                segs.push((poly[i], poly[(i + 1) % poly.len()]));
            }
        }
    }
    if let CutState::Box { lo, hi, .. } = doc.scene.cuts[k] {
        for d in 0..3 {
            let (e1, e2) = ((d + 1) % 3, (d + 2) % 3);
            for (s1, s2) in [(false, false), (false, true), (true, false), (true, true)] {
                let mut p = lo;
                p[e1] = if s1 { hi[e1] } else { lo[e1] };
                p[e2] = if s2 { hi[e2] } else { lo[e2] };
                let mut q = p;
                p[d] = lo[d];
                q[d] = hi[d];
                segs.push((p, q));
            }
        }
    }
    segs
}

impl Layer for CutHandles {
    fn hint(&self, doc: &Document) -> Option<&'static str> {
        (doc.editing.cut_selected.is_some() || doc.editing.slice_selected).then_some(
            "drag the handles (the rings turn about x, y or z in 15° steps; Shift or Alt: \
             freely) · click elsewhere or Esc: done · the 3D view's Section menu: by numbers",
        )
    }

    fn input(&mut self, view: &ViewPort, doc: &Document, out: &mut Vec<Command>) -> bool {
        if view.kind != ViewKind::ThreeD {
            return false;
        }
        let r = &view.response;
        if r.drag_started_by(egui::PointerButton::Primary) {
            let origin = r.ctx.input(|i| i.pointer.press_origin());
            self.drag = origin.and_then(|from| {
                let handle = Self::under(view, doc, from)?;
                let slice = matches!(handle.grip, Grip::Slice | Grip::SliceRing(_));
                let (edge, mid, n) = slice_geometry(doc);
                let start = if slice {
                    // The slice as a plane, for the drag to move.
                    CutState::Plane {
                        point: edge,
                        normal: n,
                    }
                } else {
                    *doc.scene.cuts.get(handle.cut)?
                };
                let pressed = match handle.grip {
                    Grip::Ring(k) | Grip::SliceRing(k) => {
                        let rho = norm(sub(handle.at, handle.base));
                        handles::nearest_on_ring(view, handle.base, k, rho, from)?.0
                    }
                    _ => handle.at,
                };
                Some(Drag {
                    handle,
                    start,
                    slice: slice.then(|| (doc.slice.plane_state(), mid)),
                    from,
                    pressed,
                })
            });
            if let Some(d) = &self.drag {
                select(doc, d.handle.cut, out);
            }
        }
        if r.clicked_by(egui::PointerButton::Primary) {
            if let Some(pos) = r.interact_pointer_pos() {
                match Self::cut_under(view, doc, pos) {
                    Some(k) => {
                        select(doc, k, out);
                        return true;
                    }
                    None => {
                        if doc.editing.cut_selected.is_some() || doc.editing.slice_selected {
                            out.push(Command::SelectCut(None));
                        }
                        return false;
                    }
                }
            }
        }
        let Some(d) = &self.drag else {
            return false;
        };
        if r.dragged_by(egui::PointerButton::Primary) {
            let free = r.ctx.input(|i| i.modifiers.shift || i.modifiers.alt);
            if let Some(pos) = r.interact_pointer_pos() {
                if d.slice.is_some() {
                    if let Some(p) = slice_dragged(view, d, pos, free) {
                        out.push(Command::SlicePlane(p));
                    }
                } else if let Some(c) = dragged(view, d, pos, free) {
                    out.push(Edit::SetCut(d.handle.cut, c).into());
                }
            }
            return true;
        }
        self.drag = None;
        false
    }

    fn overlay(&self, view: &ViewPort, doc: &Document, painter: &egui::Painter) {
        if view.kind != ViewKind::ThreeD {
            return;
        }
        let pointer = view.response.hover_pos();
        let hover = pointer.and_then(|p| Self::under(view, doc, p));
        // The cut a click would select.
        let hot_cut = match &self.drag {
            Some(d) => Some(d.handle.cut),
            None => pointer.and_then(|p| Self::cut_under(view, doc, p)),
        };
        let r = size(doc);
        if doc.slice_shown_in_3d() && (doc.editing.slice_selected || hot_cut == Some(SLICE)) {
            // The slice plane, outlined over its usual outline.
            let c = doc.slice.plane().corners();
            let width = if doc.editing.slice_selected { 2.5 } else { 2.0 };
            for i in 0..4 {
                let stroke = egui::Stroke::new(width, theme::PIN);
                handles::seg(view, painter, c[i], c[(i + 1) % 4], stroke);
            }
        }
        for (k, c) in doc.scene.cuts.iter().enumerate() {
            if !shown(doc, k) {
                continue;
            }
            let selected = doc.editing.cut_selected == Some(k);
            let hot = hot_cut == Some(k);
            // Selected: filled and in the selection's colour; else a faint
            // outline (stronger under the pointer).
            let colour = if selected {
                theme::SELECTION
            } else {
                theme::PIVOT
            };
            let line = match (selected, hot) {
                (true, _) => egui::Stroke::new(2.5, colour),
                (false, true) => egui::Stroke::new(2.0, colour),
                (false, false) => egui::Stroke::new(1.0, colour.gamma_multiply(0.6)),
            };
            let fill = if selected {
                colour.gamma_multiply(0.22)
            } else {
                egui::Color32::TRANSPARENT
            };
            for (poly, outline) in surfaces(doc, k) {
                if !selected && !outline {
                    continue;
                }
                if let Some(pts) = on_screen(view, &poly) {
                    let stroke = if outline { line } else { egui::Stroke::NONE };
                    painter.add(egui::Shape::convex_polygon(pts, fill, stroke));
                }
            }
            match *c {
                CutState::Wedge { axis, .. } => {
                    let base = wedge_centre(doc, c);
                    let a = normalize(axis);
                    handles::seg(
                        view,
                        painter,
                        sub(base, scale(a, r)),
                        add(base, scale(a, r)),
                        line,
                    );
                }
                CutState::Box { lo, hi, .. } => {
                    // The twelve edges.
                    for d in 0..3 {
                        let (e1, e2) = ((d + 1) % 3, (d + 2) % 3);
                        for (s1, s2) in [(false, false), (false, true), (true, false), (true, true)]
                        {
                            let mut p = lo;
                            p[e1] = if s1 { hi[e1] } else { lo[e1] };
                            p[e2] = if s2 { hi[e2] } else { lo[e2] };
                            let mut q = p;
                            p[d] = lo[d];
                            q[d] = hi[d];
                            handles::seg(view, painter, p, q, line);
                        }
                    }
                }
                CutState::Plane { .. } => {}
            }
        }
        // The normals of the selected plane and of the slice, as arrows.
        let mut arrows = Vec::new();
        if let Some(CutState::Plane { point, normal }) = doc
            .editing
            .cut_selected
            .filter(|&k| shown(doc, k))
            .and_then(|k| doc.scene.cuts.get(k))
            .copied()
        {
            arrows.push((point, normalize(normal), theme::SELECTION));
        }
        if doc.slice_shown_in_3d() {
            let (edge, _, n) = slice_geometry(doc);
            arrows.push((edge, n, theme::PIN));
        }
        for (from, n, colour) in arrows {
            handles::seg(
                view,
                painter,
                from,
                add(from, scale(n, r)),
                egui::Stroke::new(2.0, colour),
            );
        }
        for h in handles(doc) {
            let hot = hover.is_some_and(|x| x.cut == h.cut && x.grip == h.grip)
                || self
                    .drag
                    .as_ref()
                    .is_some_and(|d| d.handle.cut == h.cut && d.handle.grip == h.grip);
            if let Grip::Ring(k) | Grip::SliceRing(k) = h.grip {
                // A ring about an axis, in the axis's colour.
                let width = if hot { 4.0 } else { 2.0 };
                let colour = AXIS_COLOURS[k].gamma_multiply(if hot { 1.0 } else { 0.8 });
                let pts: Vec<egui::Pos2> = handles::ring_points(h.base, k, norm(sub(h.at, h.base)))
                    .into_iter()
                    .filter_map(|p| view.to_screen(p).map(|s| s.pos))
                    .collect();
                painter.add(egui::Shape::line(pts, egui::Stroke::new(width, colour)));
                continue;
            }
            let ink = if h.cut == SLICE {
                theme::PIN
            } else {
                theme::PIVOT
            };
            let line = egui::Stroke::new(2.0, ink);
            if matches!(h.grip, Grip::Side(_)) {
                handles::seg(view, painter, h.base, h.at, line);
            }
            if let Some(s) = view.to_screen(h.at) {
                let colour = if hot { theme::SELECTION } else { ink };
                painter.circle(
                    s.pos,
                    if hot { 7.0 } else { 5.5 },
                    colour,
                    egui::Stroke::new(1.5, egui::Color32::from_black_alpha(160)),
                );
            }
        }
    }
}
