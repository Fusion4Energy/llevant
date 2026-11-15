//! Shading of render images. Colours by cell, material or universe are
//! flat: each colour darkened by the angle between ray and surface.
//! Realistic colours ([`ColourBy::Realistic`](crate::colours::ColourBy::Realistic)) are physically based:
//! Cook-Torrance (GGX) specular and Lambert diffuse under a studio rig (a
//! key and a fill light that move with the camera, a sky-to-ground
//! gradient around it), each cell's roughness and metalness from its
//! material ([`Looks`](llevant_scene::materials::Looks)), lit in linear light, tone-mapped (ACES)
//! and encoded as sRGB. Clear materials (water) are a glossy surface over
//! a coloured depth.
//!
//! The shading costs nothing next to tracing: it uses only each pixel's
//! hit, and the viewer runs the same formulas on the GPU
//! (`llevant-viewer/src/view3d/shaders/colour.wgsl`, kept in step with this
//! file, in `f32` like it).

use llevant::vec3::{scale, Vec3};
use llevant::{Camera, Hits, Model, Result, Status, NO_HIT};
use rayon::prelude::*;

use crate::colours::{self, Paint, Rgb};
use llevant_scene::materials::{srgb_to_linear, Appearance, Category};

/// What a pixel's (or any) ray hit first, whoever traced it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Probe {
    /// Distance along the ray (infinite for a miss).
    pub t: f64,
    pub what: Found,
    /// The instance of the cell found ([`llevant::Instances`]), when known.
    pub instance: Option<u64>,
    /// Unit normal facing the ray, zero when the ray started inside.
    pub normal: Vec3,
    /// The gaps and overlaps the ray went through before it
    /// ([`llevant::Visibility::veil`]).
    pub veils: llevant::Veils,
}

/// What a [`Probe`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    Miss,
    /// A visible cell (index into [`Model::cells`]).
    Cell(u32),
    Gap,
    /// Overlaps, unsupported cells, too deep nesting, rays given up.
    Problem,
}

impl Probe {
    pub const MISS: Probe = Probe {
        t: f64::INFINITY,
        what: Found::Miss,
        instance: None,
        normal: [0.0; 3],
        veils: llevant::Veils {
            gap: false,
            overlap: false,
        },
    };

    /// The probes of CPU hits of `m` ([`Model::first_hits`],
    /// [`Model::render`]).
    pub fn from_hits(m: &Model, h: &Hits) -> Vec<Probe> {
        let instance: Vec<Option<u64>> = h.labels.iter().map(|l| m.instance(l)).collect();
        let what: Vec<Found> = h
            .labels
            .iter()
            .map(|l| match (l.status, l.cell()) {
                (Status::Inside, Some(c)) => Found::Cell(c),
                (Status::Inside, None) => Found::Problem,
                (Status::Gap, _) => Found::Gap,
                _ => Found::Problem,
            })
            .collect();
        h.ids
            .iter()
            .enumerate()
            .map(|(k, &id)| {
                let veils = h.veils.get(k).copied().unwrap_or_default();
                if id == NO_HIT {
                    Probe {
                        veils,
                        ..Probe::MISS
                    }
                } else {
                    Probe {
                        t: h.t[k],
                        what: what[id as usize],
                        instance: instance[id as usize],
                        normal: h.normal[k],
                        veils,
                    }
                }
            })
            .collect()
    }
}

type V3 = [f32; 3];

