//! FMESH tallies laid out over the model, in the Tally workspace, whatever
//! the tool ([`super::Layer`]). The slice shows each mesh's voxels as cut by
//! the plane (`mesh::draw::raster`), the 3D view their wireframes. A click on
//! a mesh selects it (and is not a click on the cells); a click elsewhere
//! deselects it and goes on to the tool. The selected mesh's handles move its
//! outer bounds (coarse bins keep their numbers of fine bins), move it whole,
//! and in 3D turn it about x, y or z (rings, in 15° steps). Bounds and moves
//! are rounded to steps that look alike at any zoom, unless Shift is held.
//!
//! The FMESH panel's "+ Box" and "+ Cylinder" (`panels::mesh`) arm a drag in
//! the slice that draws a new mesh: a box (along the model's axes, or the
//! plane's with a `TR=` on a radial slice) or a cylinder about the plane's
//! normal, from the drag's start out to it, as deep as the model. The panel
//! shows the selected mesh in detail and its cards.

use std::cell::RefCell;

use llevant::vec3::{add, dot, norm, scale, sub};
use llevant::{Aabb, Vec3};

use crate::command::{Command, Edit};
use crate::cuts;
use crate::document::Document;
use crate::mesh::draw::{self, Detail, Grip, Handle};
use crate::mesh::{nice_step, snap, InputMeshes, MeshState};
use crate::slice::PlaneView;
use crate::theme;
use crate::tools::{handles, Layer};
use crate::viewport::{Projection, ViewKind, ViewPort};

/// Pick distance of a handle (screen points).
const GRAB: f32 = 9.0;

/// Rounding steps: about this many screen points.
const STEP_POINTS: f64 = 10.0;

/// Turns by a ring go in steps of this many degrees (Shift: freely).
const TURN_STEP: f64 = 15.0;

/// A drag in progress.
enum Drag {
    /// Making a mesh in the slice, from `from` to `to` (screen points).
    New { from: egui::Pos2, to: egui::Pos2 },
    /// Moving a handle of mesh `k`, as it was at `start`.
    Handle {
        k: usize,
        handle: Handle,
        start: Box<MeshState>,
        from: egui::Pos2,
        /// For a ring: the point of it that was pressed.
        pressed: Vec3,
    },
}

/// The slice's image of one mesh and what it was made from.
struct Cached {
    key: (MeshState, PlaneView, [f32; 4], bool),
    texture: egui::TextureHandle,
    detail: [Detail; 3],
}

#[derive(Default)]
pub struct MeshHandles {
    drag: Option<Drag>,
    /// The view the drag is in (the layer hears from both every frame).
    drag_view: Option<ViewKind>,
    /// The slice's images of the scene's meshes, by index.
    images: RefCell<Vec<Option<Cached>>>,
}

/// The plane's axes in the slice, for handles.
fn plane_axes(view: &ViewPort) -> Option<[Vec3; 3]> {
    match &view.projection {
        Projection::Plane(p) => Some([p.frame.u, p.frame.v, p.frame.n]),
        Projection::Camera(_) => None,
    }
}

/// The rings' radius about mesh `m`.
fn ring_radius(m: &MeshState) -> f64 {
    m.world_box().map_or(1.0, |b| {
        0.35 * (0..3)
            .map(|k| b.hi[k] - b.lo[k])
            .fold(0.0, f64::max)
            .max(1e-3)
    })
}

/// A step to round to at `at`, about [`STEP_POINTS`] on screen.
fn step_at(view: &ViewPort, at: Vec3, dir: Vec3) -> f64 {
    let dir = if norm(dir) > 0.0 {
        dir
    } else {
        match &view.projection {
            Projection::Plane(p) => p.frame.u,
            Projection::Camera(c) => c.basis()[1],
        }
    };
    handles::length_per_point(view, at, dir).map_or(1.0, |l| nice_step(STEP_POINTS * l))
}

