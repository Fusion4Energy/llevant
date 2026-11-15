//! Views of a model: 2D slices (one [`Model::trace`] per image row, so
//! boundaries are exact along rows) and 3D renders (one
//! [`Model::first_hits`] ray per pixel). Colours and image files are left to
//! the caller.

use crate::bbox::Aabb;
use crate::error::Result;
use crate::hit::{Hits, Visibility, NO_HIT};
use crate::trace::{Line, Tracing, NO_SURFACE};
use crate::vec3::{add, cross, dot, norm, normalize, scale, sub, Vec3};
use crate::{Model, Scope};

/// A coordinate axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    /// 0, 1 or 2.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// A rectangle of a plane, sampled by `nx × ny` pixels: points
/// `origin + a·u + b·v` for `a` in `[0, width]`, `b` in `[0, height]`.
/// `u` and `v` are orthogonal unit vectors. Pixel rows run along `u`; row 0
/// is at the top (largest `b`).
#[derive(Clone, Debug, PartialEq)]
pub struct Slice {
    pub origin: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub width: f64,
    pub height: f64,
    pub nx: usize,
    pub ny: usize,
}

impl Slice {
    /// The plane `x = at`, `y = at` or `z = at` across `region`, `nx`
    /// pixels wide and as many rows as keep pixels square. The image axes
    /// are as in MCNP plots: (y, z) for `x = at`, (x, z) for `y = at` and
    /// (x, y) for `z = at`.
    pub fn axis(axis: Axis, at: f64, region: &Aabb, nx: usize) -> Slice {
        let (a, b) = match axis {
            Axis::X => (1, 2),
            Axis::Y => (0, 2),
            Axis::Z => (0, 1),
        };
        let axis = axis.index();
        let mut origin = [0.0; 3];
        origin[axis] = at;
        origin[a] = region.lo[a];
        origin[b] = region.lo[b];
        let mut u = [0.0; 3];
        u[a] = 1.0;
        let mut v = [0.0; 3];
        v[b] = 1.0;
        let width = region.hi[a] - region.lo[a];
        let height = region.hi[b] - region.lo[b];
        let ny = ((nx as f64 * height / width).round() as usize).max(1);
        Slice {
            origin,
            u,
            v,
            width,
            height,
            nx,
            ny,
        }
    }

    /// A `width × height` rectangle of any plane, centred on `centre`, its
    /// rows along `u` and its columns up along `v` (made orthogonal to `u`
    /// and unit), `nx` pixels wide and as many rows as keep pixels square.
    /// `None` if `u` and `v` do not span a plane.
    pub fn plane(
        centre: Vec3,
        u: Vec3,
        v: Vec3,
        width: f64,
        height: f64,
        nx: usize,
    ) -> Option<Slice> {
        let u = normalized(u)?;
        let v = normalized(sub(v, scale(u, dot(v, u))))?;
        let origin = sub(centre, add(scale(u, 0.5 * width), scale(v, 0.5 * height)));
        let ny = ((nx as f64 * height / width).round() as usize).max(1);
        Some(Slice {
            origin,
            u,
            v,
            width,
            height,
            nx,
            ny,
        })
    }

    /// The plane through the axis through `point` along `axis`, at angle
    /// `phi` (degrees, right-handed about the axis, from `zero` projected
    /// normal to it): rows along the radius at `phi`, columns up the axis,
    /// centred on `point`. A vertical slice through the axis of a torus,
    /// say. `None` if `zero` is along the axis.
    pub fn radial(
        point: Vec3,
        axis: Vec3,
        zero: Vec3,
        phi: f64,
        width: f64,
        height: f64,
        nx: usize,
    ) -> Option<Slice> {
        let a = normalized(axis)?;
        let e0 = normalized(sub(zero, scale(a, dot(zero, a))))?;
        let e90 = cross(a, e0);
        let (s, c) = phi.to_radians().sin_cos();
        let radius = add(scale(e0, c), scale(e90, s));
        Slice::plane(point, radius, a, width, height, nx)
    }

    /// One line per pixel row, from the top, through the centres of its
    /// pixels: `t` from 0 to `width`.
    pub fn rows(&self) -> Vec<Line> {
        (0..self.ny)
            .map(|j| {
                let b = self.height * (1.0 - (j as f64 + 0.5) / self.ny as f64);
                Line {
                    origin: add(self.origin, scale(self.v, b)),
                    dir: self.u,
                    t_min: 0.0,
                    t_max: self.width,
                }
            })
            .collect()
    }

