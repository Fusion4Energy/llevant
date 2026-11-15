//! FMESH tallies laid out over the model (in Tally): each mesh as plain
//! data in the scene (undone, saved with the view), the cards MCNP reads
//! for it (written by `aci_tallies_geom`, which reads them back the same),
//! and the FMESH cards the input already has.
//!
//! A mesh keeps its fine bounds, the way MCNP has them once it has read a
//! card: coarse bins and their counts are how cards and the FMESH panel
//! write them ([`coarse_bins`]), always the roundest that give the same
//! bounds. Rectangular meshes keep their bounds along their own x, y, z
//! (the first ones being `ORIGIN`), placed by `TR=`; cylindrical ones their
//! r, z and theta (revolutions) from 0, about `ORIGIN`, `AXS` and `VEC`, and
//! may be placed by `TR=` too.

pub mod draw;

use std::collections::{BTreeMap, BTreeSet};

use aci_tallies_geom::mcnp_input::{coarse_bins, fine_bounds, fmeshes_of, FmeshCard, FmeshError};
use aci_tallies_geom::{CylFrame, Geometry, Transform};
use llevant::vec3::{add, cross, dot, norm, normalize, scale, sub};
use llevant::{Aabb, Vec3};
use serde::{Deserialize, Serialize};

use crate::tools::handles::turn;

/// The geometry of a mesh, before `TR=`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "geom", rename_all = "lowercase")]
pub enum MeshShape {
    /// Bounds along the mesh's x, y and z.
    Rect {
        x: Vec<f64>,
        y: Vec<f64>,
        z: Vec<f64>,
    },
    /// Bounds of r and z (from 0, along `axs` from `origin`) and theta
    /// (revolutions from `vec`, 0 to 1).
    Cyl {
        origin: Vec3,
        axs: Vec3,
        vec: Vec3,
        r: Vec<f64>,
        z: Vec<f64>,
        theta: Vec<f64>,
    },
}

/// A `TRn` placing a mesh: the world point of the mesh's origin and its
/// axes (columns) in the world, as [`Transform`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrState {
    pub number: u32,
    pub translation: Vec3,
    pub rotation: [[f64; 3]; 3],
}

impl TrState {
    pub fn transform(&self) -> Transform {
        Transform {
            rotation: self.rotation,
            translation: self.translation,
        }
    }

    pub fn of(number: u32, t: &Transform) -> TrState {
        TrState {
            number,
            translation: t.translation,
            rotation: t.rotation,
        }
    }
}

/// One FMESH tally being laid out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshState {
    /// `FMESHn`: ends in 4.
    pub id: u32,
    /// The particle designator, `n` in `FMESH14:n`.
    #[serde(default = "neutrons")]
    pub particle: String,
    /// `*FMESH`: energy-weighted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub energy_weighted: bool,
    pub shape: MeshShape,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tr: Option<TrState>,
    /// `EMESH` bounds (MeV, from 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy: Option<Vec<f64>>,
    /// `TMESH` bounds (shakes, from -1e36).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<Vec<f64>>,
    /// `OUT=`, lowercase (`col`, `cf`, `ij`, `ik`, `jk`); MCNP's default
    /// (`col`) when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
    /// Written as the tally's `FC` card.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

fn neutrons() -> String {
    "n".into()
}

/// The `OUT=` values MCNP 6.2 takes.
pub const OUTS: [&str; 5] = ["col", "cf", "ij", "ik", "jk"];

/// The names of a shape's axes, as the FMESH panel shows them.
pub fn axis_names(cyl: bool) -> [&'static str; 3] {
    if cyl {
        ["r", "z", "θ"]
    } else {
        ["x", "y", "z"]
    }
}

impl MeshState {
    /// A mesh along the world's axes from `lo` to `hi`, of [`default_bins`].
    pub fn rect(id: u32, lo: Vec3, hi: Vec3) -> MeshState {
        MeshState::with_shape(
            id,
            MeshShape::Rect {
                x: default_bins(lo[0], hi[0]),
                y: default_bins(lo[1], hi[1]),
                z: default_bins(lo[2], hi[2]),
            },
            None,
        )
    }

    /// A mesh along a right-handed frame's axes `u`, `v`, `n` (placed by
    /// `TR=tr` at `origin`), from `lo` to `hi` in its coordinates.
    pub fn rect_in_frame(
        id: u32,
        tr: u32,
        origin: Vec3,
        [u, v, n]: [Vec3; 3],
        lo: Vec3,
        hi: Vec3,
    ) -> MeshState {
        let mut m = MeshState::rect(id, lo, hi);
        m.tr = Some(TrState {
            number: tr,
            translation: origin,
            rotation: [0, 1, 2].map(|r| [u[r], v[r], n[r]]),
        });
        m
    }

    /// A cylinder about `axis` from `base`, of `radius` and `height`, its
    /// theta measured from `vec`, in one theta bin.
    pub fn cylinder(
        id: u32,
        base: Vec3,
        axis: Vec3,
        vec: Vec3,
        radius: f64,
        height: f64,
    ) -> MeshState {
        MeshState::with_shape(
            id,
            MeshShape::Cyl {
                origin: base,
                axs: tidy(normalize(axis)),
                vec: tidy(normalize(vec)),
                r: default_bins(0.0, radius),
                z: default_bins(0.0, height),
                theta: vec![0.0, 1.0],
            },
            None,
        )
    }