/// The selected mesh's handle under `pos`.
fn under(view: &ViewPort, doc: &Document, pos: egui::Pos2) -> Option<(usize, Handle)> {
    let k = doc.editing.mesh_selected?;
    let m = doc.scene.meshes.get(k)?;
    let rho = ring_radius(m);
    draw::handles(m, plane_axes(view))
        .into_iter()
        .filter_map(|h| {
            let d = match h.grip {
                Grip::Ring(r) => handles::ring_distance(view, h.at, r, rho, pos)?,
                _ => (view.to_screen(h.at)?.pos - pos).length(),
            };
            (d < GRAB).then_some((h, d))
        })
        // Handles before rings.
        .min_by(|a, b| {
            let ring = |h: &Handle| matches!(h.grip, Grip::Ring(_));
            (ring(&a.0), a.1)
                .partial_cmp(&(ring(&b.0), b.1))
                .expect("finite")
        })
        .map(|(h, _)| (k, h))
}

/// The mesh at `pos`: in the slice, one the point is in; in 3D, the
/// nearest whose box the pointer's ray meets. The selected one first.
fn mesh_at(view: &ViewPort, doc: &Document, pos: egui::Pos2) -> Option<usize> {
    let meshes = &doc.scene.meshes;
    let order = doc.editing.mesh_selected.into_iter().chain(
        (0..meshes.len())
            .rev()
            .filter(|&k| Some(k) != doc.editing.mesh_selected),
    );
    match &view.projection {
        Projection::Plane(plane) => {
            let p = plane.from_screen(view.rect(), pos);
            order
                .into_iter()
                .find(|&k| meshes[k].geometry().is_ok_and(|g| g.locate(p).is_some()))
        }
        Projection::Camera(cam) => {
            let (i, j) = view.pixel(pos)?;
            let (o, d) = (cam.origin(i, j), cam.direction(i, j));
            let hit = |k: usize| meshes[k].world_box().and_then(|b| ray_box(o, d, &b));
            let first = order.into_iter().filter_map(|k| Some((k, hit(k)?)));
            first.min_by(|a, b| a.1.total_cmp(&b.1)).map(|(k, _)| k)
        }
    }
}

/// Where the ray from `o` along `d` enters `b` (its distance), if it does.
fn ray_box(o: Vec3, d: Vec3, b: &Aabb) -> Option<f64> {
    let (mut t0, mut t1) = (0.0f64, f64::INFINITY);
    for k in 0..3 {
        if d[k].abs() < 1e-300 {
            if o[k] < b.lo[k] || o[k] > b.hi[k] {
                return None;
            }
            continue;
        }
        let (a, c) = ((b.lo[k] - o[k]) / d[k], (b.hi[k] - o[k]) / d[k]);
        t0 = t0.max(a.min(c));
        t1 = t1.min(a.max(c));
    }
    (t0 <= t1).then_some(t0)
}

