//! The ruler (decision 0012): a line through the model and every cell it
//! crosses with its length, measured by the engine's exact line tracing
//! ([`Model::trace`]) at the level shown. Two ways to lay it: between two
//! points, or from a surface along its normal, through the layers behind it
//! up to the next void. It is navigation, not the scene: not undone, not
//! saved; both views draw it, and the Results workspace plots a field
//! along it.

use llevant::{Depth, Line, Model, Status, Vec3};

/// How a ruler was laid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RulerMode {
    /// Between two points picked.
    #[default]
    TwoPoints,
    /// From a surface picked, along its normal through the layers behind
    /// it, up to the next void.
    Layers,
}

/// One stretch of the line in one cell (or a gap, an overlap…).
#[derive(Clone, Debug, PartialEq)]
pub struct Stretch {
    /// Distances from the start (cm).
    pub from: f64,
    pub to: f64,
    /// The deepest cell (index into the model's cells), if inside one.
    pub cell: Option<u32>,
    pub status: Status,
}

impl Stretch {
    pub fn length(&self) -> f64 {
        self.to - self.from
    }
}

/// A line measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Ruler {
    pub a: Vec3,
    pub b: Vec3,
    pub mode: RulerMode,
    pub stretches: Vec<Stretch>,
}

impl Ruler {
    pub fn length(&self) -> f64 {
        llevant::vec3::norm(llevant::vec3::sub(self.b, self.a))
    }

    /// The point `d` cm along the ruler.
    pub fn at(&self, d: f64) -> Vec3 {
        let u = llevant::vec3::normalize(llevant::vec3::sub(self.b, self.a));
        [0, 1, 2].map(|k| self.a[k] + d * u[k])
    }

    /// The stretches as CSV: cell, material, from, to, length (cm).
    pub fn csv(&self, m: &Model) -> String {
        let mut out = String::from("cell,material,from_cm,to_cm,length_cm\n");
        for s in &self.stretches {
            let (cell, mat) = match s.cell {
                Some(c) => {
                    let cell = m.cell(c);
                    let mat = if cell.fill.is_some() {
                        "fill".to_string()
                    } else {
                        cell.material.to_string()
                    };
                    (cell.id.to_string(), mat)
                }
                None => (status_name(s.status).to_string(), String::new()),
            };
            out.push_str(&format!(
                "{cell},{mat},{:.6},{:.6},{:.6}\n",
                s.from,
                s.to,
                s.length()
            ));
        }
        out
    }
}

/// What a stretch outside any one cell is.
pub fn status_name(s: Status) -> &'static str {
    match s {
        Status::Inside => "inside",
        Status::Gap => "gap",
        Status::Overlap => "overlap",
        Status::OnSurface => "on a surface",
        Status::Unsupported => "unsupported",
        Status::TooDeep => "too deep",
        _ => "unknown",
    }
}

/// The ruler from `a` to `b`: the cells crossed, at `level` (`None`:
/// every level, the deepest cells).
pub fn measure(
    m: &Model,
    a: Vec3,
    b: Vec3,
    mode: RulerMode,
    level: Option<usize>,
) -> Option<Ruler> {
    let d = llevant::vec3::sub(b, a);
    let len = llevant::vec3::norm(d);
    if !(len > 0.0) || !len.is_finite() {
        return None;
    }
    let line = Line::new(a, llevant::vec3::normalize(d), 0.0, len);
    let depth = level.map_or(Depth::All, Depth::Level);
    let t = m.trace(&[line], depth).ok()?;
    let mut stretches: Vec<Stretch> = Vec::new();
    for k in t.segments(0) {
        let s = t.segment(k);
        let cell = (s.label.status == Status::Inside)
            .then(|| s.label.cell())
            .flatten();
        // Stretches of one cell split by a crossing within it are one.
        match stretches.last_mut() {
            Some(last) if last.cell == cell && last.status == s.label.status => last.to = s.t1,
            _ => stretches.push(Stretch {
                from: s.t0,
                to: s.t1,
                cell,
                status: s.label.status,
            }),
        }
    }
    Some(Ruler {
        a,
        b,
        mode,
        stretches,
    })
}

/// From `p` on surface `shape`, along its normal (the side `towards` points
/// to), through the layers behind it: traced across the model's extent,
/// and cut where the line first enters a void cell (or leaves the model)
/// after it has crossed something else.
pub fn through_layers(
    m: &Model,
    shape: u32,
    p: Vec3,
    towards: Vec3,
    level: Option<usize>,
) -> Option<Ruler> {
    let mut n = m.surface_normal(shape, p)?;
    if llevant::vec3::dot(n, towards) < 0.0 {
        n = n.map(|x| -x);
    }
    let reach = {
        let b = m.bounds(llevant::Scope::Problem);
        let d = [0, 1, 2].map(|k| b.hi[k] - b.lo[k]);
        llevant::vec3::norm(d).max(1.0)
    };
    // Start a hair before the surface, so that the first cell is the one
    // behind it.
    let a = [0, 1, 2].map(|k| p[k] - 1e-6 * n[k]);
    let b = [0, 1, 2].map(|k| p[k] + reach * n[k]);
    let mut r = measure(m, a, b, RulerMode::Layers, level)?;
    let void = |s: &Stretch| match s.cell {
        Some(c) => {
            let cell = m.cell(c);
            cell.fill.is_none() && cell.material == 0
        }
        None => s.status != Status::Inside,
    };
    // Skip what is before the first non-void stretch (a sliver at the
    // surface), then stop at the next void.
    let first = r
        .stretches
        .iter()
        .position(|s| !void(s) && s.length() > 1e-9)?;
    let end = r.stretches[first..]
        .iter()
        .position(void)
        .map_or(r.stretches.len(), |k| first + k);
    let (from, to) = (r.stretches[first].from, r.stretches[end - 1].to);
    let (a, b) = (r.at(from), r.at(to));
    r.stretches = r.stretches[first..end]
        .iter()
        .filter(|s| s.length() > 0.0)
        .map(|s| Stretch {
            from: s.from - from,
            to: s.to - from,
            ..s.clone()
        })
        .collect();
    r.a = a;
    r.b = b;
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/mcnp-oracle/models/universes.i"
        );
        Model::load(path, &llevant::Options::default(), None).unwrap()
    }

    #[test]
    fn stretches_cover_the_line_and_name_its_cells() {
        let m = model();
        let b = m.bounds(llevant::Scope::Problem);
        let c = b.center();
        let a = [b.lo[0] + 1e-3, c[1], c[2]];
        let e = [b.hi[0] - 1e-3, c[1], c[2]];
        let r = measure(&m, a, e, RulerMode::TwoPoints, None).unwrap();
        assert!(!r.stretches.is_empty());
        assert!((r.stretches[0].from).abs() < 1e-9);
        assert!((r.stretches.last().unwrap().to - r.length()).abs() < 1e-6);
        for w in r.stretches.windows(2) {
            assert!((w[0].to - w[1].from).abs() < 1e-9, "contiguous");
            assert!(
                w[0].cell != w[1].cell || w[0].status != w[1].status,
                "merged"
            );
        }
        // Each stretch's cell is the one at its middle.
        for s in &r.stretches {
            if let Some(cell) = s.cell {
                let mid = r.at(0.5 * (s.from + s.to));
                assert_eq!(m.classify_point(mid).cell(), Some(cell));
            }
        }
        let csv = r.csv(&m);
        assert_eq!(csv.lines().count(), r.stretches.len() + 1);
    }
}