    fn with_shape(id: u32, shape: MeshShape, tr: Option<TrState>) -> MeshState {
        MeshState {
            id,
            particle: neutrons(),
            energy_weighted: false,
            shape,
            tr,
            energy: None,
            time: None,
            out: None,
            comment: String::new(),
        }
    }

    pub fn is_cylinder(&self) -> bool {
        matches!(self.shape, MeshShape::Cyl { .. })
    }

    /// The bounds of the three axes (x y z, or r z theta).
    pub fn axes(&self) -> [&Vec<f64>; 3] {
        match &self.shape {
            MeshShape::Rect { x, y, z } => [x, y, z],
            MeshShape::Cyl { r, z, theta, .. } => [r, z, theta],
        }
    }

    pub fn axes_mut(&mut self) -> [&mut Vec<f64>; 3] {
        match &mut self.shape {
            MeshShape::Rect { x, y, z } => [x, y, z],
            MeshShape::Cyl { r, z, theta, .. } => [r, z, theta],
        }
    }

    /// Its card as `aci_tallies_geom` has it (checked when written).
    pub fn card(&self) -> FmeshCard {
        let local = match &self.shape {
            MeshShape::Rect { x, y, z } => Geometry::Rectangular {
                x_bounds: x.clone(),
                y_bounds: y.clone(),
                z_bounds: z.clone(),
                transform: None,
            },
            MeshShape::Cyl {
                origin,
                axs,
                vec,
                r,
                z,
                theta,
            } => Geometry::Cylindrical {
                r_bounds: r.clone(),
                z_bounds: z.clone(),
                theta_bounds: theta.clone(),
                origin: *origin,
                axis: *axs,
                vec: *vec,
            },
        };
        FmeshCard {
            id: self.id,
            particle: (!self.particle.trim().is_empty()).then(|| self.particle.trim().into()),
            energy_weighted: self.energy_weighted,
            local,
            transform: self.tr.map(|t| (t.number, t.transform())),
            energy_bounds: self.energy.clone(),
            time_bounds: self.time.clone(),
            out: self.out.clone(),
            line: 0,
        }
    }

    /// The mesh of a card read from an input.
    pub fn from_card(c: &FmeshCard) -> MeshState {
        let shape = match &c.local {
            Geometry::Rectangular {
                x_bounds,
                y_bounds,
                z_bounds,
                ..
            } => MeshShape::Rect {
                x: x_bounds.clone(),
                y: y_bounds.clone(),
                z: z_bounds.clone(),
            },
            Geometry::Cylindrical {
                r_bounds,
                z_bounds,
                theta_bounds,
                origin,
                axis,
                vec,
            } => MeshShape::Cyl {
                origin: *origin,
                axs: *axis,
                vec: *vec,
                r: r_bounds.clone(),
                z: z_bounds.clone(),
                theta: theta_bounds.clone(),
            },
        };
        MeshState {
            id: c.id,
            particle: c.particle.clone().unwrap_or_default(),
            energy_weighted: c.energy_weighted,
            shape,
            tr: c.transform.map(|(n, t)| TrState::of(n, &t)),
            energy: c.energy_bounds.clone(),
            time: c.time_bounds.clone(),
            out: c.out.clone(),
            comment: String::new(),
        }
    }

    /// Where MCNP scores it (`TR=` applied), or why it is not a mesh.
    pub fn geometry(&self) -> Result<Geometry, String> {
        let c = self.card();
        c.local.validate().map_err(|e| e.to_string())?;
        if let Some((_, t)) = &c.transform {
            t.validate().map_err(|e| e.to_string())?;
        }
        c.geometry().map_err(|e| e.to_string())
    }

    /// The cards MCNP reads: `FC` (the comment), `FMESH`, and `TRn`.
    pub fn text(&self) -> Result<String, String> {
        let mut text = String::new();
        let comment = self.comment.trim();
        if !comment.is_empty() {
            text = format!("FC{} {comment}\n", self.id);
        }
        let card = self.card().to_input().map_err(|e| e.message)?;
        Ok(text + &card)
    }

    /// Its placement: `TR=`, or none.
    pub fn placement(&self) -> Transform {
        self.tr.map_or(Transform::IDENTITY, |t| t.transform())
    }

    /// The cylinder's frame in the world (`TR=` applied).
    pub fn cyl_frame(&self) -> Option<CylFrame> {
        self.geometry().ok()?.cyl_frame()
    }

    /// Points on its outer boundary (the corners of a box; points around
    /// the rims of a cylinder), in the world.
    pub fn hull(&self) -> Vec<Vec3> {
        match &self.shape {
            MeshShape::Rect { x, y, z } => {
                let t = self.placement();
                let (Some(&x1), Some(&y1), Some(&z1)) = (x.last(), y.last(), z.last()) else {
                    return Vec::new();
                };
                let mut out = Vec::new();
                for a in [x[0], x1] {
                    for b in [y[0], y1] {
                        for c in [z[0], z1] {
                            out.push(t.to_world([a, b, c]));
                        }
                    }
                }
                out
            }
            MeshShape::Cyl { r, z, .. } => {
                let Some(f) = self.cyl_frame() else {
                    return Vec::new();
                };
                let (Some(&r1), Some(&z1)) = (r.last(), z.last()) else {
                    return Vec::new();
                };
                let mut out = Vec::new();
                for h in [0.0, z1] {
                    for k in 0..32 {
                        out.push(f.to_world(r1, h, k as f64 / 32.0));
                    }
                }
                out
            }
        }
    }