    /// The point at the centre of pixel `(i, j)` (column `i`, row `j` from
    /// the top).
    pub fn pixel_centre(&self, i: usize, j: usize) -> Vec3 {
        let a = self.width * (i as f64 + 0.5) / self.nx as f64;
        let b = self.height * (1.0 - (j as f64 + 0.5) / self.ny as f64);
        add(self.origin, add(scale(self.u, a), scale(self.v, b)))
    }

    /// Label id (into `tr.labels`) of the segment that holds each pixel
    /// centre, row by row from the top: `nx · ny` ids. `tr` is the slice's
    /// tracing ([`Model::slice`]).
    ///
    /// # Panics
    ///
    /// If `tr` does not have one line per row.
    pub fn pixels(&self, tr: &Tracing) -> Vec<u32> {
        assert_eq!(tr.line_count(), self.ny, "one traced line per row");
        let mut out = Vec::with_capacity(self.nx * self.ny);
        for j in 0..self.ny {
            let mut k = tr.segments(j).start;
            let end = tr.segments(j).end;
            for i in 0..self.nx {
                let t = self.width * (i as f64 + 0.5) / self.nx as f64;
                // Segments are ordered and cover [0, width]; a centre on a
                // boundary goes to the segment that starts there.
                while k + 1 < end && tr.t1[k] <= t {
                    k += 1;
                }
                out.push(tr.ids[k]);
            }
        }
        out
    }
}

/// A plane that cuts away the half-space its normal points into:
/// points `p` with `(p - point)·normal > 0` are removed from a render.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clip {
    pub point: Vec3,
    pub normal: Vec3,
}

/// Space cut away from a render: the union of `regions`, each the
/// intersection of the half-spaces its [`Clip`]s remove. One region of one
/// plane is a clip plane; others make wedges about an axis
/// ([`Cutaway::wedge`]), boxes kept ([`Cutaway::keep_box`]) or removed
/// ([`Cutaway::remove_box`]). A clip with a zero normal removes nothing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cutaway {
    pub regions: Vec<Vec<Clip>>,
}

/// Where a box lies against a [`Cutaway`] ([`Cutaway::place_box`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxCut {
    /// Wholly cut away (inside one region).
    Removed,
    /// Wholly kept (outside every region).
    Kept,
    /// Across a cut, or not known.
    Unknown,
}

/// Part of a ray a [`Cutaway`] keeps: `t` from `t0` to `t1`, and the normal
/// of the cut face the ray leaves the removed space through at `t0` (facing
/// the ray), `None` where the part starts at the ray's own start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Kept {
    pub t0: f64,
    pub t1: f64,
    pub normal: Option<Vec3>,
}

impl Cutaway {
    /// Nothing cut away.
    pub fn none() -> Cutaway {
        Cutaway::default()
    }

