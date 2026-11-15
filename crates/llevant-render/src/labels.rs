//! Where to write cell and surface numbers on a slice, as MCNP's plotter
//! labels its plots: each region's cell number at the point of the region
//! farthest from its border, and each surface's number on the border it
//! makes, once per border between two regions.
//!
//! The input is a traced slice's pixels ([`Grid`]): a key per pixel (equal
//! keys, same label) and the cell there. Cells need nothing more. Surfaces
//! are found by tracing short lines across borders, sampled every
//! [`Options::spacing`] pixels along each, all in one batch.

use llevant::Depth;
use llevant::{Line, Model, Slice, NO_SURFACE};
use rayon::prelude::*;

/// A traced slice's pixels, rows from the top.
pub struct Grid<'a> {
    pub nx: usize,
    pub ny: usize,
    /// Per pixel: equal keys, same label (0: blank, never labelled).
    pub keys: &'a [u64],
    /// Per pixel: the cell there (index into [`Model::cells`]), or
    /// [`NONE`] where no single cell holds it.
    pub cells: &'a [u32],
}

/// No cell (in [`Grid::cells`]).
pub const NONE: u32 = u32::MAX;

/// What a label names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A cell (index into [`Model::cells`]).
    Cell(u32),
    /// A compiled surface (index into [`Model::shapes`]).
    Surface(u32),
}

/// A label's place, in pixels from the top left corner (pixel centres at
/// `+0.5`), and how much it matters: the region's depth (pixels to its
/// border) for cells, the border's length for surfaces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub x: f32,
    pub y: f32,
    pub kind: Kind,
    pub weight: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Label cells whose region reaches this many pixels from its border.
    pub min_depth: f32,
    /// Label borders at least this many pixels long; samples along a
    /// border this many pixels apart.
    pub spacing: usize,
    /// Universe level shown (as traced), for the surfaces met.
    pub level: Option<usize>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            min_depth: 6.0,
            spacing: 24,
            level: None,
        }
    }
}

/// Chamfer distance (3-4, divided by 3: about pixels) from each pixel to
/// the nearest pixel of another key or the image's edge.
fn depths(g: &Grid) -> Vec<f32> {
    let (nx, ny) = (g.nx, g.ny);
    let mut d = vec![f32::MAX; nx * ny];
    let at = |i: usize, j: usize| j * nx + i;
    // Pixels on a border (or the edge) start at 1.
    for j in 0..ny {
        for i in 0..nx {
            let k = g.keys[at(i, j)];
            let edge = i == 0
                || j == 0
                || i + 1 == nx
                || j + 1 == ny
                || g.keys[at(i - 1, j)] != k
                || g.keys[at(i + 1, j)] != k
                || g.keys[at(i, j - 1)] != k
                || g.keys[at(i, j + 1)] != k;
            if edge {
                d[at(i, j)] = 1.0;
            }
        }
    }
    let (a, b) = (1.0f32, 4.0 / 3.0);
    for j in 0..ny {
        for i in 0..nx {
            let mut v = d[at(i, j)];
            if i > 0 {
                v = v.min(d[at(i - 1, j)] + a);
            }
            if j > 0 {
                v = v.min(d[at(i, j - 1)] + a);
                if i > 0 {
                    v = v.min(d[at(i - 1, j - 1)] + b);
                }
                if i + 1 < nx {
                    v = v.min(d[at(i + 1, j - 1)] + b);
                }
            }
            d[at(i, j)] = v;
        }
    }
    for j in (0..ny).rev() {
        for i in (0..nx).rev() {
            let mut v = d[at(i, j)];
            if i + 1 < nx {
                v = v.min(d[at(i + 1, j)] + a);
            }
            if j + 1 < ny {
                v = v.min(d[at(i, j + 1)] + a);
                if i + 1 < nx {
                    v = v.min(d[at(i + 1, j + 1)] + b);
                }
                if i > 0 {
                    v = v.min(d[at(i - 1, j + 1)] + b);
                }
            }
            d[at(i, j)] = v;
        }
    }
    d
}