    /// The box around it in the world.
    pub fn world_box(&self) -> Option<Aabb> {
        let pts = self.hull();
        let first = *pts.first()?;
        let mut b = Aabb {
            lo: first,
            hi: first,
        };
        for p in pts {
            for k in 0..3 {
                b.lo[k] = b.lo[k].min(p[k]);
                b.hi[k] = b.hi[k].max(p[k]);
            }
        }
        Some(b)
    }

    /// The middle of it in the world.
    pub fn centre(&self) -> Vec3 {
        match &self.shape {
            MeshShape::Rect { .. } => {
                let [a, b, c] = self.axes().map(|v| mid(v));
                self.placement().to_world([a, b, c])
            }
            MeshShape::Cyl { z, .. } => match self.cyl_frame() {
                Some(f) => add(f.origin, scale(f.e_axis, mid(z))),
                None => [0.0; 3],
            },
        }
    }

    /// Moved by `d` in the world: its placement, or its own origin.
    pub fn moved(&self, d: Vec3) -> MeshState {
        let mut m = self.clone();
        if let Some(t) = &mut m.tr {
            t.translation = add(t.translation, d);
            return m;
        }
        match &mut m.shape {
            MeshShape::Rect { x, y, z } => {
                for (v, dk) in [x, y, z].into_iter().zip(d) {
                    shift(v, dk);
                }
            }
            MeshShape::Cyl { origin, .. } => *origin = add(*origin, d),
        }
        m
    }

    /// A cylinder's base moved `t` along its axis, its top staying where it
    /// is (z bounds counted from the base again). `None` for a box, or past
    /// the top.
    pub fn base_moved(&self, t: f64) -> Option<MeshState> {
        let mut m = self.clone();
        let MeshShape::Cyl { origin, axs, z, .. } = &mut m.shape else {
            return None;
        };
        let mut w = moved_end(z, false, t)?;
        shift(&mut w, -t);
        w[0] = 0.0;
        *z = w;
        *origin = add(*origin, scale(normalize(*axs), t));
        Some(m)
    }

    /// Turned by `angle` (radians) about the world axis `axis` through
    /// `centre`: a cylinder without `TR=` turns its own axes, any other mesh
    /// its placement (`TR=tr` if it had none).
    pub fn turned(&self, axis: Vec3, angle: f64, centre: Vec3, tr: u32) -> MeshState {
        let mut m = self.clone();
        let rot = |v: Vec3| turn(v, axis, angle);
        if let (
            None,
            MeshShape::Cyl {
                origin, axs, vec, ..
            },
        ) = (&m.tr, &mut m.shape)
        {
            *origin = add(centre, rot(sub(*origin, centre)));
            *axs = tidy(rot(*axs));
            *vec = tidy(rot(*vec));
            return m;
        }
        let t = m.tr.unwrap_or(TrState::of(tr, &Transform::IDENTITY));
        let cols = [0, 1, 2].map(|c| tidy(rot([0, 1, 2].map(|r| t.rotation[r][c]))));
        m.tr = Some(TrState {
            number: t.number,
            translation: add(centre, rot(sub(t.translation, centre))),
            rotation: [0, 1, 2].map(|r| [cols[0][r], cols[1][r], cols[2][r]]),
        });
        m
    }

    /// Fitted around `b` (in the world), keeping its shape, placement and
    /// numbers of bins; bounds rounded outwards.
    pub fn fitted(&self, b: &Aabb) -> MeshState {
        let corners: Vec<Vec3> = (0..8)
            .map(|k| [0, 1, 2].map(|d| if k >> d & 1 == 1 { b.hi[d] } else { b.lo[d] }))
            .collect();
        let size = (0..3).map(|k| b.hi[k] - b.lo[k]).fold(0.0, f64::max);
        let step = nice_step(size / 100.0);
        let mut m = self.clone();
        let t = self.placement();
        match &mut m.shape {
            MeshShape::Rect { x, y, z } => {
                let local: Vec<Vec3> = corners.iter().map(|&p| t.to_local(p)).collect();
                for (k, v) in [x, y, z].into_iter().enumerate() {
                    let lo = local.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
                    let hi = local.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
                    let (lo, hi) = ((lo / step).floor() * step, (hi / step).ceil() * step);
                    *v = even(lo, hi.max(lo + step), bins(v));
                }
            }
            MeshShape::Cyl {
                origin,
                axs,
                vec: _,
                r,
                z,
                ..
            } => {
                // The world frame, then back through the placement.
                let a = normalize(t.rotate(*axs));
                let c = b.center();
                let along: Vec<f64> = corners.iter().map(|&p| dot(sub(p, c), a)).collect();
                let lo = along.iter().copied().fold(f64::INFINITY, f64::min);
                let hi = along.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                let radius = corners
                    .iter()
                    .map(|&p| {
                        let q = sub(p, c);
                        norm(sub(q, scale(a, dot(q, a))))
                    })
                    .fold(0.0, f64::max);
                let height = ((hi - lo) / step).ceil().max(1.0) * step;
                let radius = (radius / step).ceil().max(1.0) * step;
                let base = add(c, scale(a, lo));
                *origin = t.to_local(base);
                *r = even(0.0, radius, bins(r));
                *z = even(0.0, height, bins(z));
            }
        }
        m
    }