    /// The half-space of one plane.
    pub fn plane(c: Clip) -> Cutaway {
        Cutaway {
            regions: vec![vec![c]],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.regions.iter().all(|r| r.is_empty())
    }

    /// Both cut away.
    pub fn union(mut self, other: Cutaway) -> Cutaway {
        self.regions.extend(other.regions);
        self
    }

    /// The wedge about the axis through `point` along `axis` from angle
    /// `from` to angle `to` (degrees, turning about `axis` by the right-hand
    /// rule, from the direction `zero` projected normal to the axis; `to`
    /// taken past `from`, so 350° to 10° is the 20° across the zero). A
    /// wedge of 360° or more cuts away everything; none, nothing.
    pub fn wedge(point: Vec3, axis: Vec3, zero: Vec3, from: f64, to: f64) -> Cutaway {
        let a = normalize(axis);
        let u = normalize(sub(zero, scale(a, dot(zero, a))));
        let v = cross(a, u);
        let span = (to - from).rem_euclid(360.0);
        let span = if span == 0.0 && to != from {
            360.0
        } else {
            span
        };
        if span == 0.0 || !u.iter().all(|x| x.is_finite()) {
            return Cutaway::none();
        }
        // The normal of the half-plane at angle φ, towards larger angles.
        let side = |deg: f64| {
            let (s, c) = deg.to_radians().sin_cos();
            add(scale(u, -s), scale(v, c))
        };
        let after = Clip {
            point,
            normal: side(from),
        };
        let before = Clip {
            point,
            normal: scale(side(from + span), -1.0),
        };
        if span >= 360.0 {
            // Everything: the two sides of one plane.
            let n = side(from);
            Cutaway {
                regions: vec![
                    vec![Clip { point, normal: n }],
                    vec![Clip {
                        point,
                        normal: scale(n, -1.0),
                    }],
                ],
            }
        } else if span <= 180.0 {
            Cutaway {
                regions: vec![vec![after, before]],
            }
        } else {
            // More than a half-turn: all but the wedge from `to` to
            // `from`, which is less than one.
            Cutaway {
                regions: vec![vec![after], vec![before]],
            }
        }
    }

    /// Everything outside `b`.
    pub fn keep_box(b: &Aabb) -> Cutaway {
        let mut regions = Vec::new();
        for k in 0..3 {
            let mut n = [0.0; 3];
            n[k] = 1.0;
            regions.push(vec![Clip {
                point: b.hi,
                normal: n,
            }]);
            n[k] = -1.0;
            regions.push(vec![Clip {
                point: b.lo,
                normal: n,
            }]);
        }
        Cutaway { regions }
    }

    /// Everything inside `b`.
    pub fn remove_box(b: &Aabb) -> Cutaway {
        let mut region = Vec::new();
        for k in 0..3 {
            let mut n = [0.0; 3];
            n[k] = -1.0;
            region.push(Clip {
                point: b.hi,
                normal: n,
            });
            n[k] = 1.0;
            region.push(Clip {
                point: b.lo,
                normal: n,
            });
        }
        Cutaway {
            regions: vec![region],
        }
    }

    /// Whether `p` is cut away.
    pub fn removes(&self, p: Vec3) -> bool {
        self.regions.iter().any(|r| {
            !r.is_empty()
                && r.iter().all(|c| {
                    let n = normalized(c.normal);
                    n.is_some_and(|n| dot(sub(p, c.point), n) > 0.0)
                })
        })
    }

    /// Where box `b` lies: wholly in one region cut away, wholly outside
    /// every region, or neither known (across a cut, or in the union of
    /// regions without being in one). On a cut plane counts as kept, as in
    /// [`Cutaway::removes`].
    pub fn place_box(&self, b: &Aabb) -> BoxCut {
        let corners = |n: Vec3, p: Vec3| {
            // The smallest and largest of `(corner - p)·n` over the corners.
            let (mut lo, mut hi) = (0.0, 0.0);
            for k in 0..3 {
                let (a, z) = (n[k] * (b.lo[k] - p[k]), n[k] * (b.hi[k] - p[k]));
                lo += a.min(z);
                hi += a.max(z);
            }
            (lo, hi)
        };
        let mut kept = true;
        for r in &self.regions {
            if r.is_empty() {
                continue;
            }
            let mut inside = true;
            let mut outside = false;
            for c in r {
                let Some(n) = normalized(c.normal) else {
                    inside = false;
                    outside = true;
                    break;
                };
                let (lo, hi) = corners(n, c.point);
                if lo <= 0.0 {
                    inside = false;
                }
                if hi <= 0.0 {
                    outside = true;
                }
            }
            if inside {
                return BoxCut::Removed;
            }
            if !outside {
                kept = false;
            }
        }
        if kept {
            BoxCut::Kept
        } else {
            BoxCut::Unknown
        }
    }

    /// The parts of the ray `origin + t·dir`, `t` in `[t_min, t_max]`, that
    /// are kept, in order.
    pub fn kept(&self, origin: Vec3, dir: Vec3, t_min: f64, t_max: f64) -> Vec<Kept> {
        // Where the ray is in each region, and the plane it leaves it by.
        let mut removed: Vec<(f64, f64, Option<Vec3>)> = Vec::new();
        for r in &self.regions {
            if r.is_empty() {
                continue;
            }
            let (mut a, mut b, mut exit) = (t_min, t_max, None);
            let mut inside = true;
            for c in r {
                let Some(n) = normalized(c.normal) else {
                    inside = false;
                    break;
                };
                let s0 = dot(sub(origin, c.point), n);
                let ds = dot(dir, n);
                if ds == 0.0 {
                    if s0 <= 0.0 {
                        inside = false;
                        break;
                    }
                    continue;
                }
                let t = -s0 / ds;
                if ds > 0.0 {
                    a = a.max(t);
                } else if t < b {
                    b = t;
                    exit = Some(n);
                }
            }
            if inside && a < b {
                removed.push((a, b, exit));
            }
        }
        removed.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut out = Vec::new();
        let (mut t, mut normal) = (t_min, None);
        for (a, b, exit) in removed {
            if b <= t {
                continue;
            }
            if a > t {
                out.push(Kept {
                    t0: t,
                    t1: a,
                    normal,
                });
            }
            t = b;
            normal = exit;
        }
        if t < t_max {
            out.push(Kept {
                t0: t,
                t1: t_max,
                normal,
            });
        }
        out
    }
}

impl From<Clip> for Cutaway {
    fn from(c: Clip) -> Cutaway {
        Cutaway::plane(c)
    }
}

impl From<Option<Clip>> for Cutaway {
    fn from(c: Option<Clip>) -> Cutaway {
        c.map_or_else(Cutaway::none, Cutaway::plane)
    }
}

fn normalized(n: Vec3) -> Option<Vec3> {
    let len = norm(n);
    (len > 0.0 && len.is_finite()).then(|| scale(n, 1.0 / len))
}

/// A camera: `width × height` pixels looking from `eye` at `target`, `up`
/// any vector not along the view direction, rays reaching `reach` cm.
/// A pinhole with `fov_deg` the vertical field of view, or with `ortho` the
/// height of the view (cm) seen straight on, every ray parallel. `cut` is
/// cut away from what it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fov_deg: f64,
    pub width: usize,
    pub height: usize,
    pub reach: f64,
    pub cut: Cutaway,
    pub ortho: Option<f64>,
}