/// The mesh a drag in the slice from `from` to `to` makes, or none if it
/// is too short.
fn new_mesh(
    view: &ViewPort,
    doc: &Document,
    from: egui::Pos2,
    to: egui::Pos2,
    free: bool,
) -> Option<MeshState> {
    let Projection::Plane(plane) = &view.projection else {
        return None;
    };
    if (to - from).length() < 4.0 {
        return None;
    }
    let rect = view.rect();
    let f = plane.frame;
    let step = nice_step(STEP_POINTS * plane.cm_per_point(rect));
    let round = |x: f64| if free { x } else { snap(x, step) };
    let (a, _) = f.coords(plane.from_screen(rect, from));
    let (b, _) = f.coords(plane.from_screen(rect, to));
    let [a, b] = [a, b].map(|c| c.map(round));
    let id = InputMeshes::free_tally(doc.input_meshes.as_deref(), &doc.scene.meshes);
    // How far the model goes either way along `n` from `p`, rounded out.
    let depth = |p: Vec3, n: Vec3| {
        let r = &doc.region;
        let along: Vec<f64> = (0..8)
            .map(|k| {
                let c = [0, 1, 2].map(|d| if k >> d & 1 == 1 { r.hi[d] } else { r.lo[d] });
                dot(sub(c, p), n)
            })
            .collect();
        let lo = along.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = along.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let s = nice_step((hi - lo) / 100.0);
        ((lo / s).floor() * s, (hi / s).ceil() * s)
    };
    if doc.editing.drawing_mesh == Some(true) {
        let centre = f.point(a);
        let radius = norm(sub(f.point(b), centre));
        if !(radius > 0.0) {
            return None;
        }
        let (lo, hi) = depth(centre, f.n);
        let base = add(centre, scale(f.n, lo));
        return Some(MeshState::cylinder(id, base, f.n, f.u, radius, hi - lo));
    }
    let (lo2, hi2) = (
        [a[0].min(b[0]), a[1].min(b[1])],
        [a[0].max(b[0]), a[1].max(b[1])],
    );
    if !(lo2[0] < hi2[0] && lo2[1] < hi2[1]) {
        return None;
    }
    match cuts::along_axis(f.n) {
        // Along the model's axes: the plane's coordinates are the model's.
        Some(n) => {
            let (u, v) = (cuts::along_axis(f.u)?, cuts::along_axis(f.v)?);
            let (dlo, dhi) = depth([0.0; 3], f.n);
            let (mut lo, mut hi) = ([0.0; 3], [0.0; 3]);
            (lo[u], hi[u]) = (lo2[0], hi2[0]);
            (lo[v], hi[v]) = (lo2[1], hi2[1]);
            // The normal may point down its axis.
            let s = f.n[n];
            (lo[n], hi[n]) = if s > 0.0 { (dlo, dhi) } else { (-dhi, -dlo) };
            Some(MeshState::rect(id, lo, hi))
        }
        // Along the plane's axes, placed by a TR.
        None => {
            let n = llevant::vec3::cross(f.u, f.v);
            let (dlo, dhi) = depth(f.origin, n);
            let tr = InputMeshes::free_tr(doc.input_meshes.as_deref(), &doc.scene.meshes);
            Some(MeshState::rect_in_frame(
                id,
                tr,
                f.origin,
                [f.u, f.v, n],
                [lo2[0], lo2[1], dlo],
                [hi2[0], hi2[1], dhi],
            ))
        }
    }
}

/// Mesh `start` after its handle was dragged from `from` to `pos`.
fn dragged(
    view: &ViewPort,
    doc: &Document,
    d: &Drag,
    pos: egui::Pos2,
    free: bool,
) -> Option<MeshState> {
    let Drag::Handle {
        handle: h,
        start,
        from,
        pressed,
        ..
    } = d
    else {
        return None;
    };
    let step = step_at(view, h.at, h.dir);
    let round = |x: f64| if free { x } else { snap(x, step) };
    match h.grip {
        Grip::End(k, high) => {
            let t = handles::along(view, h.at, h.dir, *from, pos)?;
            if start.is_cylinder() && k == 1 && !high {
                return start.base_moved(round(t));
            }
            let v = start.axes()[k];
            let end = if high { *v.last()? } else { v[0] };
            let w = crate::mesh::moved_end(v, high, round(end + t))?;
            let mut m = (**start).clone();
            *m.axes_mut()[k] = w;
            Some(m)
        }
        Grip::Move => {
            let delta = match &view.projection {
                Projection::Plane(p) => {
                    let r = view.rect();
                    sub(p.from_screen(r, pos), p.from_screen(r, *from))
                }
                Projection::Camera(c) => {
                    let [_, right, up] = c.basis();
                    let a = handles::along(view, h.at, right, *from, pos)?;
                    let b = handles::along(view, h.at, up, *from, pos)?;
                    add(scale(right, a), scale(up, b))
                }
            };
            Some(start.moved(delta.map(round)))
        }
        Grip::Ring(k) => {
            let rho = ring_radius(start);
            let angle = handles::ring_angle(view, h.at, k, rho, *pressed, *from, pos)?;
            let angle = if free {
                angle
            } else {
                (angle.to_degrees() / TURN_STEP).round() * TURN_STEP.to_radians()
            };
            let tr = InputMeshes::free_tr(doc.input_meshes.as_deref(), &doc.scene.meshes);
            Some(start.turned(h.dir, angle, h.at, tr))
        }
    }
}