    /// Placed along the slice's plane: a box's axes along the plane's
    /// (`u`, `v`, `n`, by `TR=tr`), a cylinder's axis along its normal;
    /// about the same centre, its bounds kept.
    pub fn aligned(&self, [u, v, n]: [Vec3; 3], tr: u32) -> MeshState {
        let c = self.centre();
        let mut m = self.clone();
        match &mut m.shape {
            MeshShape::Rect { x, y, z } => {
                let local_c = [mid(x), mid(y), mid(z)];
                let n = cross(u, v);
                let rotation = [0, 1, 2].map(|r| [u[r], v[r], n[r]].map(tidy1));
                let t = Transform {
                    rotation,
                    translation: [0.0; 3],
                };
                let number = self.tr.map_or(tr, |t| t.number);
                m.tr = Some(TrState {
                    number,
                    translation: sub(c, t.rotate(local_c)),
                    rotation,
                });
            }
            MeshShape::Cyl {
                origin,
                axs,
                vec,
                z,
                ..
            } => {
                m.tr = None;
                *axs = tidy(normalize(n));
                *vec = tidy(normalize(u));
                *origin = sub(c, scale(*axs, mid(z)));
            }
        }
        m
    }

    /// Figures to judge it by.
    pub fn stats(&self) -> Stats {
        let ax = self.axes();
        let widths = |v: &Vec<f64>, f: f64| -> (f64, f64) {
            v.windows(2)
                .map(|w| (w[1] - w[0]) * f)
                .fold((f64::INFINITY, 0.0), |(a, b), d| (a.min(d), b.max(d)))
        };
        let mut sizes = [0, 1, 2].map(|k| widths(ax[k], 1.0));
        if self.is_cylinder() {
            // Theta as arc length at the outer radius.
            let r = ax[0].last().copied().unwrap_or(0.0);
            sizes[2] = widths(ax[2], std::f64::consts::TAU * r);
        }
        let voxels = ax
            .iter()
            .map(|v| v.len().saturating_sub(1) as u64)
            .product::<u64>();
        let printed = |b: &Option<Vec<f64>>| match b {
            Some(b) if b.len() > 2 => b.len() as u64, // the bins and their Total
            _ => 1,
        };
        let bins = voxels * printed(&self.energy) * printed(&self.time);
        let big = sizes.iter().map(|s| s.1).fold(0.0, f64::max);
        let small = sizes.iter().map(|s| s.0).fold(f64::INFINITY, f64::min);
        // Bytes a value takes in the meshtal file: a row of coordinates,
        // result and error (and volume and their product with `cf`), or two
        // numbers of a matrix.
        let per_bin = match self.out.as_deref() {
            Some("cf") => 100.0,
            Some("ij" | "ik" | "jk") => 26.0,
            _ => 70.0,
        };
        Stats {
            voxels,
            bins,
            sizes,
            aspect: if small > 0.0 {
                big / small
            } else {
                f64::INFINITY
            },
            meshtal_bytes: bins as f64 * per_bin,
        }
    }

    /// What MCNP would refuse, or what is likely a mistake: this mesh as
    /// the `k`th of `all`, for a model whose region is `region`.
    pub fn problems(
        &self,
        k: usize,
        all: &[MeshState],
        input: Option<&InputMeshes>,
        region: &Aabb,
    ) -> Vec<Problem> {
        let mut out = Vec::new();
        if let Err(e) = self.text() {
            out.push(Problem::Error(e));
        }
        if let Some(i) = input {
            if i.tallies.contains(&self.id) {
                out.push(Problem::Error(format!(
                    "the input has a tally {} already",
                    self.id
                )));
            }
            if let Some(t) = &self.tr {
                if let Some(theirs) = i.transforms.get(&t.number) {
                    let same = theirs.is_some_and(|x| close(&x, &t.transform()));
                    if !same {
                        out.push(Problem::Error(format!(
                            "the input has a different TR{} already",
                            t.number
                        )));
                    }
                }
            }
        }
        for (j, other) in all.iter().enumerate() {
            if j == k {
                continue;
            }
            if other.id == self.id {
                out.push(Problem::Error(format!(
                    "another mesh here is FMESH{} too",
                    self.id
                )));
            }
            if let (Some(a), Some(b)) = (&self.tr, &other.tr) {
                if a.number == b.number && !close(&a.transform(), &b.transform()) {
                    out.push(Problem::Error(format!(
                        "another mesh here has a different TR{}",
                        a.number
                    )));
                }
            }
        }
        if self.tr.is_some_and(|t| t.number > 999) {
            out.push(Problem::Error("MCNP's TR numbers go up to 999".into()));
        }
        if let Some(b) = self.world_box() {
            let apart = (0..3).any(|d| b.hi[d] < region.lo[d] || b.lo[d] > region.hi[d]);
            if apart {
                out.push(Problem::Warning("it is outside the model".into()));
            }
        }
        let s = self.stats();
        if s.bins > 100_000_000 {
            out.push(Problem::Warning(format!(
                "{} bins: MCNP needs ~{} of memory for them",
                s.bins,
                bytes(s.bins as f64 * 24.0)
            )));
        }
        if s.aspect > 100.0 {
            out.push(Problem::Warning(format!(
                "voxels {:.0}× longer one way than another",
                s.aspect
            )));
        }
        out
    }
}

