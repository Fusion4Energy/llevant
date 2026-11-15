//! What is drawn of a mesh (in Tally): in the slice, its voxels as an image
//! of their boundaries, each pixel located in the mesh (exact for any `TR=`,
//! cylinder and plane, radial ones included); in 3D, its wireframe; and
//! where its handles are.

use aci_tallies_geom::mcnp_input::coarse_bins;
use aci_tallies_geom::Geometry;
use llevant::vec3::{add, dot, norm, normalize, scale, sub};
use llevant::Vec3;
use rayon::prelude::*;

use super::{MeshShape, MeshState};
use crate::slice::PlaneView;

/// How much of an axis's bins the slice's image shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    Fine,
    /// The fine bins are too small to see: their coarse bins.
    Coarse,
    /// The coarse bins too.
    Hidden,
}

/// The slice's image of a mesh.
pub struct Raster {
    pub image: egui::ColorImage,
    pub detail: [Detail; 3],
}

/// Pixels outside the mesh.
const OUT: u32 = u32::MAX;

/// Above this share of neighbouring pixels in different bins, an axis's
/// bins are too small to draw.
const DENSE: f64 = 0.2;

/// The image (`nx` by `ny` pixels over `rect`) of mesh `g` cut by `plane`:
/// its outline, the boundaries between its voxels and a faint fill, in
/// `ink`.
pub fn raster(
    g: &Geometry,
    plane: &PlaneView,
    rect: egui::Rect,
    nx: usize,
    ny: usize,
    ink: egui::Color32,
) -> Raster {
    let (sx, sy) = (rect.width() / nx as f32, rect.height() / ny as f32);
    let rows: Vec<Vec<[u32; 3]>> = (0..ny)
        .into_par_iter()
        .map(|iy| {
            (0..nx)
                .map(|ix| {
                    let pos = rect.min + egui::vec2((ix as f32 + 0.5) * sx, (iy as f32 + 0.5) * sy);
                    match g.locate(plane.from_screen(rect, pos)) {
                        Some(v) => {
                            let (i, j, k) = g.ijk(v);
                            [i as u32, j as u32, k as u32]
                        }
                        None => [OUT; 3],
                    }
                })
                .collect()
        })
        .collect();
    let mut ids: Vec<[u32; 3]> = rows.concat();

    // Coarse bins where the fine ones are too small, nothing where those
    // are too.
    let (a, b, c) = g.axes();
    let mut detail = [Detail::Fine; 3];
    for (k, fine) in [a, b, c].into_iter().enumerate() {
        if density(&ids, nx, ny, k) <= DENSE {
            continue;
        }
        let (_, _, counts) = coarse_bins(fine);
        let coarse_of: Vec<u32> = counts
            .iter()
            .enumerate()
            .flat_map(|(n, &c)| std::iter::repeat_n(n as u32, c as usize))
            .collect();
        for id in ids.iter_mut().filter(|id| id[k] != OUT) {
            id[k] = coarse_of.get(id[k] as usize).copied().unwrap_or(0);
        }
        detail[k] = if density(&ids, nx, ny, k) <= DENSE {
            Detail::Coarse
        } else {
            Detail::Hidden
        };
    }
    let shown: Vec<usize> = (0..3).filter(|&k| detail[k] != Detail::Hidden).collect();
    let differ = |p: &[u32; 3], q: &[u32; 3]| shown.iter().any(|&k| p[k] != q[k]);

    let (edge, line, fill) = (ink, ink.gamma_multiply(0.7), ink.gamma_multiply(0.08));
    let mut pixels = vec![egui::Color32::TRANSPARENT; nx * ny];
    for iy in 0..ny {
        for ix in 0..nx {
            let p = &ids[iy * nx + ix];
            if p[0] == OUT {
                continue;
            }
            let at = |dx: isize, dy: isize| -> Option<&[u32; 3]> {
                let (x, y) = (ix as isize + dx, iy as isize + dy);
                (x >= 0 && y >= 0 && (x as usize) < nx && (y as usize) < ny)
                    .then(|| &ids[y as usize * nx + x as usize])
            };
            let near = [at(1, 0), at(-1, 0), at(0, 1), at(0, -1)];
            pixels[iy * nx + ix] = if near.iter().any(|q| q.is_none_or(|q| q[0] == OUT)) {
                edge
            } else if [near[0], near[2]].iter().flatten().any(|q| differ(p, q)) {
                line
            } else {
                fill
            };
        }
    }
    Raster {
        image: egui::ColorImage::new([nx, ny], pixels),
        detail,
    }
}

