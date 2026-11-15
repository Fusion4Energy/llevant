//! Surfaces marked on the slice while a cell's geometry is edited
//! (decision 0012, phase 8): where each lies on the plane, drawn over the
//! image. Found on a grid over the plane shown (off the UI thread): each
//! node taken into the frame of the cell's universe there
//! ([`llevant::Model::point_in_unit`]), the side of each surface read, and
//! a line drawn across each square of nodes whose sides differ, within one
//! instance of the universe, each crossing found along its edge by halving
//! (well below a screen point). A feature thinner than a grid square can
//! still be missed: the grid is not the exact curve.
//!
//! A geometry being edited is drawn too, before it is applied: the cell
//! as edited compiled alone, with its `TRCL`, the cells its `#n` name and
//! the surfaces and transforms they use
//! ([`llevant_scene::source::Source::cell_alone`]), asked at each node
//! whether it holds it ([`llevant::Model::cell_holds`]), and the edge of
//! where it does traced the same way ([`EDITED`]).
//!
//! The 3D view draws both too ([`Marks::for_3d`], [`crate::view3d::edits`]):
//! the surfaces written in the problem's frame for the instance of the
//! universe at the point inspected, and the cell as edited from a trace of
//! its mini model alone, at a reduced size, off the UI thread.

use std::sync::Arc;

use llevant::{Affine, Camera, Model, Vec3};
use llevant_scene::source::Source;

use crate::slice::PlaneView;
use crate::tasks::Latest;
use crate::view3d::edits::{surface_words, Shell};

/// Nodes across the plane.
const ACROSS: usize = 220;
/// Cells of the universe searched for a surface the cell does not use yet.
const SEARCHED: usize = 20_000;

/// [`Seg::surface`] of the edge of the cell as edited.
pub const EDITED: i64 = i64::MIN;

/// What to mark: surfaces (MCNP numbers) as cell `cell` sees them (in the
/// frame of its universe; the problem's for a number the model has not
/// got), and a cell drawn as it would be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkRequest {
    pub cell: i64,
    pub surfaces: Vec<i64>,
    pub preview: Option<Preview>,
    /// For a cell in a universe, the point (its bits) whose instance the
    /// 3D view marks; `None` for the point inspected.
    pub at: Option<[u64; 3]>,
}

impl MarkRequest {
    /// Compiled surface `shape` at the point picked `p`, as the cell of
    /// its path whose card uses it sees it (the deepest cell, if none
    /// does); `None` outside every cell.
    pub fn surface(m: &Model, p: &crate::pick::Picked, shape: u32) -> Option<MarkRequest> {
        let id = m.surface_id(shape)?.id;
        let path = &p.label.path;
        let uses = |c: u32| m.cell_surfaces(c).iter().any(|&(s, _)| s == shape);
        let c = path
            .iter()
            .rev()
            .map(|s| s.cell)
            .find(|&c| uses(c))
            .or(path.last().map(|s| s.cell))?;
        Some(MarkRequest {
            cell: m.cell_id(c),
            surfaces: vec![id],
            preview: None,
            at: Some(p.point.map(f64::to_bits)),
        })
    }
}

/// A cell drawn before it is applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    /// Cell `id` of the input (or a new one) with this geometry
    /// ([`Source::cell_alone`]).
    Cell { id: i64, geometry: String },
    /// Cell `cell` of an input of its own (a new surface's side).
    Input { text: String, cell: i64 },
}

/// A piece of a marked surface's trace on the plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seg {
    pub a: Vec3,
    pub b: Vec3,
    pub surface: i64,
}

/// What the grid was found for.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    plane: PlaneView,
    model: usize,
    shapes: Vec<(u32, i64)>,
    preview: Option<Preview>,
}

/// The cell as edited, compiled alone, and its index there.
type Alone = Arc<(Model, u32)>;

/// What the grid found: its key, the trace, the cell as edited.
type Found = (Key, Vec<Seg>, Option<Alone>);

