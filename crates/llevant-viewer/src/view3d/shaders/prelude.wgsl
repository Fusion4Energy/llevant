// Shared by every post-process pass of the 3D view (`passes.rs`). A pass
// defines `fn effect(p: vec2<i32>) -> vec4<f32>`: the colour of pixel `p`
// (column, row from the top), from the G-buffer (`hit_*`), the previous
// pass's image (`source`) and its own numbers (`F.params`). Colours are
// 8-bit sRGB values as fractions, as egui shows them.

struct Frame {
    // Image width and height, the cell under the pointer (or NONE), 0.
    size: vec4<u32>,
    // Eye of the traced view (w: ray reach), forward (w: tan of half the
    // vertical field of view), right (w: half the height of an orthographic
    // view, 0 through a pinhole) and up unit vectors.
    eye: vec4<f32>,
    fwd: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    background: vec4<f32>,
    // The pass's own numbers.
    params: array<vec4<f32>, 3>,
}

@group(0) @binding(0) var<uniform> F: Frame;
// Hits of the image, 9 words per pixel (llevant_gpu::HIT_WORDS).
@group(0) @binding(1) var<storage, read> gbuf: array<u32>;
@group(0) @binding(2) var src: texture_2d<f32>;
@group(0) @binding(3) var dst: texture_storage_2d<rgba8unorm, write>;
// Two words per cell: its colour (0xAABBGGRR, sRGB) and its material's
// roughness and metalness (bytes 0 and 1, as fractions of 255).
@group(0) @binding(4) var<storage, read> palette: array<u32>;
// The selection: one bit per cell (cell c at bit c % 32 of word c / 32).
@group(0) @binding(5) var<storage, read> marks: array<u32>;

const NONE: u32 = 0xffffffffu;
const ST_INSIDE: u32 = 0u;
const ST_GAP: u32 = 1u;
const ST_PROBLEM: u32 = 2u;
const ST_MISS: u32 = 3u;
const ST_STEPS: u32 = 4u;
const ST_TOO_DEEP: u32 = 5u;

// Gaps and problems as patterns fixed to the pixels, as
// `llevant_render::colours::marked`: magenta stripes STRIPE_PX wide along a
// row for problems, a black and grey checker of CHECK_PX squares for gaps.
const STRIPE_PX: i32 = 6;
const CHECK_PX: i32 = 4;

fn floor_div(a: i32, b: i32) -> i32 {
    return select(a / b, (a - b + 1) / b, a < 0);
}

fn marked(gap: bool, p: vec2<i32>) -> vec3<f32> {
    if gap {
        if (floor_div(p.x, CHECK_PX) + floor_div(p.y, CHECK_PX)) % 2 == 0 {
            return vec3<f32>(0.0);
        }
        return vec3<f32>(120.0 / 255.0);
    }
    if floor_div(p.x + p.y, STRIPE_PX) % 2 == 0 {
        return vec3<f32>(1.0, 0.0, 1.0);
    }
    return vec3<f32>(45.0 / 255.0, 0.0, 45.0 / 255.0);
}

fn size() -> vec2<i32> {
    return vec2<i32>(i32(F.size.x), i32(F.size.y));
}

fn in_image(p: vec2<i32>) -> bool {
    return all(p >= vec2<i32>(0)) && all(p < size());
}

fn hit_word(p: vec2<i32>, k: u32) -> u32 {
    return gbuf[9u * (u32(p.y) * F.size.x + u32(p.x)) + k];
}

// Distance along the ray (cm).
fn hit_t(p: vec2<i32>) -> f32 {
    return bitcast<f32>(hit_word(p, 0u));
}

// Deepest cell (index into the model's cells), or NONE.
fn hit_cell(p: vec2<i32>) -> u32 {
    return hit_word(p, 1u);
}

// Level-0 cell, or NONE.
fn hit_top_cell(p: vec2<i32>) -> u32 {
    return hit_word(p, 7u);
}

// Instance of the deepest cell (`llevant::Instances`), or NONE.
fn hit_instance(p: vec2<i32>) -> u32 {
    return hit_word(p, 8u);
}

fn hit_status(p: vec2<i32>) -> u32 {
    return hit_word(p, 3u) & 0xffu;
}

// The veils the ray went through (`llevant::Veils`): gap (x), overlap (y).
fn hit_veils(p: vec2<i32>) -> vec2<bool> {
    let w = hit_word(p, 3u);
    return vec2<bool>(((w >> 15u) & 1u) == 1u, ((w >> 14u) & 1u) == 1u);
}

// Whether the ray stopped at something (a cell, a gap or a problem).
fn hit_any(p: vec2<i32>) -> bool {
    let s = hit_status(p);
    return s != ST_MISS;
}

// Unit normal facing the ray, zero when the ray started inside.
fn hit_normal(p: vec2<i32>) -> vec3<f32> {
    return vec3<f32>(
        bitcast<f32>(hit_word(p, 4u)),
        bitcast<f32>(hit_word(p, 5u)),
        bitcast<f32>(hit_word(p, 6u)),
    );
}

// Pixel `p`'s centre on the image, -1 to 1 across its height.
fn image_xy(p: vec2<i32>) -> vec2<f32> {
    let aspect = f32(F.size.x) / f32(F.size.y);
    let x = (2.0 * (f32(p.x) + 0.5) / f32(F.size.x) - 1.0) * aspect;
    let y = 1.0 - 2.0 * (f32(p.y) + 0.5) / f32(F.size.y);
    return vec2<f32>(x, y);
}

// Unit direction of the ray through pixel `p`, as the renderer casts it.
fn ray_dir(p: vec2<i32>) -> vec3<f32> {
    if F.right.w > 0.0 {
        return F.fwd.xyz;
    }
    let h = F.fwd.w;
    let xy = image_xy(p) * h;
    return normalize(F.fwd.xyz + xy.x * F.right.xyz + xy.y * F.up.xyz);
}

// Where the ray of pixel `p` starts: the eye, or its point of the plane
// through the eye for an orthographic view.
fn ray_origin(p: vec2<i32>) -> vec3<f32> {
    if F.right.w > 0.0 {
        let xy = image_xy(p) * F.right.w;
        return F.eye.xyz + xy.x * F.right.xyz + xy.y * F.up.xyz;
    }
    return F.eye.xyz;
}

// Point hit at pixel `p` (global frame, cm).
fn hit_point(p: vec2<i32>) -> vec3<f32> {
    return ray_origin(p) + hit_t(p) * ray_dir(p);
}

fn cell_colour(cell: u32) -> vec4<f32> {
    return unpack4x8unorm(palette[2u * cell]);
}

// Roughness (x), metalness (y) and clarity (z) of the cell's material.
fn cell_material(cell: u32) -> vec3<f32> {
    return unpack4x8unorm(palette[2u * cell + 1u]).xyz;
}

// Whether cell `cell` is selected.
fn is_selected(cell: u32) -> bool {
    if cell == NONE || cell / 32u >= arrayLength(&marks) {
        return false;
    }
    return ((marks[cell / 32u] >> (cell % 32u)) & 1u) == 1u;
}

// The previous pass's colour at `p`.
fn source(p: vec2<i32>) -> vec4<f32> {
    return textureLoad(src, p, 0);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if !in_image(p) {
        return;
    }
    textureStore(dst, p, effect(p));
}
