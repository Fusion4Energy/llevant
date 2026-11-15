// A geometry being edited, over the image (`edits.rs`): the surfaces
// marked, evaluated at each pixel's hit point (a line where the side
// changes between neighbouring pixels of one face, a wash where the surface
// is the face seen), and the shell of the cell as edited, traced apart at a
// reduced size, in front of what is seen or (fainter) behind it.
// params[0]: shell width and height (pixels), whether the shell is shown,
// whether surfaces are.

// Head (box lo, count, box hi, bounded), then 20 words per surface: kind
// (llevant::export::SHAPE_*), then its numbers.
@group(1) @binding(0) var<storage, read> shapes: array<f32>;
// Per shell pixel: distance along the ray (negative: none), cosine.
@group(1) @binding(1) var<storage, read> shell: array<f32>;

// theme::MARKED_SURFACE and theme::EDITED_CELL.
const MARKED: vec3<f32> = vec3<f32>(0.376, 0.627, 0.941);
const EDITED: vec3<f32> = vec3<f32>(0.941, 0.588, 0.235);
const HEAD: u32 = 8u;
const STRIDE: u32 = 20u;

fn sv(s: u32, k: u32) -> f32 {
    return shapes[HEAD + STRIDE * s + 1u + k];
}

fn sv3(s: u32, k: u32) -> vec3<f32> {
    return vec3<f32>(sv(s, k), sv(s, k + 1u), sv(s, k + 2u));
}

fn shape_count() -> u32 {
    return min(u32(shapes[3]), (arrayLength(&shapes) - HEAD) / STRIDE);
}

fn in_bounds(p: vec3<f32>) -> bool {
    if shapes[7] < 0.5 {
        return true;
    }
    let lo = vec3<f32>(shapes[0], shapes[1], shapes[2]);
    let hi = vec3<f32>(shapes[4], shapes[5], shapes[6]);
    return all(p >= lo) && all(p <= hi);
}

// The quadric of surface `s` at `p`: value (x) and gradient length (y).
fn quadric(s: u32, p: vec3<f32>) -> vec2<f32> {
    let d = p - sv3(s, 0u);
    let xx = sv(s, 3u);
    let yy = sv(s, 4u);
    let zz = sv(s, 5u);
    let xy = sv(s, 6u);
    let yz = sv(s, 7u);
    let zx = sv(s, 8u);
    let ad = vec3<f32>(
        xx * d.x + xy * d.y + zx * d.z,
        xy * d.x + yy * d.y + yz * d.z,
        zx * d.x + yz * d.y + zz * d.z,
    );
    let b = sv3(s, 9u);
    return vec2<f32>(dot(d, ad) + dot(b, d) + sv(s, 12u), length(2.0 * ad + b));
}

// Surface `s` at `p`: its side (x: 1 positive, 0 negative) and about how
// far `p` is from it (y, cm; large when unknown).
fn side(s: u32, p: vec3<f32>) -> vec2<f32> {
    let kind = u32(shapes[HEAD + STRIDE * s]);
    var f = 0.0;
    var dist = 1e30;
    if kind == 0u {
        f = dot(sv3(s, 0u), p) - sv(s, 3u);
        dist = abs(f);
    } else if kind == 1u || kind == 2u {
        let q = quadric(s, p);
        f = q.x;
        if q.y > 0.0 {
            dist = abs(q.x) / q.y;
        }
        if kind == 2u {
            // Negative inside the double cone on the nappe side.
            let nappe = dot(sv3(s, 16u), p - sv3(s, 13u));
            if f < 0.0 && nappe <= 0.0 {
                f = 1.0;
                dist = 1e30;
            }
        }
    } else {
        let q = vec3<f32>(dot(sv3(s, 0u), p), dot(sv3(s, 3u), p), dot(sv3(s, 6u), p)) + sv3(s, 9u);
        let ax = u32(sv(s, 12u));
        let w = q[ax];
        let u = q[(ax + 1u) % 3u];
        let v = q[(ax + 2u) % 3u];
        let dw = w / sv(s, 14u);
        let dr = (sqrt(u * u + v * v) - sv(s, 13u)) / sv(s, 15u);
        f = dw * dw + dr * dr - 1.0;
    }
    return vec2<f32>(select(0.0, 1.0, f >= 0.0), dist);
}