/// One label per connected region of a cell: at its deepest pixel, if that
/// is at least `min_depth` from the border. Deepest first.
pub fn cell_anchors(g: &Grid, min_depth: f32) -> Vec<Anchor> {
    let (nx, ny) = (g.nx, g.ny);
    if nx == 0 || ny == 0 {
        return Vec::new();
    }
    let d = depths(g);
    let mut seen = vec![false; nx * ny];
    let mut stack = Vec::new();
    let mut out = Vec::new();
    for start in 0..nx * ny {
        if seen[start] {
            continue;
        }
        let key = g.keys[start];
        seen[start] = true;
        stack.push(start);
        let mut best = start;
        while let Some(p) = stack.pop() {
            if d[p] > d[best] {
                best = p;
            }
            let (i, j) = (p % nx, p / nx);
            let mut visit = |q: usize| {
                if !seen[q] && g.keys[q] == key {
                    seen[q] = true;
                    stack.push(q);
                }
            };
            if i > 0 {
                visit(p - 1);
            }
            if i + 1 < nx {
                visit(p + 1);
            }
            if j > 0 {
                visit(p - nx);
            }
            if j + 1 < ny {
                visit(p + nx);
            }
        }
        let cell = g.cells[best];
        if key != 0 && cell != NONE && d[best] >= min_depth {
            out.push(Anchor {
                x: (best % nx) as f32 + 0.5,
                y: (best / nx) as f32 + 0.5,
                kind: Kind::Cell(cell),
                weight: d[best],
            });
        }
    }
    out.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    out
}

/// A border pixel pair: the pixel and its right (`down` false) or lower
/// neighbour, of another key.
#[derive(Clone, Copy)]
struct Edge {
    p: usize,
    down: bool,
}

/// Labels of the surfaces along the borders between regions: borders are
/// grouped by the pair of keys they separate and by connection, sampled
/// every `spacing` pixels, and each sample traced across; each surface met
/// in a group is labelled once, at the middle of its samples. Longest
/// first.
pub fn surface_anchors(m: &Model, slice: &Slice, g: &Grid, o: &Options) -> Vec<Anchor> {
    surface_anchors_with(m, g, o, &|i, j| slice.pixel_centre(i, j))
}