impl Camera {
    /// A camera looking along `dir` that frames all of `region` (fov 30°,
    /// `up` along z unless `dir` is).
    pub fn fit(region: &Aabb, dir: Vec3, width: usize, height: usize) -> Camera {
        let dir = normalize(dir);
        let centre = region.center();
        let radius = 0.5 * norm(sub(region.hi, region.lo));
        let fov_deg: f64 = 30.0;
        let aspect = (width as f64 / height as f64).min(1.0);
        let half = (0.5 * fov_deg).to_radians();
        // The bounding sphere fits the narrower side of the image.
        let dist = radius / (half.tan() * aspect).atan().sin();
        let up = if dir[2].abs() > 0.99 {
            [0.0, 1.0, 0.0]
        } else {
            [0.0, 0.0, 1.0]
        };
        Camera {
            eye: sub(centre, scale(dir, dist)),
            target: centre,
            up,
            fov_deg,
            width,
            height,
            reach: dist + radius,
            cut: Cutaway::none(),
            ortho: None,
        }
    }

    /// Forward, right and up unit vectors of the image.
    pub fn basis(&self) -> [Vec3; 3] {
        let f = normalize(sub(self.target, self.eye));
        let r = normalize(cross(f, self.up));
        let u = cross(r, f);
        [f, r, u]
    }

    /// Pixel `(i, j)`'s centre on the image, from -1 to 1 across its height
    /// (`x` right, `y` up; `x` reaches the aspect ratio).
    fn image_xy(&self, i: usize, j: usize) -> (f64, f64) {
        let aspect = self.width as f64 / self.height as f64;
        let x = (2.0 * (i as f64 + 0.5) / self.width as f64 - 1.0) * aspect;
        let y = 1.0 - 2.0 * (j as f64 + 0.5) / self.height as f64;
        (x, y)
    }

    /// Where the ray through pixel `(i, j)` starts: the eye, or with an
    /// orthographic camera its point of the plane through the eye.
    pub fn origin(&self, i: usize, j: usize) -> Vec3 {
        match self.ortho {
            None => self.eye,
            Some(h) => {
                let [_, r, u] = self.basis();
                let (x, y) = self.image_xy(i, j);
                add(self.eye, add(scale(r, 0.5 * h * x), scale(u, 0.5 * h * y)))
            }
        }
    }

    /// Unit direction of the ray through the centre of pixel `(i, j)`
    /// (column `i`, row `j` from the top), whatever is cut away.
    pub fn direction(&self, i: usize, j: usize) -> Vec3 {
        let [f, r, u] = self.basis();
        if self.ortho.is_some() {
            return f;
        }
        let h = (0.5 * self.fov_deg).to_radians().tan();
        let (x, y) = self.image_xy(i, j);
        normalize(add(f, add(scale(r, x * h), scale(u, y * h))))
    }

