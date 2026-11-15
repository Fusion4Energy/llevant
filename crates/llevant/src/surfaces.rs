//! Questions about surfaces, for inspectors and labels: which surfaces a
//! cell uses, which surface is nearest a point, and which surfaces pass
//! through a point. All on demand, outside the classify / trace / first-hit
//! paths.

use crate::classify::Label;
use crate::error::Result;
use crate::expr;
use crate::surface::Side;
use crate::trace::{check_lines, Line, NO_SURFACE};
use crate::vec3::{add, norm, scale, Vec3};
use crate::Model;

/// The surface crossed nearest a point, from [`Model::nearest_surface`].
#[derive(Clone, Debug, PartialEq)]
pub struct NearSurface {
    /// Index of the compiled surface ([`Model::surface_id`] gives its MCNP
    /// number).
    pub shape: u32,
    /// Where it was crossed, and how far that is from the point (cm).
    pub point: Vec3,
    pub distance: f64,
    /// The unit direction of the line that crossed it.
    pub dir: Vec3,
}

impl Model {
    /// The compiled surfaces of cell `cell`'s program, each once (in order of
    /// first use), with the sense the program asks for (`true`: positive
    /// side). Complements (`#n`) are expanded into the surfaces they bring,
    /// with their senses as evaluated; a surface used with both senses
    /// appears twice.
    pub fn cell_surfaces(&self, cell: u32) -> Vec<(u32, bool)> {
        let mut out: Vec<(u32, bool)> = Vec::new();
        for leaf in expr::leaves(self.code(cell)) {
            if !out.contains(&leaf) {
                out.push(leaf);
            }
        }
        out
    }