/// [`MeshState::stats`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    pub voxels: u64,
    /// Values MCNP prints: voxels times energy and time bins (with their
    /// Total bins).
    pub bins: u64,
    /// Smallest and largest bin along each axis (cm; theta as arc length
    /// at the outer radius).
    pub sizes: [(f64, f64); 3],
    /// Largest bin over smallest.
    pub aspect: f64,
    /// About how large the meshtal file's part for it is.
    pub meshtal_bytes: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    /// MCNP would refuse the card(s).
    Error(String),
    /// Likely a mistake.
    Warning(String),
}

/// `x` bytes as `12 kB`, `3.4 GB`.
pub fn bytes(x: f64) -> String {
    let units = ["B", "kB", "MB", "GB", "TB"];
    let mut v = x;
    let mut u = 0;
    while v >= 1000.0 && u + 1 < units.len() {
        v /= 1000.0;
        u += 1;
    }
    if v < 10.0 && u > 0 {
        format!("{v:.1} {}", units[u])
    } else {
        format!("{v:.0} {}", units[u])
    }
}

fn close(a: &Transform, b: &Transform) -> bool {
    let d = |x: &[f64], y: &[f64]| {
        x.iter()
            .zip(y)
            .all(|(p, q)| (p - q).abs() <= 1e-9 * (1.0 + p.abs()))
    };
    d(&a.translation, &b.translation) && d(a.rotation.as_flattened(), b.rotation.as_flattened())
}

/// The cards of `meshes`, one after the other, each `TRn` once, and none
/// the input already has as it is.
pub fn cards_text<'a>(
    meshes: impl IntoIterator<Item = &'a MeshState>,
    input: Option<&InputMeshes>,
) -> Result<String, String> {
    let mut out = String::new();
    let mut written = BTreeSet::new();
    for m in meshes {
        let mut text = m.text().map_err(|e| format!("FMESH{}: {e}", m.id))?;
        if let Some(t) = &m.tr {
            let theirs = input
                .and_then(|i| i.transforms.get(&t.number))
                .copied()
                .flatten();
            let known = theirs.is_some_and(|x| close(&x, &t.transform()));
            if !written.insert(t.number) || known {
                let tr = aci_tallies_geom::mcnp_input::transform_card(t.number, &t.transform());
                if let Some(s) = text.strip_suffix(&tr) {
                    text = s.to_string();
                }
            }
        }
        out.push_str(&text);
    }
    Ok(out)
}

/// The middle of the first and last bounds.
fn mid(v: &[f64]) -> f64 {
    match (v.first(), v.last()) {
        (Some(a), Some(b)) => 0.5 * (a + b),
        _ => 0.0,
    }
}

/// Fine bins of `v`.
pub fn bins(v: &[f64]) -> u32 {
    v.len().saturating_sub(1).max(1) as u32
}

fn shift(v: &mut [f64], d: f64) {
    for x in v {
        *x += d;
    }
}

/// `n` even bins from `lo` to `hi` (as MCNP splits a coarse bin).
pub fn even(lo: f64, hi: f64, n: u32) -> Vec<f64> {
    fine_bounds(lo, &[hi], &[n.max(1)])
}

/// Bins of about 10 cm from `lo` to `hi`, at most 20.
pub fn default_bins(lo: f64, hi: f64) -> Vec<f64> {
    let n = ((hi - lo) / 10.0).ceil().clamp(1.0, 20.0);
    even(lo, hi, n as u32)
}

/// Bounds `fine` with their first (`high` false) or last one moved to
/// `to`: coarse bins it passes are dropped, and the one it ends in keeps its
/// number of fine bins. `None` if nothing would be left.
pub fn moved_end(fine: &[f64], high: bool, to: f64) -> Option<Vec<f64>> {
    let (mut lo, mut coarse, mut counts) = coarse_bins(fine);
    if !to.is_finite() {
        return None;
    }
    if high {
        while coarse.len() > 1 && coarse[coarse.len() - 2] >= to {
            coarse.pop();
            counts.pop();
        }
        if to <= lo {
            return None;
        }
        *coarse.last_mut()? = to;
    } else {
        while coarse.len() > 1 && coarse[0] <= to {
            coarse.remove(0);
            counts.remove(0);
        }
        if to >= *coarse.first()? {
            return None;
        }
        lo = to;
    }
    Some(fine_bounds(lo, &coarse, &counts))
}

/// The number of 1, 2 or 5 times a power of ten nearest above `x`.
pub fn nice_step(x: f64) -> f64 {
    if !(x > 0.0) || !x.is_finite() {
        return 1.0;
    }
    let p = 10f64.powf(x.log10().floor());
    for m in [1.0, 2.0, 5.0, 10.0] {
        if m * p >= x * 0.999_999 {
            return round_digits(m * p);
        }
    }
    round_digits(10.0 * p)
}