/// [`surface_anchors`] with the model point of each pixel's centre given.
fn surface_anchors_with(
    m: &Model,
    g: &Grid,
    o: &Options,
    centre: &(dyn Fn(usize, usize) -> llevant::Vec3 + Sync),
) -> Vec<Anchor> {
    let (nx, ny) = (g.nx, g.ny);
    if nx < 2 || ny < 2 {
        return Vec::new();
    }
    let at = |i: usize, j: usize| j * nx + i;
    // Border edges, and per pixel the index of its edges (right, down).
    let mut edges: Vec<Edge> = Vec::new();
    let mut edge_at = vec![[u32::MAX; 2]; nx * ny];
    for j in 0..ny {
        for i in 0..nx {
            let p = at(i, j);
            let k = g.keys[p];
            for (n, down) in [(i + 1 < nx, false), (j + 1 < ny, true)] {
                if !n {
                    continue;
                }
                let q = if down { p + nx } else { p + 1 };
                let kq = g.keys[q];
                // Blank on both sides is no border; blank on one side is
                // the shown level's edge.
                if kq != k && (k != 0 || kq != 0) {
                    edge_at[p][down as usize] = edges.len() as u32;
                    edges.push(Edge { p, down });
                }
            }
        }
    }
    let pair = |e: &Edge| {
        let q = if e.down { e.p + nx } else { e.p + 1 };
        let (a, b) = (g.keys[e.p], g.keys[q]);
        (a.min(b), a.max(b))
    };
    // Groups: edges of one key pair, connected through edges whose pixels
    // are 8-neighbours.
    let mut group = vec![u32::MAX; edges.len()];
    let mut groups: Vec<Vec<u32>> = Vec::new();
    let mut stack = Vec::new();
    for s in 0..edges.len() {
        if group[s] != u32::MAX {
            continue;
        }
        let g_id = groups.len() as u32;
        let want = pair(&edges[s]);
        let mut members = Vec::new();
        group[s] = g_id;
        stack.push(s as u32);
        while let Some(e) = stack.pop() {
            members.push(e);
            let p = edges[e as usize].p;
            let (i, j) = ((p % nx) as isize, (p / nx) as isize);
            for dj in -1..=1isize {
                for di in -1..=1isize {
                    let (x, y) = (i + di, j + dj);
                    if x < 0 || y < 0 || x as usize >= nx || y as usize >= ny {
                        continue;
                    }
                    for &f in &edge_at[at(x as usize, y as usize)] {
                        if f != u32::MAX
                            && group[f as usize] == u32::MAX
                            && pair(&edges[f as usize]) == want
                        {
                            group[f as usize] = g_id;
                            stack.push(f);
                        }
                    }
                }
            }
        }
        groups.push(members);
    }
    // Samples: every `spacing`-th edge of each long enough group (in the
    // order the fill met them, which follows the border), the middle one
    // of a short group.
    let spacing = o.spacing.max(1);
    let mut samples: Vec<(u32, u32)> = Vec::new(); // (group, edge)
    for (k, members) in groups.iter().enumerate() {
        if members.len() < spacing {
            continue;
        }
        let first = (members.len() % spacing) / 2;
        for &e in members.iter().skip(first).step_by(spacing) {
            samples.push((k as u32, e));
        }
    }
    // A line from each sample's pixel centre to its neighbour's.
    let lines: Vec<Line> = samples
        .iter()
        .map(|&(_, e)| {
            let e = edges[e as usize];
            let (i, j) = (e.p % nx, e.p / nx);
            let (a, b) = if e.down {
                (centre(i, j), centre(i, j + 1))
            } else {
                (centre(i, j), centre(i + 1, j))
            };
            Line::between(a, b)
        })
        .collect();
    let Ok(tr) = m.trace(&lines, Depth::from(o.level)) else {
        return Vec::new();
    };
    // The surface each sample crosses: the first crossing inside the line,
    // named as a pick names it (the coincident surface that bounds there).
    let mut found: Vec<(u32, u32, usize)> = (0..samples.len())
        .into_par_iter()
        .filter_map(|n| {
            let r = tr.segments(n);
            let x = r
                .clone()
                .take(r.len().saturating_sub(1))
                .find(|&x| tr.surface_out[x] != NO_SURFACE)?;
            let line = &lines[n];
            let s = m.bounding_surface(tr.surface_out[x], line.at(tr.t1[x]), line.dir);
            Some((samples[n].0, s, n))
        })
        .collect();
    found.sort_unstable();
    let mut out = Vec::new();
    for run in found.chunk_by(|a, b| a.0 == b.0 && a.1 == b.1) {
        let (k, s, _) = run[0];
        let mid = run[run.len() / 2].2;
        let e = edges[samples[mid].1 as usize];
        let (i, j) = ((e.p % nx) as f32, (e.p / nx) as f32);
        let (x, y) = if e.down {
            (i + 0.5, j + 1.0)
        } else {
            (i + 1.0, j + 0.5)
        };
        out.push(Anchor {
            x,
            y,
            kind: Kind::Surface(s),
            weight: (run.len() * spacing) as f32 + groups[k as usize].len() as f32 * 1e-6,
        });
    }
    out.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    out
}

/// Samples of an image's pixels that labels look at, at most (more are
/// decimated: labels need no pixel's precision).
pub const MAX_SAMPLES: usize = 300_000;