/// The share of neighbouring pixels in the mesh whose bins along axis `k`
/// differ.
fn density(ids: &[[u32; 3]], nx: usize, ny: usize, k: usize) -> f64 {
    let (mut pairs, mut different) = (0u64, 0u64);
    for iy in 0..ny {
        for ix in 0..nx {
            let p = &ids[iy * nx + ix];
            if p[0] == OUT {
                continue;
            }
            for q in [
                (ix + 1 < nx).then(|| &ids[iy * nx + ix + 1]),
                (iy + 1 < ny).then(|| &ids[(iy + 1) * nx + ix]),
            ]
            .into_iter()
            .flatten()
            {
                if q[0] != OUT {
                    pairs += 1;
                    different += (q[k] != p[k]) as u64;
                }
            }
        }
    }
    if pairs == 0 {
        0.0
    } else {
        different as f64 / pairs as f64
    }
}

/// Most lines drawn across a face per axis: more bins than this are
/// drawn every so many.
const MOST_LINES: usize = 64;

/// The bounds of `v` drawn: the first and last, and inner ones (every so
/// many when there are many).
fn drawn(v: &[f64]) -> Vec<f64> {
    let n = v.len().saturating_sub(1);
    let stride = n.div_ceil(MOST_LINES).max(1);
    v.iter()
        .enumerate()
        .filter(|&(i, _)| i % stride == 0 || i == n)
        .map(|(_, &x)| x)
        .collect()
}

/// Segments per revolution of a circle.
const ROUND: usize = 64;

/// The mesh's wireframe in the world: polylines, each with whether it is
/// part of the outline (rather than a bin's boundary).
pub fn wireframe(m: &MeshState) -> Vec<(Vec<Vec3>, bool)> {
    let mut out = Vec::new();
    match &m.shape {
        MeshShape::Rect { x, y, z } => {
            let t = m.placement();
            let ax = [x, y, z];
            let ends = ax.map(|v| [v[0], *v.last().unwrap_or(&v[0])]);
            for d in 0..3 {
                let (e, f) = ((d + 1) % 3, (d + 2) % 3);
                // The four edges along d.
                for se in ends[e] {
                    for sf in ends[f] {
                        let mut p = [0.0; 3];
                        p[e] = se;
                        p[f] = sf;
                        let mut q = p;
                        p[d] = ends[d][0];
                        q[d] = ends[d][1];
                        out.push((vec![t.to_world(p), t.to_world(q)], true));
                    }
                }
                // On the faces normal to e: lines across them along f, at
                // the inner bounds of d.
                let inner = drawn(ax[d]);
                for side in ends[e] {
                    for &b in &inner[1..inner.len().saturating_sub(1)] {
                        let mut p = [0.0; 3];
                        p[d] = b;
                        p[e] = side;
                        let mut q = p;
                        p[f] = ends[f][0];
                        q[f] = ends[f][1];
                        out.push((vec![t.to_world(p), t.to_world(q)], false));
                    }
                }
                for side in ends[f] {
                    for &b in &inner[1..inner.len().saturating_sub(1)] {
                        let mut p = [0.0; 3];
                        p[d] = b;
                        p[f] = side;
                        let mut q = p;
                        p[e] = ends[e][0];
                        q[e] = ends[e][1];
                        out.push((vec![t.to_world(p), t.to_world(q)], false));
                    }
                }
            }
        }
        MeshShape::Cyl { r, z, theta, .. } => {
            let Some(f) = m.cyl_frame() else {
                return out;
            };
            let (r1, z1) = (*r.last().unwrap_or(&0.0), *z.last().unwrap_or(&0.0));
            let circle = |rad: f64, h: f64| -> Vec<Vec3> {
                (0..=ROUND)
                    .map(|k| f.to_world(rad, h, k as f64 / ROUND as f64))
                    .collect()
            };
            // The rims, and the outer surface at each z bound.
            for h in drawn(z) {
                out.push((circle(r1, h), h == 0.0 || h == z1));
            }
            // The caps: a circle at each r bound, spokes at each theta
            // bound; and lines along the outer surface there.
            for h in [0.0, z1] {
                for &rad in drawn(r).iter().filter(|&&x| x > 0.0 && x < r1) {
                    out.push((circle(rad, h), false));
                }
            }
            let spokes = if theta.len() > 2 {
                drawn(theta)
            } else {
                vec![0.0]
            };
            for &th in spokes.iter().filter(|&&th| th < 1.0) {
                let outline = theta.len() <= 2;
                for h in [0.0, z1] {
                    out.push((
                        vec![f.to_world(r[0], h, th), f.to_world(r1, h, th)],
                        outline,
                    ));
                }
                out.push((
                    vec![f.to_world(r1, 0.0, th), f.to_world(r1, z1, th)],
                    outline,
                ));
            }
        }
    }
    out
}