impl MeshHandles {
    /// The slice's image of mesh `k`, made again if it or the view
    /// changed: its texture and how much it shows.
    fn image(
        &self,
        k: usize,
        m: &MeshState,
        plane: &PlaneView,
        rect: egui::Rect,
        selected: bool,
        ctx: &egui::Context,
    ) -> Option<(egui::TextureId, [Detail; 3])> {
        let key = (
            m.clone(),
            *plane,
            [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
            selected,
        );
        let mut images = self.images.borrow_mut();
        if images.len() <= k {
            images.resize_with(k + 1, || None);
        }
        if images[k].as_ref().is_none_or(|c| c.key != key) {
            let g = m.geometry().ok()?;
            let ink = if selected {
                theme::SELECTION
            } else {
                theme::MESH
            };
            let (nx, ny) = (
                (rect.width().round() as usize).clamp(1, 4096),
                (rect.height().round() as usize).clamp(1, 4096),
            );
            let r = draw::raster(&g, plane, rect, nx, ny, ink);
            let texture = ctx.load_texture("mesh", r.image, egui::TextureOptions::NEAREST);
            images[k] = Some(Cached {
                key,
                texture,
                detail: r.detail,
            });
        }
        images[k].as_ref().map(|c| (c.texture.id(), c.detail))
    }
}

impl Layer for MeshHandles {
    fn hint(&self, doc: &Document) -> Option<&'static str> {
        match (doc.editing.drawing_mesh, doc.editing.mesh_selected) {
            (Some(true), _) => Some(
                "drag in the slice from the axis out: a new FMESH cylinder · Shift: no rounding · \
                 Esc: cancel",
            ),
            (Some(false), _) => {
                Some("drag in the slice: a new FMESH box · Shift: no rounding · Esc: cancel")
            }
            (None, Some(_)) => Some(
                "drag its handles: bounds, move, turn (3D rings) · Shift: no rounding · \
                 Delete: remove it · click another mesh: select it",
            ),
            (None, None) => {
                Some("click a mesh: select it · new ones: + Box, + Cylinder in the FMESH panel")
            }
        }
    }