/// Cell anchors (if `cells`) then surface anchors (if `surfaces`) of a
/// traced slice, found on every `stride`-th pixel of each row and column so
/// that at most [`MAX_SAMPLES`] are looked at; positions and weights in the
/// slice's own pixels.
pub fn anchors(
    m: &Model,
    slice: &Slice,
    g: &Grid,
    cells: bool,
    surfaces: bool,
    o: &Options,
) -> Vec<Anchor> {
    let n = g.nx * g.ny;
    let stride = ((n as f64 / MAX_SAMPLES as f64).sqrt().ceil() as usize).max(1);
    if stride == 1 {
        let mut out = Vec::new();
        if cells {
            out.extend(cell_anchors(g, o.min_depth));
        }
        if surfaces {
            out.extend(surface_anchors(m, slice, g, o));
        }
        return out;
    }
    // Sample (i, j) is pixel (i·stride + off, j·stride + off).
    let off = stride / 2;
    let (nx, ny) = ((g.nx - off).div_ceil(stride), (g.ny - off).div_ceil(stride));
    let mut keys = Vec::with_capacity(nx * ny);
    let mut cs = Vec::with_capacity(nx * ny);
    for j in 0..ny {
        let row = (j * stride + off) * g.nx;
        for i in 0..nx {
            keys.push(g.keys[row + i * stride + off]);
            cs.push(g.cells[row + i * stride + off]);
        }
    }
    let d = Grid {
        nx,
        ny,
        keys: &keys,
        cells: &cs,
    };
    let s = stride as f32;
    let o2 = Options {
        min_depth: o.min_depth / s,
        spacing: (o.spacing / stride).max(1),
        level: o.level,
    };
    let mut out = Vec::new();
    if cells {
        out.extend(cell_anchors(&d, o2.min_depth));
    }
    if surfaces {
        out.extend(surface_anchors_with(m, &d, &o2, &|i, j| {
            slice.pixel_centre(i * stride + off, j * stride + off)
        }));
    }
    // Back to the image's pixels: sample centre `i + 0.5` is pixel centre
    // `i·stride + off + 0.5`.
    for a in &mut out {
        a.x = (a.x - 0.5) * s + off as f32 + 0.5;
        a.y = (a.y - 0.5) * s + off as f32 + 0.5;
        a.weight *= s;
    }
    out
}