    /// The point `t` along the ray of pixel `(i, j)`.
    pub fn point(&self, i: usize, j: usize, t: f64) -> Vec3 {
        add(self.origin(i, j), scale(self.direction(i, j), t))
    }

    /// The parts of pixel `(i, j)`'s ray (from its origin to `reach`) that
    /// `cut` keeps, in order, with the normal of the cut face each starts
    /// on.
    pub fn segments(&self, i: usize, j: usize) -> Vec<(Line, Option<Vec3>)> {
        let (origin, dir) = (self.origin(i, j), self.direction(i, j));
        self.cut
            .kept(origin, dir, 0.0, self.reach)
            .into_iter()
            .map(|k| {
                let line = Line {
                    origin,
                    dir,
                    t_min: k.t0,
                    t_max: k.t1,
                };
                (line, k.normal)
            })
            .collect()
    }

    /// The first part of pixel `(i, j)`'s ray that is kept ([`Camera::segments`]);
    /// `None` if nothing of it is.
    pub fn ray(&self, i: usize, j: usize) -> Option<Line> {
        self.segments(i, j).first().map(|s| s.0)
    }

    /// [`Camera::ray`] of every pixel, row by row from the top.
    pub fn rays(&self) -> Vec<Option<Line>> {
        (0..self.height)
            .flat_map(|j| (0..self.width).map(move |i| (i, j)))
            .map(|(i, j)| self.ray(i, j))
            .collect()
    }
}

impl Model {
    /// Box around part of the model, measured: `n × n` lines along each
    /// axis across [`Model::bounds`], then again across that result,
    /// keeping the ends of the segments in the level-0 cells of `scope`
    /// (and of overlaps), widened by one line spacing. Much tighter than
    /// [`Model::bounds`] when cell boxes are loose (cells from CAD
    /// conversions often have boxes kilometres wide), but sampled: a part
    /// thinner than the spacing at the very edge can be left out. Empty if
    /// nothing is found.
    pub fn extent(&self, scope: Scope, n: usize) -> Aabb {
        let b = self.bounds(scope);
        if !b.is_finite() || n == 0 {
            return Aabb::EMPTY;
        }
        // A second pass over the first result, with finer spacing.
        let first = self.extent_in(scope, &b, n);
        if first.is_empty() {
            return first;
        }
        self.extent_in(scope, &first, n)
    }

    fn extent_in(&self, scope: Scope, b: &Aabb, n: usize) -> Aabb {
        let mut lines = Vec::with_capacity(3 * n * n);
        let mut pad = 0.0f64;
        for d in 0..3 {
            let (a, c) = match d {
                0 => (1, 2),
                1 => (0, 2),
                _ => (0, 1),
            };
            let (ha, hc) = (
                (b.hi[a] - b.lo[a]) / n as f64,
                (b.hi[c] - b.lo[c]) / n as f64,
            );
            pad = pad.max(ha).max(hc);
            for i in 0..n {
                for j in 0..n {
                    let mut o = [0.0; 3];
                    o[d] = b.lo[d];
                    o[a] = b.lo[a] + (i as f64 + 0.5) * ha;
                    o[c] = b.lo[c] + (j as f64 + 0.5) * hc;
                    let mut dir = [0.0; 3];
                    dir[d] = 1.0;
                    lines.push(Line {
                        origin: o,
                        dir,
                        t_min: 0.0,
                        t_max: b.hi[d] - b.lo[d],
                    });
                }
            }
        }
        let tr = self.trace_unchecked(&lines, false, None);
        let holds: Vec<bool> = tr
            .labels
            .iter()
            .map(|l| match l.path.first() {
                Some(s) => {
                    let c = &self.cells[s.cell as usize];
                    match scope {
                        Scope::Contents => c.material != 0 || c.fill.is_some(),
                        Scope::Problem => !c.imp_zero,
                    }
                }
                None => matches!(l.status, crate::Status::Overlap),
            })
            .collect();
        let mut out = Aabb::EMPTY;
        for (i, line) in lines.iter().enumerate() {
            for k in tr.segments(i) {
                if holds[tr.ids[k] as usize] {
                    out.grow_point(line.at(tr.t0[k]));
                    out.grow_point(line.at(tr.t1[k]));
                }
            }
        }
        if out.is_empty() {
            return out;
        }
        Aabb {
            lo: [0, 1, 2].map(|i| (out.lo[i] - pad).max(b.lo[i])),
            hi: [0, 1, 2].map(|i| (out.hi[i] + pad).min(b.hi[i])),
        }
    }