    /// [`Model::cell_surfaces`] as the program has them: one per leaf, so
    /// with repeats, and without allocating (for indexes over every cell).
    pub fn cell_surface_leaves(&self, cell: u32) -> impl Iterator<Item = (u32, bool)> + '_ {
        expr::leaves(self.code(cell))
    }

    /// The surface crossed nearest `p` along any of `dirs` (each line runs
    /// `reach` cm both ways from `p`), as a slice at universe level `level`
    /// shows it (`None`: every level). `None` if no line crosses one.
    pub fn nearest_surface(
        &self,
        p: Vec3,
        dirs: &[Vec3],
        reach: f64,
        level: Option<usize>,
    ) -> Result<Option<NearSurface>> {
        let lines: Vec<Line> = dirs
            .iter()
            .filter(|d| norm(**d) > 0.0)
            .map(|&d| {
                let d = scale(d, 1.0 / norm(d));
                Line::new(add(p, scale(d, -reach)), d, 0.0, 2.0 * reach)
            })
            .collect();
        check_lines(&lines)?;
        let tr = self.trace_few(&lines, level);
        let mut best: Option<NearSurface> = None;
        for (i, line) in lines.iter().enumerate() {
            for k in tr.segments(i) {
                // Ends inside the line: the end of one segment is the start of
                // the next, so the outgoing ends cover every crossing.
                let (t, s) = (tr.t1[k], tr.surface_out[k]);
                if s == NO_SURFACE {
                    continue;
                }
                let distance = (t - reach).abs();
                if distance <= reach && best.as_ref().is_none_or(|b| distance < b.distance) {
                    best = Some(NearSurface {
                        shape: s,
                        point: line.at(t),
                        distance,
                        dir: line.dir,
                    });
                }
            }
        }
        Ok(best.map(|mut b| {
            b.shape = self.bounding_surface(b.shape, b.point, b.dir);
            b
        }))
    }

    /// The surface to name for a crossing of `shape` at `p` along `dir`:
    /// `shape` if it bounds a cell there, else the first (by MCNP number) of
    /// the ones that do. Tracing names the lowest-indexed of the surfaces
    /// through a crossing that its candidate cells use, which can be an
    /// infinite facet plane of a body elsewhere that coincides there (the
    /// tops of two boxes side by side); this names the right box.
    pub fn bounding_surface(&self, shape: u32, p: Vec3, dir: Vec3) -> u32 {
        let across = self.surfaces_across(p, dir);
        match across.first() {
            Some(&first) if !across.contains(&shape) => first,
            _ => shape,
        }
    }

    /// The unit normal of compiled surface `shape` at `p`: the gradient of
    /// its function, pointing to its positive side. `None` where it has
    /// none (a cone's apex) or for no surface.
    pub fn surface_normal(&self, shape: u32, p: Vec3) -> Option<Vec3> {
        let g = self.shapes.get(shape as usize)?.gradient(p);
        let n = norm(g);
        (n > 0.0 && n.is_finite()).then(|| scale(g, 1.0 / n))
    }

    /// `p` in the frame of universe unit `unit` ([`Model::units`]), where
    /// an instance of that universe holds `p`: the point its cells (and
    /// their compiled surfaces) see there, and a key equal for the points of
    /// one instance. Unit 0 is the problem's own frame. `None` where no
    /// instance of it holds `p` (or a gap or overlap above it does).
    pub fn point_in_unit(&self, p: Vec3, unit: u32) -> Option<(Vec3, u64)> {
        use std::hash::{Hash, Hasher};
        if unit == 0 {
            return Some((p, 0));
        }
        let label = self.classify_point(p);
        let frames = self.path_frames(&label, p);
        let key = |k: usize| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            label.path[..k].hash(&mut h);
            h.finish()
        };
        for (k, step) in label.path.iter().enumerate() {
            if self.cells[step.cell as usize].unit == unit {
                return frames.get(k).map(|&q| (q, key(k)));
            }
        }
        // On a surface of that universe: the cells beside it at the level
        // below the path's end.
        let k = label.path.len();
        if label
            .involved
            .iter()
            .any(|&c| self.cells[c as usize].unit == unit)
        {
            return frames.get(k).map(|&q| (q, key(k)));
        }
        None
    }

    /// [`Model::point_in_unit`] as a map: from the problem's frame to that
    /// of universe unit `unit` where an instance of it holds `p` (the same
    /// map for every point of that instance), and the instance's key.
    pub fn unit_frame(&self, p: Vec3, unit: u32) -> Option<(crate::Affine, u64)> {
        use std::hash::{Hash, Hasher};
        if unit == 0 {
            return Some((crate::Affine::IDENTITY, 0));
        }
        let label = self.classify_point(p);
        let maps = self.path_maps(&label);
        let key = |k: usize| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            label.path[..k].hash(&mut h);
            h.finish()
        };
        let k = label
            .path
            .iter()
            .position(|s| self.cells[s.cell as usize].unit == unit)
            .or_else(|| {
                let k = label.path.len();
                label
                    .involved
                    .iter()
                    .any(|&c| self.cells[c as usize].unit == unit)
                    .then_some(k)
            })?;
        maps.get(k).map(|g| (*g, key(k)))
    }

    /// Compiled surface `shape` as [`crate::export`] records it (its kind,
    /// `SHAPE_*`, and its numbers), written in the frame `frame` maps from
    /// (the problem's, for a map of [`Model::unit_frame`]); in its own
    /// universe's frame without one. `None` for no surface.
    pub fn shape_record(
        &self,
        shape: u32,
        frame: Option<&crate::Affine>,
    ) -> Option<(u32, Vec<f64>)> {
        let s = self.shapes.get(shape as usize)?;
        Some(match frame {
            Some(t) => crate::export::shape_data(&s.to_parent(t)),
            None => crate::export::shape_data(s),
        })
    }

    /// Whether `q` (in the frame of the surface's universe, as
    /// [`Model::point_in_unit`] gives it) is on the positive side of
    /// compiled surface `shape`: `None` for no surface.
    pub fn surface_positive(&self, shape: u32, q: Vec3) -> Option<bool> {
        let s = self.shapes.get(shape as usize)?;
        Some(s.side(q, 0.0) != Side::Neg)
    }

    /// Whether `q` (in the frame of the cell's universe, as
    /// [`Model::point_in_unit`] gives it) is in cell `cell` by its own
    /// program, other cells aside (on one of its surfaces: in it).
    pub fn cell_holds(&self, cell: u32, q: Vec3) -> bool {
        self.eval_code(self.code(cell), q, None) != expr::Tri::False
    }

    /// Every compiled surface that passes within the tolerance of `p` and
    /// bounds a cell on either side of it along `dir` (the cells of the paths
    /// a little before and after `p`, at every level): the surface a crossing
    /// names and the ones coincident with it. Sorted by MCNP number.
    pub fn surfaces_across(&self, p: Vec3, dir: Vec3) -> Vec<u32> {
        let n = norm(dir);
        if n == 0.0 || !p.iter().all(|x| x.is_finite()) {
            return Vec::new();
        }
        let d = scale(dir, 1.0 / n);
        // Far enough to leave the on-surface band, near enough to stay in
        // the cells beside it.
        let step = 4.0 * self.tolerance + 1e-12 * norm(p);
        let mut out: Vec<u32> = Vec::new();
        for side in [-step, step] {
            let label = self.classify_point(add(p, scale(d, side)));
            self.surfaces_on_path(&label, p, &mut out);
        }
        out.sort_by_key(|&s| {
            let id = self.surface_ids[s as usize];
            (id.id, id.facet, s)
        });
        out.dedup();
        out
    }

    /// Push the surfaces of `label`'s path cells (and of the cells involved
    /// at its last level) that pass within the tolerance of `p`.
    fn surfaces_on_path(&self, label: &Label, p: Vec3, out: &mut Vec<u32>) {
        let frames = self.path_frames(label, p);
        let tol = self.tolerance;
        let mut check = |cell: u32, q: Vec3| {
            let code = self.code(cell);
            for (s, _) in expr::leaves(code) {
                if !out.contains(&s)
                    && self.shapes[s as usize].side(q, tol) == Side::On
                    && self.bounds_at(code, s, q)
                {
                    out.push(s);
                }
            }
        };
        for (level, step) in label.path.iter().enumerate() {
            if let Some(&q) = frames.get(level) {
                check(step.cell, q);
            }
        }
        // Overlap, on-surface and unsupported points name the cells at the
        // level below the path's end.
        if let Some(&q) = frames.get(label.path.len()) {
            for &c in &label.involved {
                check(c, q);
            }
        }
    }

    /// Whether surface `s` (through `q`) bounds program `code` there: the
    /// program's value at `q` depends on the side of `s` taken. Other
    /// surfaces through `q` are undecided. This drops surfaces that pass
    /// through `q` without bounding the cell there (an infinite facet plane
    /// of a body elsewhere, a moved copy's).
    fn bounds_at(&self, code: &[u32], s: u32, q: Vec3) -> bool {
        let tol = self.tolerance;
        let value = |forced_pos: bool| {
            expr::eval(code, q, None, &mut |leaf, want_pos| {
                let side = if leaf == s {
                    if forced_pos {
                        Side::Pos
                    } else {
                        Side::Neg
                    }
                } else {
                    self.shapes[leaf as usize].side(q, tol)
                };
                match side {
                    Side::On => expr::Tri::Unknown,
                    Side::Pos if want_pos => expr::Tri::True,
                    Side::Neg if !want_pos => expr::Tri::True,
                    _ => expr::Tri::False,
                }
            })
        };
        value(true) != value(false)
    }
}