/// `v` rounded to a multiple of `step`, without the noise of the product
/// (`0.30000000000000004`).
pub fn snap(v: f64, step: f64) -> f64 {
    round_digits((v / step).round() * step)
}

fn round_digits(v: f64) -> f64 {
    format!("{v:.12e}").parse().unwrap_or(v)
}

/// Components within 1e-12 of 0 or ±1 made exactly that.
fn tidy(v: Vec3) -> Vec3 {
    v.map(tidy1)
}

fn tidy1(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        0.0
    } else if (x.abs() - 1.0).abs() < 1e-12 {
        x.signum()
    } else {
        x
    }
}

/// Which bounds a text field of the FMESH panel gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisKind {
    /// A box's x, y or z (from `lo`).
    Rect,
    /// A cylinder's r, z (from 0) or theta (from 0 to 1).
    CylR,
    CylZ,
    CylTheta,
    Energy,
    Time,
}

/// The fine bounds of coarse bounds and counts written as on an FMESH card
/// (`-5 2i 10`, `2 2r 1`; empty counts: one fine bin each): read by
/// `aci_tallies_geom` from a card made of them, so MCNP's shortcuts and
/// rules apply.
pub fn parse_axis(kind: AxisKind, lo: f64, coarse: &str, ints: &str) -> Result<Vec<f64>, String> {
    let (mesh, ints_kw, geom) = match kind {
        AxisKind::Rect => ("imesh", "iints", "xyz"),
        AxisKind::CylR => ("imesh", "iints", "cyl"),
        AxisKind::CylZ => ("jmesh", "jints", "cyl"),
        AxisKind::CylTheta => ("kmesh", "kints", "cyl"),
        AxisKind::Energy => ("emesh", "eints", "xyz"),
        AxisKind::Time => ("tmesh", "tints", "xyz"),
    };
    if coarse.trim().is_empty() {
        return Err("no bounds".into());
    }
    let mut words = vec![format!("geom={geom}")];
    if kind == AxisKind::Rect {
        words.push(format!("origin {lo:e} 0 0"));
    }
    words.push(format!("{mesh} {coarse}"));
    if !ints.trim().is_empty() {
        words.push(format!("{ints_kw} {ints}"));
    }
    for (kw, v) in [("imesh", "1"), ("jmesh", "1"), ("kmesh", "1")] {
        if kw != mesh {
            words.push(format!("{kw} {v}"));
        }
    }
    // One word per line: long lists stay within MCNP's columns.
    let mut card = "fmesh4:n".to_string();
    for w in words.iter().flat_map(|w| w.split_whitespace()) {
        card.push_str("\n     ");
        card.push_str(w);
    }
    let input = format!("t\n1 0 -1 imp:n=1\n2 0 1 imp:n=0\n\n1 so 1\n\nmode n\n{card}\n");
    let c = aci_tallies_geom::mcnp_input::read_fmeshes(&input)
        .pop()
        .ok_or("not read")?
        .map_err(|e| e.message)?;
    let (x, y, z) = c.local.axes();
    Ok(match kind {
        AxisKind::Rect | AxisKind::CylR => x.to_vec(),
        AxisKind::CylZ => y.to_vec(),
        AxisKind::CylTheta => z.to_vec(),
        AxisKind::Energy => c.energy_bounds.ok_or("no bounds")?,
        AxisKind::Time => c.time_bounds.ok_or("no bounds")?,
    })
}

/// Fine bounds as the FMESH panel writes them: the first one, the coarse
/// bounds, and their counts (empty when all are 1).
pub fn axis_text(fine: &[f64]) -> (String, String, String) {
    let (lo, coarse, counts) = coarse_bins(fine);
    let join = |v: Vec<String>| v.join(" ");
    let ints = if counts.iter().all(|&n| n == 1) {
        String::new()
    } else {
        join(counts.iter().map(|n| n.to_string()).collect())
    };
    (
        number(lo),
        join(coarse.iter().map(|&v| number(v)).collect()),
        ints,
    )
}

/// `v` as briefly as gives it back exactly.
pub fn number(v: f64) -> String {
    let plain = v.to_string();
    let exp = format!("{v:e}");
    if exp.len() < plain.len() {
        exp
    } else {
        plain
    }
}

/// Numbers separated by blanks or commas (`1 0 0`, `1e3,2`, Fortran `1-3`).
pub fn parse_numbers(text: &str) -> Result<Vec<f64>, String> {
    text.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|w| !w.is_empty())
        .map(|w| migjorn::parse_float(w).ok_or_else(|| format!("`{w}` is not a number")))
        .collect()
}

/// A `TRn` card's values (`o1 o2 o3` and up to 9 cosines, or angles in
/// degrees with `degrees`), as MCNP builds them.
pub fn parse_tr(number: u32, text: &str, degrees: bool) -> Result<TrState, String> {
    let values = parse_numbers(text)?;
    let a = migjorn::Affine::from_mcnp(&values, degrees)?;
    let t: Transform = a.into();
    t.validate().map_err(|e| e.to_string())?;
    Ok(TrState::of(number, &t))
}

