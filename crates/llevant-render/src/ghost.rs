//! Ghost cells drawn over an image: a shell of the first ghost boundary
//! each ray crossed before its hit ([`llevant::Ghost`]), in the ghost
//! cell's colour, shaded by its normal, more opaque where the ray grazes it
//! (silhouettes) and with each further boundary behind it. The viewer's
//! `ghost.wgsl` draws the same.

use llevant::vec3::{dot, Vec3};
use llevant::{Camera, Ghost};
use rayon::prelude::*;

use crate::colours::{self, Paint, Rgb};

/// Opacity of one boundary seen face on.
pub const ALPHA: f32 = 0.16;
/// Opacity added where a ray grazes the boundary.
pub const RIM: f32 = 0.55;
/// Largest opacity of one boundary.
pub const MAX_ALPHA: f32 = 0.8;

/// Opacity of a ray's ghost boundaries: per boundary `ALPHA`, plus `RIM` as
/// the ray grazes the first one (`cos` of the angle between them, from the
/// normal), compounded over `count` boundaries.
pub fn opacity(count: u32, cos: f32) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let graze = 1.0 - cos.abs().min(1.0);
    let one = (ALPHA + RIM * graze * graze).min(MAX_ALPHA);
    1.0 - (1.0 - one).powi(count.min(64) as i32)
}

/// The colour over `under` where a ray of direction `dir` crossed `g`.
pub fn over(paint: &Paint, g: &Ghost, dir: Vec3, under: Rgb) -> Rgb {
    if g.count == 0 {
        return under;
    }
    let shell = colours::shade(paint.cell_colour(g.cell), g.normal, dir);
    let a = opacity(g.count, dot(g.normal, dir) as f32);
    [0, 1, 2].map(|i| (under[i] as f32 + (shell[i] as f32 - under[i] as f32) * a).round() as u8)
}

/// Draw `ghosts` (one per pixel of `camera`, rows from the top, as
/// [`llevant::Hits::ghosts`]) over the 8-bit RGB image `img`. Nothing
/// without ghosts.
pub fn overlay(paint: &Paint, ghosts: &[Ghost], camera: &Camera, img: &mut [u8]) {
    if ghosts.is_empty() {
        return;
    }
    assert_eq!(
        ghosts.len(),
        camera.width * camera.height,
        "one ghost record per pixel"
    );
    img.par_chunks_mut(3).enumerate().for_each(|(k, px)| {
        let g = &ghosts[k];
        if g.count == 0 {
            return;
        }
        let dir = camera.direction(k % camera.width, k / camera.width);
        px.copy_from_slice(&over(paint, g, dir, [px[0], px[1], px[2]]));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_grows_with_boundaries_and_grazing() {
        assert_eq!(opacity(0, 1.0), 0.0);
        assert!((opacity(1, 1.0) - ALPHA).abs() < 1e-6);
        assert!(opacity(2, 1.0) > opacity(1, 1.0));
        assert!(opacity(1, 0.1) > opacity(1, 0.9));
        assert!(opacity(1, 0.0) <= MAX_ALPHA + 1e-6);
        assert!(opacity(1000, 0.0) <= 1.0);
    }
}
