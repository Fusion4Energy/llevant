// The hits shown, moved to the camera wanted now: every frame while the
// camera moves, so the image follows it at once and new traces replace
// what was moved as they land.
//
// Each pixel of the G-buffer (traced with camera `a`) is a point: its hit,
// its first ghost boundary where it hit nothing, or where nothing at all,
// the end of its ray (so that the background claims its pixels too, behind
// everything, instead of leaving holes the pixels around would fill). `scatter_depth` sends
// each point to the pixel of camera `b` it falls in, keeping the nearest
// (atomicMin on the depth's bits: positive floats order as integers);
// `scatter_owner` names the source pixel at that depth; `gather` copies the
// owner's hit (its distance measured again along the new pixel's ray), or
// where no point fell, the nearest of its neighbours' (a point covers more
// than a pixel when zooming in), else nothing (a miss: what was hidden
// before shows once traced).

struct Cam {
    // As `Frame` in the prelude: eye, forward (w: tan of half the vertical
    // field of view), right (w: half the height of an orthographic view, 0
    // through a pinhole), up.
    eye: vec4<f32>,
    fwd: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
}

struct Reproject {
    // Width, height, whether there are ghost records, 0.
    size: vec4<u32>,
    a: Cam,
    b: Cam,
}

@group(0) @binding(0) var<uniform> R: Reproject;
@group(0) @binding(1) var<storage, read> gin: array<u32>;
@group(0) @binding(2) var<storage, read_write> depth: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read_write> owner: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read_write> gout: array<u32>;

const HIT_WORDS: u32 = 9u;
const GHOST_WORDS: u32 = 4u;
const NONE: u32 = 0xffffffffu;
const ST_MISS: u32 = 3u;

fn image_xy(p: vec2<u32>) -> vec2<f32> {
    let aspect = f32(R.size.x) / f32(R.size.y);
    let x = (2.0 * (f32(p.x) + 0.5) / f32(R.size.x) - 1.0) * aspect;
    let y = 1.0 - 2.0 * (f32(p.y) + 0.5) / f32(R.size.y);
    return vec2<f32>(x, y);
}

fn ray_dir(c: Cam, p: vec2<u32>) -> vec3<f32> {
    if c.right.w > 0.0 {
        return c.fwd.xyz;
    }
    let xy = image_xy(p) * c.fwd.w;
    return normalize(c.fwd.xyz + xy.x * c.right.xyz + xy.y * c.up.xyz);
}

fn ray_origin(c: Cam, p: vec2<u32>) -> vec3<f32> {
    if c.right.w > 0.0 {
        let xy = image_xy(p) * c.right.w;
        return c.eye.xyz + xy.x * c.right.xyz + xy.y * c.up.xyz;
    }
    return c.eye.xyz;
}

fn pixels() -> u32 {
    return R.size.x * R.size.y;
}

// The point of source pixel `p` (w = 1), or w = 0 where it has none.
fn point(p: vec2<u32>) -> vec4<f32> {
    let px = p.y * R.size.x + p.x;
    var t = bitcast<f32>(gin[HIT_WORDS * px]);
    if (gin[HIT_WORDS * px + 3u] & 0xffu) == ST_MISS {
        let g = HIT_WORDS * pixels() + GHOST_WORDS * px;
        if R.size.z != 0u && gin[g] != 0u {
            t = bitcast<f32>(gin[g + 2u]);
        } else {
            t = min(R.a.eye.w, 1e7);
        }
    }
    // (Also false for NaN.)
    if !(t >= 0.0 && t < 1e30) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(ray_origin(R.a, p) + t * ray_dir(R.a, p), 1.0);
}

// Where `q` shows in camera `b`: pixel column and row (xy) and depth (z),
// z <= 0 where it does not.
fn project(q: vec3<f32>) -> vec3<f32> {
    let b = R.b;
    let v = q - b.eye.xyz;
    let z = dot(v, b.fwd.xyz);
    if z <= 0.0 {
        return vec3<f32>(0.0, 0.0, -1.0);
    }
    var half = b.right.w;
    if half <= 0.0 {
        half = z * b.fwd.w;
    }
    let aspect = f32(R.size.x) / f32(R.size.y);
    let x = dot(v, b.right.xyz) / half / aspect;
    let y = dot(v, b.up.xyz) / half;
    return vec3<f32>(0.5 * (x + 1.0) * f32(R.size.x), 0.5 * (1.0 - y) * f32(R.size.y), z);
}