/// A `TRn` card's values for `t`: the displacement, then the cosines.
pub fn tr_values(t: &TrState) -> String {
    let card = aci_tallies_geom::mcnp_input::transform_card(t.number, &t.transform());
    card.split_whitespace()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ")
}

/// What the input has: its FMESH cards, and the tally and `TRn` numbers it
/// uses (with each transform, when MCNP can build it).
#[derive(Debug, Default)]
pub struct InputMeshes {
    pub cards: Vec<Result<FmeshCard, FmeshError>>,
    pub tallies: BTreeSet<u32>,
    pub transforms: BTreeMap<u32, Option<Transform>>,
}

impl InputMeshes {
    pub fn read(text: &str) -> InputMeshes {
        InputMeshes::of(&migjorn::Model::parse(text))
    }

    /// The FMESH cards and numbers of a parsed input.
    pub fn of(model: &migjorn::Model) -> InputMeshes {
        let tallies = model
            .data_cards()
            .filter(|c| {
                c.mnemonic()
                    .is_some_and(|m| m.eq_ignore_ascii_case("f") || m.eq_ignore_ascii_case("fmesh"))
            })
            .filter_map(|c| u32::try_from(c.number()?).ok())
            .collect();
        let transforms = model
            .transforms()
            .filter_map(|t| {
                let n = u32::try_from(t.id()?).ok()?;
                Some((n, t.affine().ok().map(Transform::from)))
            })
            .collect();
        InputMeshes {
            cards: fmeshes_of(model),
            tallies,
            transforms,
        }
    }

    pub fn load(path: &std::path::Path) -> Option<InputMeshes> {
        let bytes = std::fs::read(path).ok()?;
        Some(InputMeshes::read(&String::from_utf8_lossy(&bytes)))
    }

    /// The first tally number ending in 4 that neither the input nor
    /// `meshes` uses (from 14).
    pub fn free_tally(input: Option<&InputMeshes>, meshes: &[MeshState]) -> u32 {
        (1..)
            .map(|k| 10 * k + 4)
            .find(|n| {
                !input.is_some_and(|i| i.tallies.contains(n)) && !meshes.iter().any(|m| m.id == *n)
            })
            .expect("a free number")
    }