fn f3(v: Vec3) -> V3 {
    v.map(|x| x as f32)
}
fn dot3(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm3(a: V3) -> V3 {
    let l = dot3(a, a).sqrt();
    if l > 0.0 {
        a.map(|x| x / l)
    } else {
        a
    }
}
fn mix3(a: V3, b: V3, t: f32) -> V3 {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}
fn mul3(a: V3, s: f32) -> V3 {
    a.map(|x| x * s)
}
fn add3(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

const PI: f32 = std::f32::consts::PI;
/// Irradiance of the key and fill lights.
const KEY: f32 = 2.6;
const FILL: f32 = 0.7;
/// Linear radiance of the sky and the ground of the environment.
const SKY: V3 = [0.62, 0.66, 0.72];
const GROUND: V3 = [0.22, 0.21, 0.20];

/// The studio rig of a camera: unit directions towards the key and fill
/// lights, and the environment's up, all fixed to the view.
#[derive(Clone, Copy, Debug)]
pub struct Rig {
    pub key: V3,
    pub fill: V3,
    pub up: V3,
}

impl Rig {
    pub fn new(camera: &Camera) -> Rig {
        let [f, r, u] = camera.basis().map(f3);
        Rig {
            key: norm3(add3(add3(mul3(r, -0.45), mul3(u, 0.65)), mul3(f, -0.6))),
            fill: norm3(add3(add3(mul3(r, 0.7), mul3(u, 0.05)), mul3(f, -0.7))),
            up: u,
        }
    }
}

/// Environment light seen along `d`, blurred more for rougher surfaces:
/// a sky-to-ground gradient whose horizon sharpens as `roughness` drops.
fn environment(d: V3, up: V3, roughness: f32) -> V3 {
    let w = 0.15 + 0.85 * roughness;
    let t = (0.5 + 0.5 * dot3(d, up) / w).clamp(0.0, 1.0);
    mix3(GROUND, SKY, t)
}

/// Split-sum scale and bias of the specular environment (Karis's analytic
/// fit for mobile).
fn env_brdf(f0: V3, roughness: f32, nov: f32) -> V3 {
    let c0 = [-1.0f32, -0.0275, -0.572, 0.022];
    let c1 = [1.0f32, 0.0425, 1.04, -0.04];
    let r = [0, 1, 2, 3].map(|i| roughness * c0[i] + c1[i]);
    let a004 = f32::min(r[0] * r[0], (-9.28 * nov).exp2()) * r[0] + r[1];
    let a = -1.04 * a004 + r[2];
    let b = 1.04 * a004 + r[3];
    f0.map(|x| x * a + b)
}

/// One light's diffuse and specular radiance towards `v`.
#[allow(clippy::too_many_arguments)]
fn direct(albedo: V3, rough: f32, metal: f32, f0: V3, n: V3, v: V3, l: V3, e: f32) -> V3 {
    let nol = dot3(n, l);
    if nol <= 0.0 {
        return [0.0; 3];
    }
    let nov = dot3(n, v).max(1e-4);
    let h = norm3(add3(v, l));
    let noh = dot3(n, h).max(0.0);
    let voh = dot3(v, h).max(0.0);
    let a = rough * rough;
    let a2 = a * a;
    let dd = noh * noh * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * dd * dd);
    let k = (rough + 1.0) * (rough + 1.0) / 8.0;
    let g = (nol / (nol * (1.0 - k) + k)) * (nov / (nov * (1.0 - k) + k));
    let fw = (1.0 - voh).powi(5);
    let f = f0.map(|x| x + (1.0 - x) * fw);
    let spec = d * g / (4.0 * nol * nov);
    [0, 1, 2].map(|i| ((1.0 - f[i]) * (1.0 - metal) * albedo[i] / PI + f[i] * spec) * e * nol)
}
/// Linear radiance towards the eye of a surface with linear `albedo`,
/// `roughness` and `metalness`, normal `n` (facing the eye) and unit
/// direction `v` towards the eye.
pub fn pbr(albedo: V3, roughness: f32, metalness: f32, n: V3, v: V3, rig: &Rig) -> V3 {
    let rough = roughness.clamp(0.04, 1.0);
    let metal = metalness.clamp(0.0, 1.0);
    let f0 = mix3([0.04; 3], albedo, metal);
    let mut c = add3(
        direct(albedo, rough, metal, f0, n, v, rig.key, KEY),
        direct(albedo, rough, metal, f0, n, v, rig.fill, FILL),
    );
    let nov = dot3(n, v).max(1e-4);
    let diffuse = environment(n, rig.up, 1.0);
    let r = add3(mul3(n, 2.0 * dot3(n, v)), mul3(v, -1.0));
    let spec_env = environment(r, rig.up, rough);
    let brdf = env_brdf(f0, rough, nov);
    for i in 0..3 {
        c[i] += (1.0 - metal) * albedo[i] * diffuse[i] + brdf[i] * spec_env[i];
    }
    c
}

/// Reflectance of water at normal incidence.
const WATER_F0: f32 = 0.02;

/// Linear radiance towards the eye of a clear surface (water): a glossy
/// dielectric over a depth that scatters `albedo` back.
pub fn clear(albedo: V3, roughness: f32, n: V3, v: V3, rig: &Rig) -> V3 {
    let rough = roughness.clamp(0.04, 1.0);
    let f0 = [WATER_F0; 3];
    // Glints: specular only (no albedo).
    let mut c = add3(
        direct([0.0; 3], rough, 0.0, f0, n, v, rig.key, KEY),
        direct([0.0; 3], rough, 0.0, f0, n, v, rig.fill, FILL),
    );
    let nov = dot3(n, v).max(1e-4);
    let r = add3(mul3(n, 2.0 * dot3(n, v)), mul3(v, -1.0));
    let spec_env = environment(r, rig.up, rough);
    let brdf = env_brdf(f0, rough, nov);
    // Light scattered back from the depth: the sky from above and the key.
    let sky = mix3(GROUND, SKY, 0.75);
    let body = [0, 1, 2].map(|i| albedo[i] * (sky[i] + 0.35 * KEY));
    for i in 0..3 {
        c[i] += (1.0 - brdf[i]) * body[i] + brdf[i] * spec_env[i];
    }
    c
}

/// Linear radiance towards the eye of a surface: [`pbr`], blended
/// towards [`clear`] by `clarity`.
pub fn surface(
    albedo: V3,
    roughness: f32,
    metalness: f32,
    clarity: f32,
    n: V3,
    v: V3,
    rig: &Rig,
) -> V3 {
    if clarity >= 1.0 {
        return clear(albedo, roughness, n, v, rig);
    }
    let c = pbr(albedo, roughness, metalness, n, v, rig);
    if clarity <= 0.0 {
        return c;
    }
    mix3(c, clear(albedo, roughness, n, v, rig), clarity)
}

/// ACES filmic tone curve (Narkowicz's fit).
fn aces(x: f32) -> f32 {
    ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0)
}

