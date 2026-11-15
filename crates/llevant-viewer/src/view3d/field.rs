//! A field over the 3D view ([`llevant_render::field`]): a post-process
//! pass right after the base colours, so that outlines and ambient
//! occlusion fall on it too. Its data is one buffer of its own
//! ([`field_words`], laid out in `shaders/field.wgsl`), uploaded once per
//! field read and colour ramp ([`DataKey`]); its scale and opacity are the
//! pass's numbers ([`field_params`]), so changing them uploads nothing. The
//! CPU's [`llevant_render::shading::render_image`] draws the same.

use aci_tallies_geom::Geometry;
use llevant_gpu::NONE;
use llevant_render::field::{FieldLook, Overlay};
use llevant_render::scale::Ramp;
use llevant_scene::field::Values;

use super::passes::PostPass;

/// The pass ("field"), off until a field is set.
pub struct FieldPass;

impl PostPass for FieldPass {
    fn name(&self) -> &str {
        "field"
    }
    fn wgsl(&self) -> String {
        include_str!("shaders/field.wgsl").into()
    }
    fn data_buffers(&self) -> u32 {
        1
    }
}

/// Words of the header of [`field_words`].
const HEADER: usize = 32;

/// What the pass's buffer depends on: the field's data (by its address,
/// kept alive by whoever holds the key's field) and the colour ramp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataKey(usize, Ramp, u32);

impl DataKey {
    pub fn of(shown: &Overlay) -> DataKey {
        DataKey(
            std::sync::Arc::as_ptr(&shown.field) as usize,
            shown.look.scale.ramp,
            shown.look.scale.bands,
        )
    }
}

/// The pass's numbers for `look`: whether the scale is logarithmic, its
/// ends and the opacity (as `params[0]` in `shaders/field.wgsl`), whether
/// the grey is under it (`params[1].x`) and which (`.yzw`), the colours
/// outside the scale and the threshold (`params[2]`).
pub fn field_params(look: &FieldLook) -> crate::view3d::passes::Params {
    let grey = look.no_value();
    let packed = |c: Option<[u8; 3]>| {
        c.map_or(-1.0, |[r, g, b]| {
            r as f32 + 256.0 * g as f32 + 65536.0 * b as f32
        })
    };
    let [lo, hi] = look.threshold;
    [
        look.scale.log as u32 as f32,
        look.scale.lo as f32,
        look.scale.hi as f32,
        look.opacity.clamp(0.0, 1.0),
        (look.under == llevant_render::field::Under::Neutral) as u32 as f32,
        grey[0] as f32 / 255.0,
        grey[1] as f32 / 255.0,
        grey[2] as f32 / 255.0,
        packed(look.below),
        packed(look.above),
        lo.map_or(-f32::MAX, |x| x as f32),
        hi.map_or(f32::MAX, |x| x as f32),
    ]
}

/// The pass's buffer for `shown` (see `shaders/field.wgsl`): its data and
/// colour ramp, not its scale's ends nor its opacity ([`field_params`]).
pub fn field_words(shown: &Overlay) -> Vec<u32> {
    let (field, look) = (&*shown.field, &shown.look);
    let mut w = vec![0u32; HEADER];
    let f = |x: f64| (x as f32).to_bits();
    let values = |w: &mut Vec<u32>, v: &[f32]| -> (u32, u32) {
        let at = w.len() as u32;
        w.extend(v.iter().map(|x| x.to_bits()));
        (v.len() as u32, at)
    };
    let (mode, (n, at)) = match &field.values {
        Values::Cells(v) => (1, values(&mut w, v)),
        Values::Instances(v) => (2, values(&mut w, v)),
        Values::Mesh(m) => {
            let mode = match m.geometry {
                Geometry::Rectangular { .. } => 3,
                Geometry::Cylindrical { .. } => 4,
            };
            (mode, values(&mut w, &m.values))
        }
    };
    w[0] = mode;
    w[5] = n;
    w[6] = at;
    w[7] = w.len() as u32;
    w.extend(
        look.scale
            .lut(256)
            .into_iter()
            .map(|[r, g, b]| u32::from_le_bytes([r, g, b, 255])),
    );
    w[14] = NONE;
    if let Values::Mesh(m) = &field.values {
        let (a, b, c) = m.geometry.axes();
        let (ni, nj, nk) = m.geometry.dims();
        w[8] = ni as u32;
        w[9] = nj as u32;
        w[10] = nk as u32;
        for (k, axis) in [a, b, c].into_iter().enumerate() {
            w[11 + k] = w.len() as u32;
            w.extend(axis.iter().map(|&x| f(x)));
        }
        let frame: [f64; 12] = match &m.geometry {
            Geometry::Rectangular { transform, .. } => {
                let t = transform.unwrap_or(aci_tallies_geom::Transform::IDENTITY);
                let r = t.rotation;
                [
                    r[0][0],
                    r[0][1],
                    r[0][2],
                    r[1][0],
                    r[1][1],
                    r[1][2],
                    r[2][0],
                    r[2][1],
                    r[2][2],
                    t.translation[0],
                    t.translation[1],
                    t.translation[2],
                ]
            }
            Geometry::Cylindrical { .. } => {
                let c = m.geometry.cyl_frame().expect("a cylinder has a frame");
                let mut out = [0.0; 12];
                for (k, v) in [c.origin, c.e_axis, c.e_ref, c.e_perp].iter().enumerate() {
                    out[3 * k..3 * k + 3].copy_from_slice(v);
                }
                out
            }
        };
        for (k, x) in frame.into_iter().enumerate() {
            w[16 + k] = f(x);
        }
        if let Some(cuv) = &m.by_cell {
            w[14] = w.len() as u32;
            w.extend_from_slice(&cuv.offsets);
            w[15] = w.len() as u32;
            for (c, v) in cuv.cells.iter().zip(&cuv.values) {
                w.push(*c);
                w.push(v.to_bits());
            }
        }
    }
    w
}