/// What the surfaces of the 3D view were written for: the model, the
/// surfaces, the point inspected.
type Key3d = (usize, Vec<(u32, i64)>, Option<[u64; 3]>);

/// What a shell was traced for: the camera, the mini model, the frame.
#[derive(Clone, Debug, PartialEq)]
struct ShellKey {
    camera: Camera,
    alone: usize,
    frame: Affine,
}

#[derive(Default)]
pub struct Marks {
    request: Option<MarkRequest>,
    unit: u32,
    /// The compiled surfaces marked, with their MCNP numbers.
    shapes: Vec<(u32, i64)>,
    key: Option<Key>,
    task: Option<Latest<Found>>,
    /// The trace found last for the plane shown.
    pub segments: Vec<Seg>,
    /// The cell as edited (of the preview asked for), once compiled.
    alone: Option<Alone>,
    /// The surfaces of the 3D view ([`surface_words`]) and the frame of
    /// the instance they are drawn for.
    three: Option<(Key3d, Vec<f32>, Option<Affine>)>,
    shell_key: Option<ShellKey>,
    shell_task: Option<Latest<(ShellKey, Option<Arc<Shell>>)>>,
    shell: Option<Arc<Shell>>,
}

impl Marks {
    pub fn request(&self) -> Option<&MarkRequest> {
        self.request.as_ref()
    }

    /// Mark what `req` asks for in model `m` (nothing with `None`).
    pub fn set(&mut self, m: &Model, req: Option<MarkRequest>) {
        if req == self.request {
            return;
        }
        self.segments.clear();
        self.key = None;
        self.shapes.clear();
        self.unit = 0;
        self.alone = None;
        self.three = None;
        self.shell_key = None;
        self.shell = None;
        if let Some(r) = &req {
            if let Some(c) = m.cell_index(r.cell) {
                self.unit = m.cell(c).unit;
                self.shapes = shapes_of(m, c, &r.surfaces);
            }
        }
        self.request = req;
    }

