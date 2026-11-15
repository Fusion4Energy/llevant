// The G-buffer of the 3D view: per pixel of the full image, the hit of the
// full-resolution trace where its 8×8 block is refined, else the hit of the
// nearest pixel of the low-resolution trace; with ghosts, its ghost record
// too (after all the hits, in each buffer).
//
// A low pixel's hit taken as it is would put the whole of its footprint at
// its distance along the rays of the full pixels: a terrace a fixed
// distance from the eye, with a step to the next one, which ambient
// occlusion (and anything else reading the hit points) sees as relief. So
// a full pixel takes instead the distance along its own ray to the tangent
// plane of the low hit (flat surfaces come out flat across low pixels),
// moved at most 5 times the gap between the low hit and its ray at that
// distance (surfaces seen edge-on, and past silhouettes, stay near the low
// hit).
//
// A low pixel whose ray gave up (`ST_STEPS`: previews bound the work of a
// ray, and a few rays walk far more) takes the hit of the nearest pixel
// that did not, looked for in 8 directions up to 16 pixels away, so that
// previews show no grey. Rays give up in patches (in a close-up cut by a
// plane, 2 % of them, some 8 pixels from a ray that ended).

struct Cam {
    // As `Frame` in the prelude: eye, forward (w: tan of half the vertical
    // field of view), right (w: half the height of an orthographic view, 0
    // through a pinhole), up.
    eye: vec4<f32>,
    fwd: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
}

struct Resolve {
    // Full image width, height; low image width, height.
    size: vec4<u32>,
    // x: blocks per row of the mask; y: whether there are ghost records.
    blocks: vec4<u32>,
    // The camera traced (both images are of it).
    cam: Cam,
}

@group(0) @binding(0) var<uniform> R: Resolve;
@group(0) @binding(1) var<storage, read> low: array<u32>;
@group(0) @binding(2) var<storage, read> full: array<u32>;
@group(0) @binding(3) var<storage, read> mask: array<u32>;
@group(0) @binding(4) var<storage, read_write> gbuf: array<u32>;

// Words per hit and per ghost record (`llevant_gpu::HIT_WORDS`,
// `GHOST_WORDS`).
const HIT_WORDS: u32 = 9u;
const GHOST_WORDS: u32 = 4u;
const ST_MISS: u32 = 3u;
const ST_STEPS: u32 = 4u;

// Pixel `p`'s centre on an image `size` big, -1 to 1 across its height.
fn image_xy(p: vec2<u32>, size: vec2<u32>) -> vec2<f32> {
    let aspect = f32(size.x) / f32(size.y);
    let x = (2.0 * (f32(p.x) + 0.5) / f32(size.x) - 1.0) * aspect;
    let y = 1.0 - 2.0 * (f32(p.y) + 0.5) / f32(size.y);
    return vec2<f32>(x, y);
}

fn ray_dir(xy: vec2<f32>) -> vec3<f32> {
    let c = R.cam;
    if c.right.w > 0.0 {
        return c.fwd.xyz;
    }
    let v = xy * c.fwd.w;
    return normalize(c.fwd.xyz + v.x * c.right.xyz + v.y * c.up.xyz);
}

fn ray_origin(xy: vec2<f32>) -> vec3<f32> {
    let c = R.cam;
    if c.right.w > 0.0 {
        let v = xy * c.right.w;
        return c.eye.xyz + v.x * c.right.xyz + v.y * c.up.xyz;
    }
    return c.eye.xyz;
}

// Distance along full pixel `p`'s ray to the tangent plane of low pixel
// `l`'s hit (distance `t`, normal `n`), within 5 times the gap between that
// hit and the ray.
fn on_plane(p: vec2<u32>, l: vec2<u32>, t: f32, n: vec3<f32>) -> f32 {
    let lxy = image_xy(l, R.size.zw);
    let hit = ray_origin(lxy) + t * ray_dir(lxy);
    let fxy = image_xy(p, R.size.xy);
    let o = ray_origin(fxy);
    let d = ray_dir(fxy);
    let reach = 5.0 * length(o + t * d - hit);
    let dn = dot(d, n);
    var s = t;
    if abs(dn) > 1e-6 {
        s = dot(hit - o, n) / dn;
    }
    return max(clamp(s, t - reach, t + reach), 0.0);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = R.size.x;
    let h = R.size.y;
    if gid.x >= w || gid.y >= h {
        return;
    }
    let px = gid.y * w + gid.x;
    let out = HIT_WORDS * px;
    let ghosts = R.blocks.y != 0u;
    let gout = HIT_WORDS * w * h + GHOST_WORDS * px;
    if mask[(gid.y / 8u) * R.blocks.x + gid.x / 8u] != 0u {
        for (var k = 0u; k < HIT_WORDS; k++) {
            gbuf[out + k] = full[out + k];
        }
        if ghosts {
            for (var k = 0u; k < GHOST_WORDS; k++) {
                gbuf[gout + k] = full[gout + k];
            }
        }
        return;
    }
    var lx = min((gid.x * R.size.z) / w, R.size.z - 1u);
    var ly = min((gid.y * R.size.w) / h, R.size.w - 1u);
    if (low[HIT_WORDS * (ly * R.size.z + lx) + 3u] & 0xffu) == ST_STEPS {
        // Nearest first; at each distance, edges first, then corners.
        let offsets = array<vec2<i32>, 8>(
            vec2<i32>(1, 0), vec2<i32>(-1, 0), vec2<i32>(0, 1), vec2<i32>(0, -1),
            vec2<i32>(1, 1), vec2<i32>(-1, 1), vec2<i32>(1, -1), vec2<i32>(-1, -1),
        );
        let reach = array<i32, 8>(1, 2, 3, 4, 6, 8, 12, 16);
        var found = false;
        for (var r = 0u; r < 8u && !found; r++) {
            for (var k = 0u; k < 8u; k++) {
                let q = vec2<i32>(i32(lx), i32(ly)) + reach[r] * offsets[k];
                if q.x < 0 || q.y < 0 || q.x >= i32(R.size.z) || q.y >= i32(R.size.w) {
                    continue;
                }
                let qp = u32(q.y) * R.size.z + u32(q.x);
                if (low[HIT_WORDS * qp + 3u] & 0xffu) != ST_STEPS {
                    lx = u32(q.x);
                    ly = u32(q.y);
                    found = true;
                    break;
                }
            }
        }
    }
    let lp = ly * R.size.z + lx;
    let at = HIT_WORDS * lp;
    for (var k = 0u; k < HIT_WORDS; k++) {
        gbuf[out + k] = low[at + k];
    }
    let n = vec3<f32>(
        bitcast<f32>(low[at + 4u]),
        bitcast<f32>(low[at + 5u]),
        bitcast<f32>(low[at + 6u]),
    );
    let t = bitcast<f32>(low[at]);
    if (low[at + 3u] & 0xffu) != ST_MISS && any(n != vec3<f32>(0.0)) && t >= 0.0 && t < 1e30 {
        gbuf[out] = bitcast<u32>(on_plane(gid.xy, vec2<u32>(lx, ly), t, n));
    }
    if ghosts {
        let gat = HIT_WORDS * R.size.z * R.size.w + GHOST_WORDS * lp;
        for (var k = 0u; k < GHOST_WORDS; k++) {
            gbuf[gout + k] = low[gat + k];
        }
    }
}