/// The anchors that fit: greedily, in order, each kept unless its box
/// (centred on it, `size(k, anchor)` wide and high, in pixels) overlaps
/// one kept before; at most `max`.
pub fn place(
    anchors: &[Anchor],
    max: usize,
    size: impl Fn(usize, &Anchor) -> (f32, f32),
) -> Vec<usize> {
    let mut kept: Vec<(usize, [f32; 4])> = Vec::new();
    for (k, a) in anchors.iter().enumerate() {
        if kept.len() >= max {
            break;
        }
        let (w, h) = size(k, a);
        let b = [a.x - 0.5 * w, a.y - 0.5 * h, a.x + 0.5 * w, a.y + 0.5 * h];
        let overlaps = kept
            .iter()
            .any(|(_, c)| b[0] < c[2] && c[0] < b[2] && b[1] < c[3] && c[1] < b[3]);
        if !overlaps {
            kept.push((k, b));
        }
    }
    kept.into_iter().map(|(k, _)| k).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use llevant::Options as ModelOptions;

    /// Two boxes side by side in a void world.
    const MODEL: &str = "labels
1 0 -1 imp:n=1
2 0 -2 imp:n=1
3 0 1 2 -9 imp:n=1
9 0 9 imp:n=0

1 rpp -10 0 -5 5 -5 5
2 rpp 0 10 -5 5 -5 5
9 so 100

";

    fn traced(m: &Model, s: &Slice) -> (Vec<u64>, Vec<u32>) {
        let tr = m.slice(s, Depth::All).unwrap();
        let (mut keys, mut cells) = (Vec::new(), Vec::new());
        for row in 0..s.ny {
            let r = tr.segments(row);
            let mut k = r.start;
            for i in 0..s.nx {
                let t = s.width * (i as f64 + 0.5) / s.nx as f64;
                while k + 1 < r.end && tr.t1[k] <= t {
                    k += 1;
                }
                let id = tr.ids[k];
                keys.push(id as u64 + 1);
                cells.push(tr.labels[id as usize].cell().unwrap_or(NONE));
            }
        }
        (keys, cells)
    }

    #[test]
    fn cells_and_surfaces_of_two_boxes() {
        let m = Model::from_text(MODEL, &ModelOptions::default());
        assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
        let s = Slice {
            origin: [-15.0, -10.0, 0.1],
            u: [1.0, 0.0, 0.0],
            v: [0.0, 1.0, 0.0],
            width: 30.0,
            height: 20.0,
            nx: 150,
            ny: 100,
        };
        let (keys, cells) = traced(&m, &s);
        let g = Grid {
            nx: s.nx,
            ny: s.ny,
            keys: &keys,
            cells: &cells,
        };
        let c = cell_anchors(&g, 6.0);
        let ids: Vec<i64> = c
            .iter()
            .map(|a| match a.kind {
                Kind::Cell(c) => m.cell_id(c),
                _ => unreachable!(),
            })
            .collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(sorted, [1, 2, 3], "{c:?}");
        // Each box's label lies inside it (5 px per cm).
        for a in &c {
            let x = -15.0 + a.x as f64 / 5.0;
            match a.kind {
                Kind::Cell(k) if m.cell_id(k) == 1 => assert!((-10.0..0.0).contains(&x), "{a:?}"),
                Kind::Cell(k) if m.cell_id(k) == 2 => assert!((0.0..10.0).contains(&x), "{a:?}"),
                _ => {}
            }
        }

        let o = Options::default();
        let sa = surface_anchors(&m, &s, &g, &o);
        let mut named: Vec<(i64, u8)> = sa
            .iter()
            .map(|a| match a.kind {
                Kind::Surface(k) => {
                    let id = m.surface_id(k).unwrap();
                    (id.id, id.facet)
                }
                _ => unreachable!(),
            })
            .collect();
        named.sort();
        named.dedup();
        // The shared face is facet 1 of box 1 or facet 2 of box 2 (they
        // coincide); every outer facet in the plane is named.
        for want in [(1, 2), (1, 3), (1, 4), (2, 1), (2, 3), (2, 4)] {
            assert!(named.contains(&want), "{want:?} not in {named:?}");
        }
        assert!(
            named.contains(&(1, 1)) || named.contains(&(2, 2)),
            "{named:?}"
        );
        // Every anchor lies on a border.
        for a in &sa {
            let (i, j) = (a.x as usize, a.y as usize);
            let k = keys[j.min(s.ny - 1) * s.nx + i.min(s.nx - 1)];
            let near = [(0isize, 0isize), (-1, 0), (0, -1), (-1, -1)]
                .iter()
                .any(|&(di, dj)| {
                    let (x, y) = (i as isize + di, j as isize + dj);
                    x >= 0 && y >= 0 && keys[y as usize * s.nx + x as usize] != k
                });
            assert!(near, "{a:?}");
        }
    }

    #[test]
    fn placement_drops_overlaps() {
        let a = |x: f32| Anchor {
            x,
            y: 0.0,
            kind: Kind::Cell(0),
            weight: 1.0,
        };
        let v = [a(0.0), a(5.0), a(20.0), a(40.0)];
        assert_eq!(place(&v, 10, |_, _| (10.0, 4.0)), [0, 2, 3]);
        assert_eq!(place(&v, 2, |_, _| (10.0, 4.0)), [0, 2]);
    }
}