    /// Find the trace again when the plane or the model changed, and take
    /// what was found.
    pub fn update(
        &mut self,
        m: &Arc<Model>,
        source: Option<&Arc<Source>>,
        plane: PlaneView,
        ctx: &egui::Context,
    ) {
        let preview = self.request.as_ref().and_then(|r| r.preview.clone());
        if self.request.is_none() || (self.shapes.is_empty() && preview.is_none()) {
            self.segments.clear();
            self.alone = None;
            return;
        }
        let key = Key {
            plane,
            model: Arc::as_ptr(m) as usize,
            shapes: self.shapes.clone(),
            preview,
        };
        let task = self
            .task
            .get_or_insert_with(|| Latest::new("marks", Some(ctx.clone())));
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key.clone());
            let (m, unit, source) = (m.clone(), self.unit, source.cloned());
            task.send(move || {
                let alone = match (&key.preview, &source) {
                    (Some(p), Some(s)) => alone(s, p).map(Arc::new),
                    _ => None,
                };
                let edited = alone.as_ref().map(|a| (&a.0, a.1));
                let segs = trace(&m, &key.plane, unit, &key.shapes, edited);
                (key, segs, alone)
            });
        }
        if let Some((k, segs, a)) = task.latest() {
            if self
                .key
                .as_ref()
                .is_some_and(|now| now.shapes == k.shapes && now.preview == k.preview)
            {
                self.segments = segs;
                if k.preview.is_none() || a.is_some() {
                    self.alone = a;
                }
            }
        }
    }

    /// The shell of the cell as edited last traced for the 3D view.
    pub fn shell(&self) -> Option<&Arc<Shell>> {
        self.shell.as_ref()
    }

    /// What the 3D view draws, seen by `camera` (cut away or not): the
    /// surfaces marked ([`surface_words`], empty for none) and the shell
    /// of the cell as edited (once traced for this camera, off the UI
    /// thread). For a cell in a universe, both are of the instance at
    /// `focus` (the point inspected), and nothing without one.
    pub fn for_3d(
        &mut self,
        m: &Arc<Model>,
        focus: Option<Vec3>,
        camera: &Camera,
        ctx: &egui::Context,
    ) -> (Vec<f32>, Option<Arc<Shell>>) {
        let Some(req) = &self.request else {
            return (Vec::new(), None);
        };
        let focus = req.at.map(|b| b.map(f64::from_bits)).or(focus);
        let key: Key3d = (
            Arc::as_ptr(m) as usize,
            self.shapes.clone(),
            (self.unit != 0)
                .then(|| focus.map(|p| p.map(f64::to_bits)))
                .flatten(),
        );
        if self.three.as_ref().is_none_or(|t| t.0 != key) {
            let frame = frame_of(m, self.unit, focus);
            let words = match frame {
                Some(g) if !self.shapes.is_empty() => {
                    let moved = (self.unit != 0).then_some(&g);
                    let records: Vec<(u32, Vec<f64>)> = self
                        .shapes
                        .iter()
                        .filter_map(|&(s, _)| m.shape_record(s, moved))
                        .collect();
                    let bounds = moved.and_then(|g| instance_box(m, req.cell, self.unit, g));
                    surface_words(&records, bounds.as_ref())
                }
                _ => Vec::new(),
            };
            self.three = Some((key, words, frame));
        }
        let (words, frame) = match &self.three {
            Some((_, w, f)) => (w.clone(), *f),
            None => (Vec::new(), None),
        };
        let (Some(alone), Some(frame)) = (self.alone.clone(), frame) else {
            return (words, None);
        };
        let mut pose = camera.clone();
        pose.cut = llevant::Cutaway::none();
        let key = ShellKey {
            camera: pose,
            alone: Arc::as_ptr(&alone) as usize,
            frame,
        };
        let task = self
            .shell_task
            .get_or_insert_with(|| Latest::new("shell", Some(ctx.clone())));
        if self.shell_key.as_ref() != Some(&key) {
            self.shell_key = Some(key.clone());
            task.send(move || {
                let shell = Shell::trace(&alone.0, alone.1, &key.camera, &key.frame).map(Arc::new);
                (key, shell)
            });
        }
        if let Some((k, s)) = task.latest() {
            if self
                .shell_key
                .as_ref()
                .is_some_and(|now| now.alone == k.alone && now.frame == k.frame)
            {
                self.shell = s;
            }
        }
        (words, self.shell.clone())
    }
}

/// The map from the problem's frame to that of universe unit `unit`, at
/// the instance holding `focus` (none for a universe without a point).
fn frame_of(m: &Model, unit: u32, focus: Option<Vec3>) -> Option<Affine> {
    if unit == 0 {
        return Some(Affine::IDENTITY);
    }
    m.unit_frame(focus?, unit).map(|(g, _)| g)
}