    fn input(&mut self, view: &ViewPort, doc: &Document, out: &mut Vec<Command>) -> bool {
        let r = &view.response;
        let free = r.ctx.input(|i| i.modifiers.shift);
        if r.hovered() && !r.ctx.egui_wants_keyboard_input() {
            if let Some(k) = doc.editing.mesh_selected {
                if r.ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
                    out.push(Edit::RemoveMesh(k).into());
                }
            }
        }
        if self.drag.is_some() && self.drag_view != Some(view.kind) {
            // Being dragged in the other view.
            return false;
        }
        if r.drag_started_by(egui::PointerButton::Primary) {
            self.drag_view = Some(view.kind);
            let origin = r.ctx.input(|i| i.pointer.press_origin());
            self.drag = origin.and_then(|from| match under(view, doc, from) {
                Some((k, handle)) => {
                    let start = doc.scene.meshes.get(k)?.clone();
                    let pressed = match handle.grip {
                        Grip::Ring(axis) => {
                            handles::nearest_on_ring(
                                view,
                                handle.at,
                                axis,
                                ring_radius(&start),
                                from,
                            )?
                            .0
                        }
                        _ => handle.at,
                    };
                    Some(Drag::Handle {
                        k,
                        handle,
                        start: Box::new(start),
                        from,
                        pressed,
                    })
                }
                None if view.kind == ViewKind::Slice && doc.editing.drawing_mesh.is_some() => {
                    Some(Drag::New { from, to: from })
                }
                None => None,
            });
        }
        if r.clicked_by(egui::PointerButton::Primary) {
            if let Some(pos) = r.interact_pointer_pos() {
                if view.kind == ViewKind::Slice && doc.editing.drawing_mesh.is_some() {
                    // A drag draws; a click does nothing.
                    return true;
                }
                let hit = mesh_at(view, doc, pos);
                if hit != doc.editing.mesh_selected {
                    out.push(Command::SelectMesh(hit));
                }
                return hit.is_some();
            }
        }
        if self.drag.is_none() {
            return false;
        }
        if r.dragged_by(egui::PointerButton::Primary) {
            let Some(pos) = r.interact_pointer_pos() else {
                return true;
            };
            match &mut self.drag {
                Some(Drag::New { to, .. }) => *to = pos,
                Some(d @ Drag::Handle { .. }) => {
                    let Drag::Handle { k, .. } = d else {
                        unreachable!()
                    };
                    let k = *k;
                    if let Some(m) = dragged(view, doc, d, pos, free) {
                        if doc.scene.meshes.get(k) != Some(&m) {
                            out.push(Edit::SetMesh(k, Box::new(m)).into());
                        }
                    }
                }
                None => {}
            }
            return true;
        }
        if let Some(Drag::New { from, to }) = self.drag.take() {
            if let Some(m) = new_mesh(view, doc, from, to, free) {
                out.push(Edit::AddMesh(Box::new(m)).into());
            }
        }
        true
    }

    fn overlay(&self, view: &ViewPort, doc: &Document, painter: &egui::Painter) {
        let rect = view.rect();
        let mut notes = Vec::new();
        for (k, m) in doc.scene.meshes.iter().enumerate() {
            let selected = doc.editing.mesh_selected == Some(k);
            match &view.projection {
                Projection::Plane(plane) => {
                    if let Some((tex, detail)) =
                        self.image(k, m, plane, rect, selected, painter.ctx())
                    {
                        let uv =
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        painter.image(tex, rect, uv, egui::Color32::WHITE);
                        let names = crate::mesh::axis_names(m.is_cylinder());
                        for (d, name) in detail.iter().zip(names) {
                            match d {
                                Detail::Coarse => {
                                    notes.push(format!("FMESH{}: {name} coarse bins only", m.id))
                                }
                                Detail::Hidden => notes
                                    .push(format!("FMESH{}: {name} bins too small to show", m.id)),
                                Detail::Fine => {}
                            }
                        }
                    }
                }
                Projection::Camera(_) => {
                    let ink = if selected {
                        theme::SELECTION
                    } else {
                        theme::MESH
                    };
                    for (line, outline) in draw::wireframe(m) {
                        let stroke = if outline {
                            egui::Stroke::new(if selected { 2.5 } else { 1.8 }, ink)
                        } else {
                            egui::Stroke::new(1.0, ink.gamma_multiply(0.6))
                        };
                        let pts: Option<Vec<egui::Pos2>> = line
                            .iter()
                            .map(|&p| view.to_screen(p).map(|s| s.pos))
                            .collect();
                        if let Some(pts) = pts {
                            painter.add(egui::Shape::line(pts, stroke));
                        }
                    }
                }
            }
        }
        // The selected mesh's handles.
        let hover = view.response.hover_pos().and_then(|p| under(view, doc, p));
        if let Some(m) = doc
            .editing
            .mesh_selected
            .and_then(|k| doc.scene.meshes.get(k))
        {
            let rho = ring_radius(m);
            let held = match &self.drag {
                Some(Drag::Handle { handle, .. }) => Some(handle.grip),
                _ => None,
            };
            for h in draw::handles(m, plane_axes(view)) {
                let hot = held == Some(h.grip) || hover.is_some_and(|(_, x)| x.grip == h.grip);
                match h.grip {
                    Grip::Ring(k) => {
                        let colour = AXIS_COLOURS[k].gamma_multiply(if hot { 1.0 } else { 0.7 });
                        let pts: Vec<egui::Pos2> = handles::ring_points(h.at, k, rho)
                            .into_iter()
                            .filter_map(|p| view.to_screen(p).map(|s| s.pos))
                            .collect();
                        let width = if hot { 4.0 } else { 2.0 };
                        painter.add(egui::Shape::line(pts, egui::Stroke::new(width, colour)));
                    }
                    _ => {
                        if let Some(s) = view.to_screen(h.at) {
                            let colour = if hot { theme::SELECTION } else { theme::MESH };
                            let outline =
                                egui::Stroke::new(1.5, egui::Color32::from_black_alpha(160));
                            let radius = if hot { 7.0 } else { 5.5 };
                            if h.grip == Grip::Move {
                                let r = egui::Rect::from_center_size(
                                    s.pos,
                                    egui::Vec2::splat(2.0 * radius),
                                );
                                painter.rect(r, 2.0, colour, outline, egui::StrokeKind::Middle);
                            } else {
                                painter.circle(s.pos, radius, colour, outline);
                            }
                        }
                    }
                }
            }
        }
        // The mesh being made.
        if let Some(Drag::New { from, to }) = &self.drag {
            let free = view.response.ctx.input(|i| i.modifiers.shift);
            if let Some(m) = new_mesh(view, doc, *from, *to, free) {
                if let Projection::Plane(plane) = &view.projection {
                    if let Ok(g) = m.geometry() {
                        let nx = (rect.width().round() as usize).clamp(1, 4096);
                        let ny = (rect.height().round() as usize).clamp(1, 4096);
                        let r = draw::raster(&g, plane, rect, nx, ny, theme::SELECTION);
                        let tex = painter.ctx().load_texture(
                            "new mesh",
                            r.image,
                            egui::TextureOptions::NEAREST,
                        );
                        let uv =
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        painter.image(tex.id(), rect, uv, egui::Color32::WHITE);
                        notes.push(describe(&m));
                    }
                }
            }
        }
        // The voxel under the pointer.
        if let (Projection::Plane(plane), Some(pos)) = (&view.projection, view.response.hover_pos())
        {
            let p = plane.from_screen(rect, pos);
            let meshes = &doc.scene.meshes;
            let order = doc
                .editing
                .mesh_selected
                .into_iter()
                .chain((0..meshes.len()).filter(|&k| Some(k) != doc.editing.mesh_selected));
            for k in order {
                let Ok(g) = meshes[k].geometry() else {
                    continue;
                };
                if let Some(v) = g.locate(p) {
                    let (i, j, l) = g.ijk(v);
                    notes.push(format!(
                        "FMESH{} voxel ({}, {}, {}) of ({}, {}, {})",
                        meshes[k].id,
                        i + 1,
                        j + 1,
                        l + 1,
                        g.dims().0,
                        g.dims().1,
                        g.dims().2
                    ));
                    break;
                }
            }
        }
        let mut y = rect.max.y - 8.0;
        for note in notes.iter().rev() {
            let galley = painter.layout_no_wrap(
                note.clone(),
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
            let at = egui::pos2(rect.min.x + 8.0, y - galley.size().y);
            painter.rect_filled(
                egui::Rect::from_min_size(at, galley.size()).expand(3.0),
                3.0,
                egui::Color32::from_black_alpha(170),
            );
            painter.galley(at, galley, egui::Color32::WHITE);
            y -= 22.0;
        }
    }
}