/// What a handle does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grip {
    /// Moves the first (`false`) or last bound of axis `k` (a box's x, y,
    /// z; a cylinder's r or z).
    End(usize, bool),
    /// Moves the whole mesh.
    Move,
    /// Turns it about the world axis `k` (the rings, in 3D).
    Ring(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Handle {
    pub grip: Grip,
    /// Where it is drawn (for a ring, its centre).
    pub at: Vec3,
    /// The world direction it moves along (unit; for a ring, its axis).
    pub dir: Vec3,
}

/// The handles of `m`. In the slice (`plane`: its `u`, `v`, `n`), those
/// moving along the plane's normal are left out, and a cylinder's radius
/// handle lies in the plane.
pub fn handles(m: &MeshState, plane: Option<[Vec3; 3]>) -> Vec<Handle> {
    let mut out = Vec::new();
    let across = |dir: Vec3| plane.is_some_and(|[_, _, n]| dot(dir, n).abs() > 0.9);
    match &m.shape {
        MeshShape::Rect { x, y, z } => {
            let t = m.placement();
            let ax = [x, y, z];
            let c = ax.map(|v| 0.5 * (v[0] + v.last().unwrap_or(&v[0])));
            for k in 0..3 {
                let dir = normalize(t.rotate(unit(k)));
                if across(dir) {
                    continue;
                }
                for high in [false, true] {
                    let mut p = c;
                    p[k] = if high {
                        *ax[k].last().unwrap_or(&ax[k][0])
                    } else {
                        ax[k][0]
                    };
                    out.push(Handle {
                        grip: Grip::End(k, high),
                        at: t.to_world(p),
                        dir,
                    });
                }
            }
        }
        MeshShape::Cyl { r, z, .. } => {
            let Some(f) = m.cyl_frame() else {
                return out;
            };
            let (r1, z1) = (*r.last().unwrap_or(&0.0), *z.last().unwrap_or(&0.0));
            let mid = add(f.origin, scale(f.e_axis, 0.5 * z1));
            // Out along the plane where there is one.
            let radial = plane
                .and_then(|[u, v, _]| {
                    [u, v]
                        .into_iter()
                        .map(|w| sub(w, scale(f.e_axis, dot(w, f.e_axis))))
                        .filter(|w| norm(*w) > 0.1)
                        .max_by(|a, b| norm(*a).total_cmp(&norm(*b)))
                })
                .map(normalize)
                .unwrap_or(f.e_ref);
            out.push(Handle {
                grip: Grip::End(0, true),
                at: add(mid, scale(radial, r1)),
                dir: radial,
            });
            if !across(f.e_axis) {
                for (high, h) in [(false, 0.0), (true, z1)] {
                    out.push(Handle {
                        grip: Grip::End(1, high),
                        at: add(f.origin, scale(f.e_axis, h)),
                        dir: f.e_axis,
                    });
                }
            }
        }
    }
    out.push(Handle {
        grip: Grip::Move,
        at: m.centre(),
        dir: [0.0; 3],
    });
    if plane.is_none() {
        for k in 0..3 {
            out.push(Handle {
                grip: Grip::Ring(k),
                at: m.centre(),
                dir: unit(k),
            });
        }
    }
    out
}

fn unit(k: usize) -> Vec3 {
    let mut e = [0.0; 3];
    e[k] = 1.0;
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slice::Frame;
    use crate::state::AxisName;

    #[test]
    fn the_image_marks_voxel_boundaries_where_the_mesh_has_them() {
        // 4 bins of 1 cm along x and y, the slice z = 0.5 showing x 0..8, y
        // 0..8 on 80 by 80 points: boundaries at every 10th column.
        let m = MeshState::rect(4, [0.0; 3], [4.0, 4.0, 1.0]);
        let mut m = m;
        m.axes_mut()[0].clone_from(&super::super::even(0.0, 4.0, 4));
        m.axes_mut()[1].clone_from(&super::super::even(0.0, 4.0, 4));
        let g = m.geometry().unwrap();
        let plane = PlaneView {
            frame: Frame::axis(AxisName::Z, 0.5),
            centre: [4.0, 4.0],
            width: 8.0,
            height: 8.0,
        };
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(80.0, 80.0));
        let r = raster(&g, &plane, rect, 80, 80, egui::Color32::WHITE);
        assert_eq!(r.detail, [Detail::Fine; 3]);
        let px = |x: usize, y: usize| r.image.pixels[y * 80 + x];
        // Row 75 (y = 0.45 cm from the bottom, inside): pixels left of x =
        // 1, 2, 3 cm are lines, others fill; beyond x = 4 nothing.
        let row = 75;
        assert_eq!(px(5, row), egui::Color32::WHITE.gamma_multiply(0.08));
        assert_eq!(px(9, row), egui::Color32::WHITE.gamma_multiply(0.7));
        assert_eq!(px(19, row), egui::Color32::WHITE.gamma_multiply(0.7));
        assert_eq!(px(39, row), egui::Color32::WHITE, "the outline");
        assert_eq!(px(45, row), egui::Color32::TRANSPARENT);
        // Each pixel is where the mesh locates it.
        for (x, y) in [(3, 3), (41, 77), (79, 0), (12, 60)] {
            let p = plane.from_screen(rect, egui::pos2(x as f32 + 0.5, y as f32 + 0.5));
            assert_eq!(
                g.locate(p).is_some(),
                r.image.pixels[y * 80 + x] != egui::Color32::TRANSPARENT
            );
        }
    }

    #[test]
    fn small_bins_fall_back_to_coarse_ones() {
        let mut m = MeshState::rect(4, [0.0; 3], [4.0, 4.0, 1.0]);
        // x: 2 coarse bins of 200 fine ones each.
        m.axes_mut()[0].clone_from(&aci_tallies_geom::mcnp_input::fine_bounds(
            0.0,
            &[2.0, 4.0],
            &[200, 200],
        ));
        let g = m.geometry().unwrap();
        let plane = PlaneView {
            frame: Frame::axis(AxisName::Z, 0.5),
            centre: [2.0, 2.0],
            width: 4.0,
            height: 4.0,
        };
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(40.0, 40.0));
        let r = raster(&g, &plane, rect, 40, 40, egui::Color32::WHITE);
        assert_eq!(r.detail[0], Detail::Coarse);
        assert_eq!(r.detail[1], Detail::Fine);
    }

    #[test]
    fn handles_in_the_slice_move_within_it() {
        let m = MeshState::rect(4, [0.0; 3], [4.0, 4.0, 1.0]);
        let z = Frame::axis(AxisName::Z, 0.5);
        let h = handles(&m, Some([z.u, z.v, z.n]));
        assert!(h
            .iter()
            .all(|h| !matches!(h.grip, Grip::End(2, _) | Grip::Ring(_))));
        assert_eq!(h.len(), 5);
        assert_eq!(handles(&m, None).len(), 6 + 1 + 3);
        let c = MeshState::cylinder(4, [0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], 2.0, 3.0);
        let y = Frame::axis(AxisName::Y, 0.0);
        let r = handles(&c, Some([y.u, y.v, y.n]))
            .into_iter()
            .find(|h| h.grip == Grip::End(0, true))
            .unwrap();
        assert!(
            dot(r.dir, y.n).abs() < 1e-12,
            "the radius handle is in the plane"
        );
        assert!(!wireframe(&c).is_empty() && wireframe(&m).len() >= 12);
    }
}