/// Scale of radiance before the tone curve.
const EXPOSURE: f32 = 0.7;

/// A linear radiance as an 8-bit sRGB colour (exposed, tone-mapped).
pub fn to_srgb8(c: V3) -> Rgb {
    c.map(|x| (llevant_scene::materials::linear_to_srgb(aces(EXPOSURE * x)) * 255.0 + 0.5) as u8)
}

/// Background of realistic images: [`colours::BACKGROUND`] fading
/// to light grey below the horizon of the view.
pub fn background(dir: V3, up: V3) -> Rgb {
    let s = 0.82 + 0.18 * (0.5 + 0.5 * dot3(dir, up));
    colours::BACKGROUND.map(|x| ((x as f32 / 255.0) * s * 255.0 + 0.5) as u8)
}

/// What realistic shading knows of a cell: its colour (as the palette
/// holds it, 8-bit sRGB) and its roughness, metalness and clarity (8-bit,
/// as the GPU palette holds them).
#[derive(Clone, Copy, Debug)]
pub struct CellLook {
    pub srgb: Rgb,
    pub roughness: f32,
    pub metalness: f32,
    pub clarity: f32,
    pub category: Category,
}

impl Paint<'_> {
    /// Look of the cell at index `cell`: its colour and its material's
    /// surface.
    pub fn cell_look(&self, cell: u32) -> CellLook {
        let a: &Appearance = self.looks.cell(cell);
        let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() / 255.0;
        CellLook {
            srgb: self.cell_colour(cell),
            roughness: q(a.roughness),
            metalness: q(a.metalness),
            clarity: q(a.clarity),
            category: a.category,
        }
    }
}