// The target pixel and depth bits of source pixel `p`, or NONE.
fn landing(p: vec2<u32>) -> vec2<u32> {
    let q = point(p);
    if q.w == 0.0 {
        return vec2<u32>(NONE, 0u);
    }
    let s = project(q.xyz);
    if s.z <= 0.0 || s.x < 0.0 || s.y < 0.0 || s.x >= f32(R.size.x) || s.y >= f32(R.size.y) {
        return vec2<u32>(NONE, 0u);
    }
    let i = min(u32(s.x), R.size.x - 1u);
    let j = min(u32(s.y), R.size.y - 1u);
    return vec2<u32>(j * R.size.x + i, bitcast<u32>(s.z));
}

@compute @workgroup_size(8, 8)
fn clear(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= R.size.x || gid.y >= R.size.y {
        return;
    }
    let px = gid.y * R.size.x + gid.x;
    atomicStore(&depth[px], NONE);
    atomicStore(&owner[px], NONE);
}

@compute @workgroup_size(8, 8)
fn scatter_depth(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= R.size.x || gid.y >= R.size.y {
        return;
    }
    let t = landing(gid.xy);
    if t.x != NONE {
        atomicMin(&depth[t.x], t.y);
    }
}

@compute @workgroup_size(8, 8)
fn scatter_owner(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= R.size.x || gid.y >= R.size.y {
        return;
    }
    let t = landing(gid.xy);
    if t.x != NONE && atomicLoad(&depth[t.x]) == t.y {
        atomicMin(&owner[t.x], gid.y * R.size.x + gid.x);
    }
}

@compute @workgroup_size(8, 8)
fn gather(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = R.size.x;
    let h = R.size.y;
    if gid.x >= w || gid.y >= h {
        return;
    }
    let qx = gid.y * w + gid.x;
    var o = atomicLoad(&owner[qx]);
    if o == NONE {
        // A hole: the nearest point of the pixels around.
        var best = NONE;
        for (var dj = -1; dj <= 1; dj++) {
            for (var di = -1; di <= 1; di++) {
                let x = i32(gid.x) + di;
                let y = i32(gid.y) + dj;
                if x < 0 || y < 0 || x >= i32(w) || y >= i32(h) {
                    continue;
                }
                let n = u32(y) * w + u32(x);
                let d = atomicLoad(&depth[n]);
                let no = atomicLoad(&owner[n]);
                if no != NONE && d < best {
                    best = d;
                    o = no;
                }
            }
        }
    }
    let out = HIT_WORDS * qx;
    let gout_at = HIT_WORDS * pixels() + GHOST_WORDS * qx;
    if o == NONE {
        // Nothing seen there yet.
        gout[out] = bitcast<u32>(0.0);
        gout[out + 1u] = NONE;
        gout[out + 2u] = NONE;
        gout[out + 3u] = ST_MISS;
        gout[out + 4u] = 0u;
        gout[out + 5u] = 0u;
        gout[out + 6u] = 0u;
        gout[out + 7u] = NONE;
        gout[out + 8u] = NONE;
        if R.size.z != 0u {
            for (var k = 0u; k < GHOST_WORDS; k++) {
                gout[gout_at + k] = 0u;
            }
        }
        return;
    }
    let at = HIT_WORDS * o;
    for (var k = 0u; k < HIT_WORDS; k++) {
        gout[out + k] = gin[at + k];
    }
    if (gin[at + 3u] & 0xffu) != ST_MISS {
        // Its distance along this pixel's ray.
        let src = vec2<u32>(o % w, o / w);
        let q = point(src).xyz;
        let t = dot(q - ray_origin(R.b, gid.xy), ray_dir(R.b, gid.xy));
        gout[out] = bitcast<u32>(max(t, 0.0));
    }
    if R.size.z != 0u {
        let gat = HIT_WORDS * pixels() + GHOST_WORDS * o;
        for (var k = 0u; k < GHOST_WORDS; k++) {
            gout[gout_at + k] = gin[gat + k];
        }
    }
}