/// The box (problem's frame) of the instance `g` maps into: that of cell
/// `id` in it, or of its universe's region, padded; `None` if unbounded.
fn instance_box(m: &Model, id: i64, unit: u32, g: &Affine) -> Option<llevant::Aabb> {
    let finite = |b: &llevant::Aabb| b.lo.iter().chain(&b.hi).all(|x| x.is_finite());
    let own = m.cell_index(id).map(|c| m.cell(c).bbox).filter(finite);
    let b = own.or_else(|| Some(m.units().get(unit as usize)?.roi).filter(finite))?;
    let back = g.inverse();
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for k in 0..8 {
        let c = [
            if k & 1 == 0 { b.lo[0] } else { b.hi[0] },
            if k & 2 == 0 { b.lo[1] } else { b.hi[1] },
            if k & 4 == 0 { b.lo[2] } else { b.hi[2] },
        ];
        let p = back.apply(c);
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    let pad = 0.02 * (0..3).map(|i| hi[i] - lo[i]).fold(0.0, f64::max) + 1e-3;
    Some(llevant::Aabb {
        lo: lo.map(|x| x - pad),
        hi: hi.map(|x| x + pad),
    })
}

/// The compiled surfaces numbered `ids` as cell `c` sees them: those of
/// its program, and for a number it does not use, those of the cells of
/// its universe that do.
fn shapes_of(m: &Model, c: u32, ids: &[i64]) -> Vec<(u32, i64)> {
    let id_of = |s: u32| m.surface_id(s).map(|x| x.id);
    let mut out: Vec<(u32, i64)> = Vec::new();
    let push = |s: u32, out: &mut Vec<(u32, i64)>| {
        if let Some(id) = id_of(s).filter(|id| ids.contains(id)) {
            if !out.iter().any(|e| e.0 == s) {
                out.push((s, id));
            }
        }
    };
    for (s, _) in m.cell_surfaces(c) {
        push(s, &mut out);
    }
    let missing: Vec<i64> = ids
        .iter()
        .copied()
        .filter(|id| !out.iter().any(|e| e.1 == *id))
        .collect();
    if !missing.is_empty() {
        let unit = &m.units()[m.cell(c).unit as usize];
        for &other in unit.cells.iter().take(SEARCHED) {
            for (s, _) in m.cell_surfaces(other) {
                if id_of(s).is_some_and(|id| missing.contains(&id)) {
                    push(s, &mut out);
                }
            }
        }
    }
    out
}

/// The cell `preview` draws, compiled alone, and its index there (`None`
/// when it cannot be: a `LIKE` cell, something the input has not got).
fn alone(source: &Source, preview: &Preview) -> Option<(Model, u32)> {
    let (text, id) = match preview {
        Preview::Cell { id, geometry } => (source.cell_alone(*id, geometry).ok()?, *id),
        Preview::Input { text, cell } => (text.clone(), *cell),
    };
    let m = Model::from_text(&text, &llevant::Options::default());
    let c = m.cell_index(id)?;
    Some((m, c))
}

/// The trace of `shapes` on `plane`, seen from universe unit `unit`, and
/// the edge of where cell `edited.1` of model `edited.0` is ([`EDITED`]).
fn trace(
    m: &Model,
    plane: &PlaneView,
    unit: u32,
    shapes: &[(u32, i64)],
    edited: Option<(&Model, u32)>,
) -> Vec<Seg> {
    use rayon::prelude::*;
    let nx = ACROSS;
    let ny = ((nx as f64 * plane.height / plane.width).round() as usize).clamp(2, 2 * ACROSS);
    let (w, h) = (plane.width, plane.height);
    let at = |i: usize, j: usize| -> [f64; 2] {
        [
            plane.centre[0] - 0.5 * w + w * i as f64 / (nx - 1) as f64,
            plane.centre[1] + 0.5 * h - h * j as f64 / (ny - 1) as f64,
        ]
    };
    // The side of mark `k` at `q` (the universe's frame): a surface's
    // positive side, or inside the cell as edited.
    let side = |k: usize, q: Vec3| match shapes.get(k) {
        Some(&(s, _)) => m.surface_positive(s, q).unwrap_or(false),
        None => edited.is_some_and(|(a, c)| a.cell_holds(c, q)),
    };
    let marks = shapes.len() + usize::from(edited.is_some());
    // Per node: its instance, its point in the universe's frame and the
    // side of each mark.
    let nodes: Vec<Option<(u64, Vec3, Vec<bool>)>> = (0..nx * ny)
        .into_par_iter()
        .map(|k| {
            let p = plane.frame.point(at(k % nx, k / nx));
            let (q, key) = m.point_in_unit(p, unit)?;
            Some((key, q, (0..marks).map(|i| side(i, q)).collect()))
        })
        .collect();
    let ids: Vec<i64> = shapes
        .iter()
        .map(|s| s.1)
        .chain(edited.map(|_| EDITED))
        .collect();
    (0..ny - 1)
        .into_par_iter()
        .flat_map_iter(|j| {
            let mut out = Vec::new();
            for i in 0..nx - 1 {
                let corners = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)];
                let n: Vec<&(u64, Vec3, Vec<bool>)> = corners
                    .iter()
                    .filter_map(|&(a, b)| nodes[b * nx + a].as_ref())
                    .collect();
                if n.len() < 4 || n.iter().any(|x| x.0 != n[0].0) {
                    continue;
                }
                for (k, &id) in ids.iter().enumerate() {
                    let mut cross: Vec<Vec3> = Vec::new();
                    for e in 0..4 {
                        let (a, b) = (e, (e + 1) % 4);
                        if n[a].2[k] == n[b].2[k] {
                            continue;
                        }
                        // Where the side changes along the edge: within one
                        // instance the universe's frame is the plane's moved,
                        // so the point there moves in a line from end to end.
                        let t = crossing(n[a].1, n[b].1, n[a].2[k], |q| side(k, q));
                        let (pa, pb) = (
                            at(corners[a].0, corners[a].1),
                            at(corners[b].0, corners[b].1),
                        );
                        cross.push(
                            plane
                                .frame
                                .point([pa[0] + t * (pb[0] - pa[0]), pa[1] + t * (pb[1] - pa[1])]),
                        );
                    }
                    for pair in cross.as_chunks::<2>().0 {
                        out.push(Seg {
                            a: pair[0],
                            b: pair[1],
                            surface: id,
                        });
                    }
                }
            }
            out
        })
        .collect()
}