/// The colours of the x, y and z rings (as the axes gizmo has them).
const AXIS_COLOURS: [egui::Color32; 3] = [
    egui::Color32::from_rgb(230, 80, 80),
    egui::Color32::from_rgb(110, 200, 90),
    egui::Color32::from_rgb(90, 150, 255),
];

/// A mesh's size, as the slice notes it while one is being made.
fn describe(m: &MeshState) -> String {
    let ax = m.axes();
    let len = |v: &Vec<f64>| v.last().copied().unwrap_or(0.0) - v[0];
    if m.is_cylinder() {
        format!(
            "new FMESH{}: cylinder r {} cm, {} cm long",
            m.id,
            crate::mesh::number(len(ax[0])),
            crate::mesh::number(len(ax[1]))
        )
    } else {
        format!(
            "new FMESH{}: {} × {} × {} cm",
            m.id,
            crate::mesh::number(len(ax[0])),
            crate::mesh::number(len(ax[1])),
            crate::mesh::number(len(ax[2]))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rays_enter_boxes() {
        let b = Aabb {
            lo: [0.0; 3],
            hi: [1.0; 3],
        };
        assert_eq!(ray_box([-1.0, 0.5, 0.5], [1.0, 0.0, 0.0], &b), Some(1.0));
        assert_eq!(ray_box([-1.0, 2.0, 0.5], [1.0, 0.0, 0.0], &b), None);
        assert_eq!(ray_box([0.5; 3], [0.0, 0.0, 1.0], &b), Some(0.0));
    }
}