fn usable(p: vec2<i32>) -> bool {
    return in_image(p) && hit_status(p) == ST_INSIDE && in_bounds(hit_point(p));
}

// 1: a line of a surface at `p`; 2: a surface is the face seen there.
fn surface_mark(p: vec2<i32>) -> u32 {
    if !usable(p) {
        return 0u;
    }
    let a = hit_point(p);
    let ta = hit_t(p);
    let n = shape_count();
    // Nearer than this is on the surface: a few f32 steps of the point.
    let on = 2e-6 * (length(a) + ta) + 1e-4;
    var wash = false;
    let qs = array<vec2<i32>, 4>(
        vec2<i32>(1, 0),
        vec2<i32>(-1, 0),
        vec2<i32>(0, 1),
        vec2<i32>(0, -1),
    );
    for (var s = 0u; s < n; s++) {
        let sa = side(s, a);
        if sa.y < on {
            wash = true;
            continue;
        }
        for (var i = 0; i < 4; i++) {
            let q = p + qs[i];
            if !usable(q) {
                continue;
            }
            let tb = hit_t(q);
            // Not across a jump in depth (another face).
            if abs(ta - tb) > 0.02 * max(ta, tb) {
                continue;
            }
            let sb = side(s, hit_point(q));
            if sb.y >= on && sb.x != sa.x {
                return 1u;
            }
        }
    }
    return select(0u, 2u, wash);
}

// The shell's distance and cosine at image pixel `p` (distance negative:
// none).
fn shell_at(p: vec2<i32>) -> vec2<f32> {
    if !in_image(p) {
        return vec2<f32>(-1.0, 0.0);
    }
    let w = u32(F.params[0].x);
    let h = u32(F.params[0].y);
    let x = min(u32((f32(p.x) + 0.5) * f32(w) / f32(F.size.x)), w - 1u);
    let y = min(u32((f32(p.y) + 0.5) * f32(h) / f32(F.size.y)), h - 1u);
    let k = 2u * (y * w + x);
    if k + 1u >= arrayLength(&shell) {
        return vec2<f32>(-1.0, 0.0);
    }
    return vec2<f32>(shell[k], shell[k + 1u]);
}

fn effect(p: vec2<i32>) -> vec4<f32> {
    var c = source(p);
    if F.params[0].w > 0.5 && shape_count() > 0u {
        let m = surface_mark(p);
        if m == 1u {
            c = vec4<f32>(MARKED, 1.0);
        } else if m == 2u {
            c = vec4<f32>(mix(c.rgb, MARKED, 0.35), c.a);
        }
    }
    if F.params[0].z > 0.5 {
        let s = shell_at(p);
        let inside = s.x >= 0.0;
        var edge = false;
        let qs = array<vec2<i32>, 4>(
            p + vec2<i32>(1, 0),
            p - vec2<i32>(1, 0),
            p + vec2<i32>(0, 1),
            p - vec2<i32>(0, 1),
        );
        for (var i = 0; i < 4; i++) {
            if in_image(qs[i]) && (shell_at(qs[i]).x >= 0.0) != inside {
                edge = true;
            }
        }
        if inside {
            let t = hit_t(p);
            let front = !hit_any(p) || s.x <= t * 1.0005 + 1e-3;
            let shade = EDITED * (0.35 + 0.65 * s.y);
            c = vec4<f32>(mix(c.rgb, shade, select(0.22, 0.5, front)), c.a);
        }
        if edge && inside {
            c = vec4<f32>(EDITED, 1.0);
        }
    }
    return c;
}