    /// The first `TRn` number that neither the input nor `meshes` uses.
    pub fn free_tr(input: Option<&InputMeshes>, meshes: &[MeshState]) -> u32 {
        (1..)
            .find(|n| {
                !input.is_some_and(|i| i.transforms.contains_key(n))
                    && !meshes.iter().any(|m| m.tr.is_some_and(|t| t.number == *n))
            })
            .expect("a free number")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_back(m: &MeshState) -> FmeshCard {
        let text = m.text().unwrap();
        let input = format!("t\n1 0 -1 imp:n=1\n2 0 1 imp:n=0\n\n1 so 100\n\nmode n\n{text}\n");
        aci_tallies_geom::mcnp_input::read_fmeshes(&input)
            .pop()
            .unwrap()
            .unwrap()
    }

    #[test]
    fn meshes_are_cards_and_back() {
        let mut m = MeshState::rect(14, [-10.0, -5.0, 0.0], [10.0, 5.0, 3.0]);
        m.comment = "flux".into();
        m.energy = Some(vec![0.0, 0.1, 1.0, 14.0]);
        m.out = Some("ij".into());
        assert!(m
            .text()
            .unwrap()
            .starts_with("FC14 flux\nFMESH14:n GEOM=XYZ ORIGIN -10 -5 0"));
        assert_eq!(
            MeshState::from_card(&read_back(&m)),
            MeshState {
                comment: String::new(),
                ..m.clone()
            }
        );

        let c = MeshState::cylinder(
            24,
            [1.0, 2.0, 3.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            50.0,
            120.0,
        );
        let turned = c.turned([1.0, 0.0, 0.0], 0.3, [0.0; 3], 5);
        assert!(turned.tr.is_none(), "a cylinder turns its own axes");
        let back = MeshState::from_card(&read_back(&turned));
        assert_eq!(back.axes(), turned.axes());

        let placed = m.turned([0.0, 0.0, 1.0], std::f64::consts::FRAC_PI_2, [0.0; 3], 7);
        let t = placed.tr.unwrap();
        assert_eq!(t.number, 7);
        assert_eq!(
            t.rotation,
            [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]
        );
        let card = read_back(&placed);
        let g = card.geometry().unwrap();
        // The corner at local (10, 5, 3) is now at world (-5, 10, 3).
        assert!(g.locate([-4.9, 9.9, 2.9]).is_some());
        assert!(g.locate([9.9, 4.9, 2.9]).is_none());
    }

    #[test]
    fn view_files_keep_meshes() {
        let mut m = MeshState::cylinder(4, [0.0; 3], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0], 5.0, 7.0);
        m.tr = Some(TrState::of(3, &Transform::IDENTITY));
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"geom\":\"cyl\""), "{json}");
        assert_eq!(serde_json::from_str::<MeshState>(&json).unwrap(), m);
    }

    #[test]
    fn ends_move_and_keep_their_bins() {
        let v = fine_bounds(0.0, &[10.0, 20.0], &[2, 5]);
        let w = moved_end(&v, true, 30.0).unwrap();
        assert_eq!(w, fine_bounds(0.0, &[10.0, 30.0], &[2, 5]));
        // Past a coarse bound: that coarse bin goes.
        assert_eq!(
            moved_end(&v, true, 8.0).unwrap(),
            fine_bounds(0.0, &[8.0], &[2])
        );
        assert_eq!(
            moved_end(&v, false, 12.0).unwrap(),
            fine_bounds(12.0, &[20.0], &[5])
        );
        assert_eq!(moved_end(&v, false, 25.0), None);
        assert_eq!(moved_end(&v, true, -1.0), None);
    }

    #[test]
    fn fields_read_as_cards_do() {
        assert_eq!(
            parse_axis(AxisKind::Rect, -10.0, "-5 2i 10", "2 2r 1").unwrap(),
            [-10.0, -7.5, -5.0, -2.5, 0.0, 2.5, 5.0, 10.0]
        );
        assert_eq!(
            parse_axis(AxisKind::CylTheta, 0.0, "0.5 1", "").unwrap(),
            [0.0, 0.5, 1.0]
        );
        assert!(parse_axis(AxisKind::CylTheta, 0.0, "0.5", "")
            .unwrap_err()
            .contains("1 revolution"));
        assert!(parse_axis(AxisKind::Rect, 5.0, "1", "").is_err());
        let (lo, coarse, ints) = axis_text(&fine_bounds(-3.0, &[0.0042, 13.3333], &[2, 3]));
        assert_eq!(
            (lo.as_str(), coarse.as_str(), ints.as_str()),
            ("-3", "0.0042 13.3333", "2 3")
        );
        let tr = parse_tr(2, "1 2 3 90 0 90 90 90 0 0 90 90", true).unwrap();
        assert_eq!(tr.translation, [1.0, 2.0, 3.0]);
        assert!(tr_values(&tr).starts_with("1 2 3 "));
        assert!(parse_tr(2, "1 2 x", false).is_err());
    }

    #[test]
    fn fitting_rounds_outwards_and_keeps_bins() {
        let m = MeshState::rect(14, [0.0; 3], [1.0, 1.0, 1.0]);
        let b = Aabb {
            lo: [-12.34, 0.1, 5.0],
            hi: [87.6, 3.0, 9.9],
        };
        let f = m.fitted(&b);
        let ax = f.axes();
        assert_eq!((ax[0][0], *ax[0].last().unwrap()), (-13.0, 88.0));
        assert_eq!(ax[0].len(), m.axes()[0].len());
        let c = MeshState::cylinder(24, [0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], 1.0, 1.0)
            .fitted(&b);
        let wb = c.world_box().unwrap();
        for k in 0..3 {
            assert!(
                wb.lo[k] <= b.lo[k] + 1e-9 && wb.hi[k] >= b.hi[k] - 1e-9,
                "{wb:?}"
            );
        }
    }

    #[test]
    fn problems_name_clashes() {
        let input = InputMeshes::read(
            "t\n1 0 -1 imp:n=1\n2 0 1 imp:n=0\n\n1 so 100\n\nmode n\nf14:n 1\ntr3 1 0 0\n",
        );
        assert_eq!(input.tallies, BTreeSet::from([14]));
        assert_eq!(InputMeshes::free_tally(Some(&input), &[]), 24);
        assert_eq!(InputMeshes::free_tr(Some(&input), &[]), 1);
        let region = Aabb {
            lo: [-100.0; 3],
            hi: [100.0; 3],
        };
        let mut m = MeshState::rect(14, [0.0; 3], [1.0; 3]);
        m.tr = Some(TrState::of(3, &Transform::IDENTITY));
        let p = m.problems(0, &[m.clone(), m.clone()], Some(&input), &region);
        let text = format!("{p:?}");
        assert!(text.contains("tally 14 already"), "{text}");
        assert!(text.contains("different TR3"), "{text}");
        assert!(text.contains("FMESH14 too"), "{text}");
        let far = MeshState::rect(24, [500.0; 3], [600.0; 3]);
        assert!(format!("{:?}", far.problems(0, &[], None, &region)).contains("outside"));
        assert!(format!(
            "{:?}",
            MeshState { id: 15, ..far }.problems(0, &[], None, &region)
        )
        .contains("ends in 4"));
    }

    #[test]
    fn cards_of_several_meshes_write_each_tr_once() {
        let mut a = MeshState::rect(14, [0.0; 3], [1.0; 3]);
        a.tr = Some(TrState::of(3, &Transform::IDENTITY));
        let b = MeshState {
            id: 24,
            ..a.clone()
        };
        let text = cards_text([&a, &b], None).unwrap();
        assert_eq!(text.matches("TR3 ").count(), 1, "{text}");
        assert_eq!(text.matches("TR=3").count(), 2, "{text}");
        let input = InputMeshes::read("t\n1 0 -1 imp:n=1\n2 0 1 imp:n=0\n\n1 so 1\n\ntr3 0 0 0\n");
        let text = cards_text([&a], Some(&input)).unwrap();
        assert!(!text.contains("TR3 ") && text.contains("TR=3"), "{text}");
    }

    #[test]
    fn steps_are_round() {
        assert_eq!(nice_step(0.13), 0.2);
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(7.0), 10.0);
        assert_eq!(snap(0.1 + 0.2, 0.1), 0.3);
        assert_eq!(bytes(2.5e9), "2.5 GB");
    }
}