    /// Trace the rows of a slice ([`Slice::rows`]); [`Slice::pixels`]
    /// samples the result. [`Error::Line`](crate::Error::Line) if `u` is
    /// not a unit vector or the width is not finite and positive.
    pub fn slice(&self, slice: &Slice, depth: crate::Depth) -> Result<Tracing> {
        self.trace(&slice.rows(), depth)
    }

    /// First hits of the camera's rays, one per pixel, row by row from the
    /// top ([`Model::pixel_hits`] of every pixel).
    pub fn render(&self, camera: &Camera, vis: &Visibility) -> Result<Hits> {
        let pixels: Vec<(usize, usize)> = (0..camera.height)
            .flat_map(|j| (0..camera.width).map(move |i| (i, j)))
            .collect();
        self.pixel_hits(camera, &pixels, vis, None)
    }

    /// First hits of the rays of `pixels` (column, row from the top) of
    /// `camera`, `t` along each ray from [`Camera::origin`]. Only what the
    /// camera's cut keeps is seen: each part of a ray in turn
    /// ([`Camera::segments`]) until one hits. A part that starts on a cut
    /// face inside a visible cell hits there, with the face's normal (and no
    /// surface). Pixels whose ray is cut away entirely hit nothing.
    /// [`Error::Line`](crate::Error::Line) for a degenerate camera (the eye
    /// at the target, `up` along the view, a zero-sized image). `skip` as
    /// for [`Model::first_hits`].
    pub fn pixel_hits(
        &self,
        camera: &Camera,
        pixels: &[(usize, usize)],
        vis: &Visibility,
        skip: Option<&crate::Skip>,
    ) -> Result<Hits> {
        let segments: Vec<Vec<(Line, Option<Vec3>)>> =
            pixels.iter().map(|&(i, j)| camera.segments(i, j)).collect();
        let n = pixels.len();
        let mut out = Hits {
            t: vec![f64::INFINITY; n],
            ids: vec![NO_HIT; n],
            surface: vec![NO_SURFACE; n],
            normal: vec![[0.0; 3]; n],
            labels: Vec::new(),
            ghosts: if vis.has_ghosts() {
                vec![Default::default(); n]
            } else {
                Vec::new()
            },
            veils: if vis.veil {
                vec![Default::default(); n]
            } else {
                Vec::new()
            },
        };
        // The k-th parts of the rays not hit yet, in one batch each.
        let mut open: Vec<usize> = (0..n).filter(|&p| !segments[p].is_empty()).collect();
        let mut k = 0;
        let mut label_ids: std::collections::HashMap<crate::Label, u32> = Default::default();
        while !open.is_empty() {
            let rays: Vec<Line> = open.iter().map(|&p| segments[p][k].0).collect();
            let h = self.first_hits(&rays, vis, skip)?;
            let mut next = Vec::new();
            for (q, &p) in open.iter().enumerate() {
                if !h.ghosts.is_empty() {
                    out.ghosts[p] = out.ghosts[p].then(h.ghosts[q]);
                }
                if !h.veils.is_empty() {
                    out.veils[p] = out.veils[p].or(h.veils[q]);
                }
                if h.ids[q] == NO_HIT {
                    if k + 1 < segments[p].len() {
                        next.push(p);
                    }
                    continue;
                }
                let label = &h.labels[h.ids[q] as usize];
                let id = *label_ids.entry(label.clone()).or_insert_with(|| {
                    out.labels.push(label.clone());
                    (out.labels.len() - 1) as u32
                });
                out.t[p] = h.t[q];
                out.ids[p] = id;
                out.surface[p] = h.surface[q];
                out.normal[p] = h.normal[q];
                let (line, cap) = &segments[p][k];
                if let Some(c) = cap {
                    if h.surface[q] == NO_SURFACE && h.t[q] <= line.t_min {
                        out.normal[p] = *c;
                    }
                }
            }
            open = next;
            k += 1;
        }
        Ok(out)
    }
}
