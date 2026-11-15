//! What is under the cursor, exactly, from the CPU engine: the first
//! visible hit of a 3D pixel's ray, or the point of a slice pixel, and the
//! surface there (the one a 3D hit enters through, the nearest crossing
//! within a few screen points in a slice).

use llevant::{Camera, Label, Model, NearSurface, Status, SurfaceId, Vec3, Visibility};
use llevant::{NO_HIT, NO_SURFACE};

/// How near the pointer (screen points) a slice's surface is picked.
pub const SURFACE_REACH: f64 = 6.0;

/// A point and where it is.
#[derive(Clone, Debug)]
pub struct Picked {
    pub point: Vec3,
    pub label: Label,
    /// Distance from the eye (3D picks).
    pub t: Option<f64>,
    /// The surface at the point: entered there by a 3D ray (distance 0),
    /// or the nearest crossing in a slice.
    pub surface: Option<NearSurface>,
    /// For the point inspected ([`Picked::detailed`]): every surface through
    /// the surface's point that bounds a cell there, the picked one and
    /// those coincident with it; and the cells using the picked one.
    pub through: Vec<u32>,
    pub users: Option<Vec<u32>>,
}

/// `15`, or `15.3` for facet 3 of macrobody 15.
pub fn surface_name(id: SurfaceId) -> String {
    match id.facet {
        0 => id.id.to_string(),
        f => format!("{}.{f}", id.id),
    }
}

impl Picked {
    /// Index of the deepest cell, when inside one.
    pub fn cell(&self) -> Option<u32> {
        self.label.cell()
    }

    /// The picked surface's MCNP number.
    pub fn surface_id(&self, m: &Model) -> Option<SurfaceId> {
        m.surface_id(self.surface.as_ref()?.shape)
    }

    /// `self` with what the inspector shows beyond a hover's banner: the
    /// surfaces through the surface's point and the cells using it.
    pub fn detailed(self, m: &Model, x: &llevant_scene::index::ModelIndex) -> Picked {
        let mut p = self.with_through(m);
        if let Some(s) = &p.surface {
            p.users = m.surface_id(s.shape).map(|id| x.surface_cells(m, &[id.id]));
        }
        p
    }

    /// `self` with the surfaces through the surface's point
    /// ([`Picked::through`]), the picked one first if no cell is bounded
    /// by it there (an IMP=0 boundary seen from outside, say).
    pub fn with_through(mut self, m: &Model) -> Picked {
        if let Some(s) = &self.surface {
            self.through = m.surfaces_across(s.point, s.dir);
            if !self.through.contains(&s.shape) {
                self.through.insert(0, s.shape);
            }
        }
        self
    }

    /// `self` picking compiled surface `shape` instead (one through the
    /// same point).
    pub fn with_surface(&self, shape: u32) -> Picked {
        let mut p = self.clone();
        if let Some(s) = &mut p.surface {
            s.shape = shape;
        }
        p
    }

    /// One to three short lines for a banner: the deepest cell and what it
    /// holds, or the status where no single cell holds the point; and the
    /// surface there.
    pub fn summary(&self, m: &Model) -> Vec<String> {
        let mut out = self.cell_summary(m);
        if let Some(id) = self.surface_id(m) {
            out.push(format!("s {}", surface_name(id)));
        }
        out
    }

    fn cell_summary(&self, m: &Model) -> Vec<String> {
        match (self.label.status, self.label.path.last()) {
            (Status::Inside, Some(s)) => {
                let c = m.cell(s.cell);
                let u = m.units()[c.unit as usize].universe;
                let what = match (c.fill.is_some(), c.material) {
                    (true, _) => "filled".to_string(),
                    (false, 0) => "void".to_string(),
                    (false, mat) => format!("m{mat} at {}", c.density),
                };
                vec![format!("cell {} (u={u})", c.id), what]
            }
            (status, _) => vec![status.name().to_string()],
        }
    }

    /// Lines of text describing the pick.
    pub fn describe(&self, m: &Model) -> Vec<String> {
        let p = self.point;
        let mut out = vec![format!("({:.4}, {:.4}, {:.4}) cm", p[0], p[1], p[2])];
        if let Some(t) = self.t {
            out.push(format!("{t:.3} cm from the eye"));
        }
        let status = match self.label.status {
            Status::Inside => "inside",
            Status::Gap => "gap: no cell holds it",
            Status::Overlap => "overlap",
            Status::OnSurface => "on a surface",
            Status::Unsupported => "unsupported cell",
            Status::TooDeep => "nesting too deep",
            _ => "unknown",
        };
        out.push(status.to_string());
        for (level, s) in self.label.path.iter().enumerate() {
            let c = m.cell(s.cell);
            let u = m.units()[c.unit as usize].universe;
            let mut line = format!("level {level}: cell {} (u={u})", c.id);
            if let Some(ijk) = s.lattice {
                line += &format!(" [{} {} {}]", ijk[0], ijk[1], ijk[2]);
            }
            if c.fill.is_none() {
                if c.material == 0 {
                    line += ", void";
                } else {
                    line += &format!(", m{} at {}", c.material, c.density);
                }
            }
            out.push(line);
        }
        if let Some(id) = self.surface_id(m) {
            out.push(format!("surface {}", surface_name(id)));
        }
        if !self.label.involved.is_empty() {
            let ids: Vec<String> = self
                .label
                .involved
                .iter()
                .map(|&c| m.cell(c).id.to_string())
                .collect();
            out.push(format!("cells involved: {}", ids.join(" ")));
        }
        out
    }
}

/// The first visible hit along pixel `(i, j)` of `camera`.
pub fn pick_ray(
    m: &Model,
    vis: &Visibility,
    camera: &Camera,
    i: usize,
    j: usize,
) -> Option<Picked> {
    let ray = camera.ray(i, j)?;
    let h = m.first_hits(&[ray], vis, None).ok()?;
    let id = h.ids[0];
    if id == NO_HIT {
        return None;
    }
    let t = h.t[0];
    let point = [0, 1, 2].map(|k| ray.origin[k] + t * ray.dir[k]);
    let surface = (h.surface[0] != NO_SURFACE).then(|| NearSurface {
        shape: m.bounding_surface(h.surface[0], point, ray.dir),
        point,
        distance: 0.0,
        dir: ray.dir,
    });
    // The label of the visible segment, as the renderer sees it.
    Some(Picked {
        point,
        label: h.labels[id as usize].clone(),
        t: Some(t),
        surface,
        through: Vec::new(),
        users: None,
    })
}

/// Where the point `p` is.
pub fn pick_point(m: &Model, p: Vec3) -> Picked {
    Picked {
        point: p,
        label: m.classify_point(p),
        t: None,
        surface: None,
        through: Vec::new(),
        users: None,
    }
}

/// [`pick_point`] at `p` of a plane with axes `u` and `v`, with the surface
/// crossed nearest it within `reach` cm (along the axes and diagonals), as
/// the slice shows it at `level`.
pub fn pick_plane_point(
    m: &Model,
    p: Vec3,
    u: Vec3,
    v: Vec3,
    reach: f64,
    level: Option<usize>,
) -> Picked {
    let mut out = pick_point(m, p);
    let d = |a: f64, b: f64| [0, 1, 2].map(|k| a * u[k] + b * v[k]);
    let dirs = [d(1.0, 0.0), d(0.0, 1.0), d(1.0, 1.0), d(1.0, -1.0)];
    out.surface = m.nearest_surface(p, &dirs, reach, level).ok().flatten();
    out
}
