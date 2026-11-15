//! A geometry being edited, drawn over the 3D image (decision 0012,
//! phase 8) by one post-process pass ([`EditMarks`], `shaders/marks.wgsl`)
//! that traces nothing:
//!
//! - the marked surfaces: each written in the problem's frame (for the
//!   instance of its universe inspected) and evaluated at the point each
//!   pixel hit; a line where the side changes between neighbouring pixels
//!   on one face, a wash where the surface is the face seen;
//! - the cell as edited: its first face from a trace of the mini model
//!   alone ([`crate::marks`]), made off the UI thread at a reduced size
//!   ([`Shell`]), laid over the image as a translucent shell, stronger in
//!   front of what is seen, with its outline.
//!
//! Off (no cost) while nothing is marked.

use llevant::Camera;

use super::passes::PostPass;

/// Words before the surfaces in the pass's first buffer: the box of the
/// instance (`lo`, count, `hi`, whether bounded).
const HEAD: usize = 8;
/// Words per surface: its kind, then its numbers
/// ([`llevant::Model::shape_record`]).
pub const STRIDE: usize = 20;
/// Largest side of a shell image, in pixels.
pub const SHELL_SIDE: usize = 640;

/// The pass ([`PostPass::data_buffers`]: the surfaces, the shell).
pub struct EditMarks;

impl PostPass for EditMarks {
    fn name(&self) -> &str {
        "marks"
    }
    fn wgsl(&self) -> String {
        include_str!("shaders/marks.wgsl").into()
    }
    fn data_buffers(&self) -> u32 {
        2
    }
}

/// The surfaces marked, as the pass reads them: `records` (kind and
/// numbers, in the problem's frame), drawn only on points of `bounds` when
/// given (the instance's box).
pub fn surface_words(records: &[(u32, Vec<f64>)], bounds: Option<&llevant::Aabb>) -> Vec<f32> {
    let mut w = vec![0.0f32; HEAD + STRIDE * records.len()];
    if let Some(b) = bounds {
        w[..3].copy_from_slice(&b.lo.map(|x| x as f32));
        w[4..7].copy_from_slice(&b.hi.map(|x| x as f32));
        w[7] = 1.0;
    }
    w[3] = records.len() as f32;
    for (k, (kind, data)) in records.iter().enumerate() {
        let at = HEAD + STRIDE * k;
        w[at] = *kind as f32;
        for (i, x) in data.iter().take(STRIDE - 1).enumerate() {
            w[at + 1 + i] = *x as f32;
        }
    }
    w
}

/// The first face of a cell as edited, seen by `camera` (the view's, full
/// size): per pixel of a `width` × `height` image, the distance along the
/// ray (negative for none) and the cosine between the ray and the face.
#[derive(Clone, Debug)]
pub struct Shell {
    pub camera: Camera,
    pub width: usize,
    pub height: usize,
    pub words: Vec<f32>,
}

impl Shell {
    /// The size of a shell image for `camera`.
    pub fn size(camera: &Camera) -> (usize, usize) {
        let s = (SHELL_SIDE as f64 / camera.width.max(camera.height) as f64).min(1.0);
        let f = |n: usize| ((n as f64 * s).round() as usize).max(1);
        (f(camera.width), f(camera.height))
    }

    /// The first face of cell `cell` of `m` (a model of its own) seen by
    /// `camera` (the problem's frame, full size), its rays taken into the
    /// cell's frame by `frame`, at [`Shell::size`]; nothing seen through
    /// the camera's cut. `None` for no such cell or a degenerate camera.
    pub fn trace(
        m: &llevant::Model,
        cell: u32,
        camera: &Camera,
        frame: &llevant::Affine,
    ) -> Option<Shell> {
        let (w, h) = Shell::size(camera);
        let mut local = camera.clone();
        local.eye = frame.apply(camera.eye);
        local.target = frame.apply(camera.target);
        local.up = frame.apply_dir(camera.up);
        local.width = w;
        local.height = h;
        local.cut = llevant::Cutaway::none();
        let mut vis = llevant::Visibility::all(m);
        vis.cells.iter_mut().for_each(|v| *v = false);
        *vis.cells.get_mut(cell as usize)? = true;
        vis.gaps = false;
        vis.problems = false;
        let hits = m.render(&local, &vis).ok()?;
        Some(Shell::from_hits(camera.clone(), &hits, &local))
    }

    /// From the hits of a model of its own, traced with `local` (the
    /// camera moved into its frame at [`Shell::size`]).
    fn from_hits(camera: Camera, hits: &llevant::Hits, local: &Camera) -> Shell {
        let n = local.width * local.height;
        let mut words = vec![-1.0f32; 2 * n];
        for k in 0..n.min(hits.t.len()) {
            if hits.ids[k] == llevant::NO_HIT || !hits.t[k].is_finite() {
                continue;
            }
            let d = local.direction(k % local.width, k / local.width);
            let nrm = hits.normal[k];
            let c = (d[0] * nrm[0] + d[1] * nrm[1] + d[2] * nrm[2]).abs();
            words[2 * k] = hits.t[k] as f32;
            words[2 * k + 1] = c as f32;
        }
        Shell {
            camera,
            width: local.width,
            height: local.height,
            words,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surfaces_packed() {
        let b = llevant::Aabb {
            lo: [-1.0, -2.0, -3.0],
            hi: [1.0, 2.0, 3.0],
        };
        let w = surface_words(&[(0, vec![1.0, 0.0, 0.0, 5.0])], Some(&b));
        assert_eq!(w.len(), HEAD + STRIDE);
        assert_eq!(&w[..8], &[-1.0, -2.0, -3.0, 1.0, 1.0, 2.0, 3.0, 1.0]);
        assert_eq!(&w[8..13], &[0.0, 1.0, 0.0, 0.0, 5.0]);
        let w = surface_words(&[], None);
        assert_eq!((w[3], w[7]), (0.0, 0.0));
    }
}