/// Halvings of a grid edge to place a crossing: 1/256 of a grid square,
/// below a screen point.
const HALVINGS: usize = 8;

/// Where along `a`→`b` (0 to 1) `side` changes from `at_a`.
fn crossing(a: Vec3, b: Vec3, at_a: bool, side: impl Fn(Vec3) -> bool) -> f64 {
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..HALVINGS {
        let t = 0.5 * (lo + hi);
        let q = std::array::from_fn(|i| a[i] + t * (b[i] - a[i]));
        if side(q) == at_a {
            lo = t;
        } else {
            hi = t;
        }
    }
    0.5 * (lo + hi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slice::Frame;

    const TEXT: &str = "marks
1 0 -1 imp:n=1
2 0 1 -9 imp:n=1
9 0 9 imp:n=0

1 so 5
9 so 50

";

    #[test]
    fn a_sphere_traced_on_a_plane() {
        let m = Model::from_text(TEXT, &llevant::Options::default());
        let plane = PlaneView {
            frame: Frame::axis(crate::state::AxisName::Z, 0.0),
            centre: [0.0, 0.0],
            width: 20.0,
            height: 20.0,
        };
        let c = m.cell_index(2).unwrap();
        let shapes = shapes_of(&m, c, &[1]);
        assert_eq!(shapes.len(), 1);
        let segs = trace(&m, &plane, 0, &shapes, None);
        assert!(segs.len() > 100, "{}", segs.len());
        for s in &segs {
            let r = (s.a[0] * s.a[0] + s.a[1] * s.a[1]).sqrt();
            assert!((r - 5.0).abs() < 2e-3, "{r}");
            assert_eq!(s.surface, 1);
        }
        // A number no cell of the universe uses: nothing.
        assert!(shapes_of(&m, c, &[77]).is_empty());

        // Cell 2 as edited, "-9 #1" moved by a TRCL: its edge is surface
        // 1 moved (and 9, off the plane shown).
        let source = Source::from_str(&TEXT.replace("2 0 1 -9", "2 0 1 -9 trcl=(3 0 0)"));
        let cell = |geometry: &str| Preview::Cell {
            id: 2,
            geometry: geometry.into(),
        };
        let (a, ci) = alone(&source, &cell("-9 #1")).unwrap();
        let segs = trace(&m, &plane, 0, &shapes, Some((&a, ci)));
        let edge: Vec<&Seg> = segs.iter().filter(|s| s.surface == EDITED).collect();
        assert!(edge.len() > 100);
        for s in edge {
            let r = ((s.a[0] - 3.0).powi(2) + s.a[1] * s.a[1]).sqrt();
            assert!((r - 5.0).abs() < 2e-3, "{r}");
        }
        assert!(alone(&source, &cell("-9 #77")).is_none());
        // A new cell, not in the input.
        let new = Preview::Cell {
            id: 40,
            geometry: "-1".into(),
        };
        assert!(alone(&source, &new).is_some());
    }
}

#[cfg(test)]
mod timing {
    use super::*;
    use crate::slice::Frame;

    /// The time of a trace on the reference model: a cell of level 0 and
    /// one in a universe, each with its surfaces marked, and with itself
    /// as the cell as edited.
    #[test]
    #[ignore = "needs LLEVANT_BIG_MODEL"]
    fn trace_time_on_the_big_model() {
        let Ok(path) = std::env::var("LLEVANT_BIG_MODEL") else {
            return;
        };
        let text = std::fs::read_to_string(&path).unwrap();
        let m = Model::from_text(&text, &llevant::Options::default());
        let source = Source::from_str(&text);
        let finite = |b: &llevant::Aabb| b.lo.iter().chain(&b.hi).all(|x| x.is_finite());
        let many = |c: u32| m.cell_surfaces(c).len() >= 6;
        let all = 0..m.cells().len() as u32;
        // A cell of level 0, seen through its own box; and a cell of a
        // universe filling a cell of level 0, seen through that one's box.
        let top = all
            .clone()
            .find(|&c| m.cell(c).unit == 0 && finite(&m.cell(c).bbox) && many(c))
            .unwrap();
        let (holder, inner) = all
            .filter(|&h| m.cell(h).unit == 0 && finite(&m.cell(h).bbox))
            .find_map(|h| match &m.cell(h).fill {
                Some(llevant::Fill::Universe(t)) => m.units()[t.unit() as usize]
                    .cells
                    .iter()
                    .copied()
                    .find(|&c| many(c))
                    .map(|c| (h, c)),
                _ => None,
            })
            .unwrap();
        for (c, b) in [(top, m.cell(top).bbox), (inner, m.cell(holder).bbox)] {
            let cell = m.cell(c);
            let ids: Vec<i64> = m
                .cell_surfaces(c)
                .into_iter()
                .filter_map(|(s, _)| m.surface_id(s).map(|x| x.id))
                .collect();
            let shapes = shapes_of(&m, c, &ids);
            let mid = |i: usize| 0.5 * (b.lo[i] + b.hi[i]);
            let size = (0..2).map(|i| b.hi[i] - b.lo[i]).fold(0.0, f64::max);
            let plane = PlaneView {
                frame: Frame::axis(crate::state::AxisName::Z, mid(2)),
                centre: [mid(0), mid(1)],
                width: size * 1.2,
                height: size * 0.7,
            };
            let geometry = source
                .cell_fields(cell.id)
                .map(|f| f.geometry.clone())
                .unwrap_or_default();
            let alone = alone(
                &source,
                &Preview::Cell {
                    id: cell.id,
                    geometry,
                },
            );
            let edited = alone.as_ref().map(|(a, i)| (a, *i));
            for _ in 0..2 {
                trace(&m, &plane, cell.unit, &shapes, edited);
            }
            let n = 5;
            let t = std::time::Instant::now();
            let mut segs = 0;
            for _ in 0..n {
                segs = trace(&m, &plane, cell.unit, &shapes, edited).len();
            }
            eprintln!(
                "cell {} (unit {}): {} surfaces, edited {}: {:.1} ms a trace, {segs} segments",
                cell.id,
                cell.unit,
                shapes.len(),
                edited.is_some(),
                t.elapsed().as_secs_f64() * 1e3 / n as f64
            );
        }
    }
}
