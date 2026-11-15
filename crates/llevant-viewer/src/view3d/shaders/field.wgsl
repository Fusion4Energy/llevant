// A field over the 3D view (`llevant_render::field`): in a cell, the base
// (the neutral grey shaded flat, or the image itself) and where the cell has
// a value, the value's colour, shaded flat, `opacity` of the way over it.
// The pass's numbers (`field_params`): params[0].x logarithmic scale (1) or
// linear (0), .y and .z the scale's ends, .w the opacity; params[1].x the
// base: 1 the grey (`Under::Neutral`), 0 the image (`Under::Geometry`), and
// .yzw that grey (`FieldLook::no_value`, 0 to 1); params[2].x and .y the
// colours below and above the scale (r + 256 g + 65536 b, or -1 for the
// scale's ends: `FieldLook::below`, `above`), .z and .w the threshold (values
// outside are no value; ±3.4e38 for no bound). Its
// one buffer (`view3d/field.rs`, `field_words`): a header of 32 words, then
// the sections it points to.
//
//  0 mode: 1 per cell, 2 per instance, 3 rectangular mesh, 4 cylindrical mesh
//  1..4 unused
//  5 number of values, 6 where they start (f32, NaN for none)
//  7 where the 256 colours of the ramp start (0xAABBGGRR)
//  8, 9, 10 voxels along the mesh's three axes
// 11, 12, 13 where each axis's bounds start (f32, one more than voxels)
// 14 where the cell-under-voxel offsets start (voxels + 1), or NONE
// 15 where its (cell, value) pairs start
// 16..28 rectangular: rotation rows (9) and translation (3), world =
//        R · local + t; cylindrical: origin, axis, theta = 0 and 1/4
//        revolution unit vectors (3 each)

@group(1) @binding(0) var<storage, read> fd: array<u32>;

fn fdf(i: u32) -> f32 {
    return bitcast<f32>(fd[i]);
}

fn fdv3(i: u32) -> vec3<f32> {
    return vec3<f32>(fdf(i), fdf(i + 1u), fdf(i + 2u));
}