fn linear(c: Rgb) -> V3 {
    c.map(|x| srgb_to_linear(x as f32 / 255.0))
}
/// Render image (8-bit RGB rows from the top) from the probes of a
/// camera's pixels, coloured by `paint`: flat colours [`colours::shade`]d by
/// the angle of each ray to the surface, or lit physically for realistic
/// colours, and the paint's field over it (shaded flat). Gaps and problems
/// are their patterns ([`colours::marked`]), shaded; the pattern of the
/// gaps (or overlaps) a ray went through is half over what it sees beyond
/// ([`colours::veiled`], before the field).
///
/// # Panics
///
/// If there is not one probe per pixel.
pub fn render_image(paint: &Paint, probes: &[Probe], camera: &Camera) -> Result<Vec<u8>> {
    let n = camera.width * camera.height;
    assert_eq!(probes.len(), n, "one probe per pixel");
    let pixel = |k: usize| (k % camera.width, k / camera.width);
    let dir = |k: usize| -> Vec3 {
        let (i, j) = pixel(k);
        camera.direction(i, j)
    };
    let point = |k: usize, t: f64| -> Vec3 {
        let (i, j) = pixel(k);
        camera.point(i, j, t)
    };
    let rig = Rig::new(camera);
    let mut img = vec![0u8; 3 * n];
    img.par_chunks_mut(3).enumerate().for_each(|(k, px)| {
        let p = &probes[k];
        let d = dir(k);
        let c = match p.what {
            Found::Miss if paint.pbr() => background(f3(d), rig.up),
            Found::Miss => colours::BACKGROUND,
            Found::Cell(c) if paint.pbr() => {
                let look = paint.cell_look(c);
                let v = f3(scale(d, -1.0));
                let n = if p.normal == [0.0; 3] {
                    v
                } else {
                    f3(p.normal)
                };
                to_srgb8(surface(
                    linear(look.srgb),
                    look.roughness,
                    look.metalness,
                    look.clarity,
                    n,
                    v,
                    &rig,
                ))
            }
            Found::Cell(c) => colours::shade(paint.cell_colour(c), p.normal, d),
            Found::Gap | Found::Problem => {
                let (i, j) = pixel(k);
                let m = match p.what {
                    Found::Gap => colours::Mark::Gap,
                    _ => colours::Mark::Problem,
                };
                colours::shade(colours::marked(m, i as i64, j as i64), p.normal, d)
            }
        };
        let (i, j) = pixel(k);
        let c = colours::veiled(c, p.veils, i, j);
        px.copy_from_slice(&c);
    });
    if let Some(f) = paint.field {
        img.par_chunks_mut(3).enumerate().for_each(|(k, px)| {
            let p = &probes[k];
            let Found::Cell(c) = p.what else {
                return;
            };
            // The base shaded as the field's colours are (flat), unless
            // it is the image's own.
            let shaded = |c| colours::shade(c, p.normal, dir(k));
            let base = match f.look.under {
                crate::field::Under::Neutral => shaded(f.look.no_value()),
                crate::field::Under::Geometry => [px[0], px[1], px[2]],
            };
            let v = f.field.value_at(c, p.instance, point(k, p.t));
            let c = match v.and_then(|v| f.look.value_colour(v)) {
                Some(c) => crate::field::mix(base, shaded(c), f.look.opacity),
                None => base,
            };
            px.copy_from_slice(&c);
        });
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig() -> Rig {
        let cam = Camera::fit(
            &llevant::Aabb {
                lo: [-1.0; 3],
                hi: [1.0; 3],
            },
            [0.0, 1.0, 0.0],
            10,
            10,
        );
        Rig::new(&cam)
    }

    #[test]
    fn lit_surfaces() {
        let rig = rig();
        let v = [0.0, -1.0, 0.0];
        let grey = pbr([0.5; 3], 0.9, 0.0, v, v, &rig);
        let lin = to_srgb8(grey);
        assert!(lin[0] > 90 && lin[0] < 230, "{lin:?}");
        // Metal at grazing angles reflects more of the environment.
        let side = norm3([1.0, -0.05, 0.0]);
        let m0 = pbr([0.56; 3], 0.3, 1.0, v, v, &rig);
        assert!(m0.iter().all(|x| x.is_finite() && *x >= 0.0));
        let m1 = pbr([0.56; 3], 0.3, 1.0, side, v, &rig);
        assert!(m1.iter().all(|x| x.is_finite() && *x >= 0.0));
    }

    #[test]
    fn water() {
        let rig = rig();
        let v = [0.0, -1.0, 0.0];
        let water = Category::Water.appearance();
        let deep = clear(water.albedo, water.roughness, v, v, &rig);
        // Water is blue-green.
        assert!(deep[2] > deep[1] && deep[1] > 5.0 * deep[0], "{deep:?}");
        let half = surface(water.albedo, 0.3, 1.0, 0.5, v, v, &rig);
        assert!(half.iter().all(|x| x.is_finite()));
    }
}