// Bin of `x` among `n` bins whose bounds start at `at`: half-open, the last
// one closed; -1 outside.
fn bin(at: u32, n: u32, x: f32) -> i32 {
    if n == 0u || !(x >= fdf(at)) || !(x <= fdf(at + n)) {
        return -1;
    }
    var lo = 0u;
    var hi = n;
    // The last bound at or below x, among the n lower bounds.
    while hi - lo > 1u {
        let mid = (lo + hi) / 2u;
        if fdf(at + mid) <= x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    return i32(lo);
}

// Whether a stored value is one: not NaN nor infinite (by its bits, since
// WGSL may assume floats are never NaN).
fn is_value(bits: u32) -> bool {
    return (bits & 0x7f800000u) != 0x7f800000u;
}

// A value, or none.
struct Value {
    ok: bool,
    v: f32,
}

fn stored(i: u32) -> Value {
    let bits = fd[i];
    return Value(is_value(bits), bitcast<f32>(bits));
}

const NO_VALUE: Value = Value(false, 0.0);

// The voxel holding `x` (world frame), or -1.
fn voxel(x: vec3<f32>) -> i32 {
    let mode = fd[0];
    var local: vec3<f32>;
    if mode == 3u {
        let d = x - fdv3(25u);
        // R's columns are the local axes in the world.
        local = vec3<f32>(
            fdf(16u) * d.x + fdf(19u) * d.y + fdf(22u) * d.z,
            fdf(17u) * d.x + fdf(20u) * d.y + fdf(23u) * d.z,
            fdf(18u) * d.x + fdf(21u) * d.y + fdf(24u) * d.z,
        );
    } else {
        let d = x - fdv3(16u);
        let z = dot(d, fdv3(19u));
        let a = dot(d, fdv3(22u));
        let b = dot(d, fdv3(25u));
        var theta = atan2(b, a) / 6.283185307179586;
        if theta < 0.0 {
            theta += 1.0;
        }
        local = vec3<f32>(sqrt(a * a + b * b), z, theta);
    }
    let n = vec3<u32>(fd[8], fd[9], fd[10]);
    let i = bin(fd[11], n.x, local.x);
    let j = bin(fd[12], n.y, local.y);
    var k = bin(fd[13], n.z, local.z);
    if mode == 4u && k < 0 {
        // Theta wraps around.
        k = bin(fd[13], n.z, local.z + 1.0);
    }
    if i < 0 || j < 0 || k < 0 {
        return -1;
    }
    return (i * i32(n.y) + j) * i32(n.z) + k;
}

// The field's value at pixel `p`, if any.
fn field_value(p: vec2<i32>) -> Value {
    let mode = fd[0];
    let n = fd[5];
    let values = fd[6];
    let cell = hit_cell(p);
    if mode == 1u || mode == 2u {
        var k = cell;
        if mode == 2u {
            k = hit_instance(p);
        }
        if k == NONE || k >= n {
            return NO_VALUE;
        }
        return stored(values + k);
    }
    let v = voxel(hit_point(p));
    if v < 0 || u32(v) >= n {
        return NO_VALUE;
    }
    let offsets = fd[14];
    if offsets == NONE {
        return stored(values + u32(v));
    }
    // Cell under voxel: the part of the voxel in this cell.
    let pairs = fd[15];
    for (var q = fd[offsets + u32(v)]; q < fd[offsets + u32(v) + 1u]; q++) {
        if fd[pairs + 2u * q] == cell {
            return stored(pairs + 2u * q + 1u);
        }
    }
    return NO_VALUE;
}

// The ramp's colour at fraction t, between its 256 entries.
fn ramp(t: f32) -> vec3<f32> {
    let x = clamp(t, 0.0, 1.0) * 255.0;
    let k = min(u32(floor(x)), 254u);
    let a = unpack4x8unorm(fd[fd[7] + k]).rgb;
    let b = unpack4x8unorm(fd[fd[7] + k + 1u]).rgb;
    return mix(a, b, x - f32(k));
}

// A colour written as r + 256 g + 65536 b.
fn unpacked(x: f32) -> vec3<f32> {
    let n = u32(x);
    return vec3<f32>(f32(n & 255u), f32((n >> 8u) & 255u), f32(n >> 16u)) / 255.0;
}

fn effect(p: vec2<i32>) -> vec4<f32> {
    let c = source(p);
    if hit_status(p) != ST_INSIDE || hit_cell(p) == NONE {
        return c;
    }
    // Flat shading, as `llevant_render::colours::shade`.
    var shade = 1.0;
    let n = hit_normal(p);
    if any(n != vec3<f32>(0.0)) {
        shade = 0.25 + 0.75 * abs(dot(n, ray_dir(p)));
    }
    var base = c.rgb;
    if F.params[1].x > 0.5 {
        base = F.params[1].yzw * shade;
    }
    let value = field_value(p);
    if !value.ok {
        return vec4<f32>(base, c.a);
    }
    let v = value.v;
    if v < F.params[2].z || v > F.params[2].w {
        return vec4<f32>(base, c.a);
    }
    let lo = F.params[0].y;
    let hi = F.params[0].z;
    let log = F.params[0].x > 0.5;
    var t = 0.0;
    if log {
        if v > 0.0 && lo > 0.0 && hi > lo {
            t = (log2(v) - log2(lo)) / (log2(hi) - log2(lo));
        }
    } else if hi > lo {
        t = (v - lo) / (hi - lo);
    }
    var colour = ramp(t);
    // Outside the scale, `FieldLook::outside`.
    if (v < lo || (log && !(v > 0.0))) && F.params[2].x >= 0.0 {
        colour = unpacked(F.params[2].x);
    } else if v > hi && F.params[2].y >= 0.0 {
        colour = unpacked(F.params[2].y);
    }
    colour *= shade;
    return vec4<f32>(mix(base, colour, clamp(F.params[0].w, 0.0, 1.0)), c.a);
}
