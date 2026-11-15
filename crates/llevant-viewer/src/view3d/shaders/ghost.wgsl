// Ghost cells over the image (after every effect,
// before the highlight): the first ghost boundary each ray crossed, in its
// cell's colour shaded by its normal, more opaque where the ray grazes it
// and with each boundary behind it, as `llevant_render::ghost::over`.
// The records follow the hits in the G-buffer, 4 words per pixel: count,
// cell, t, normal (octahedral, two snorm16).
// params[0]: opacity of one boundary face on (x), added grazing (y), at
// most (z).

fn ghost_word(p: vec2<i32>, k: u32) -> u32 {
    let n = F.size.x * F.size.y;
    return gbuf[9u * n + 4u * (u32(p.y) * F.size.x + u32(p.x)) + k];
}

fn unpack_normal(w: u32) -> vec3<f32> {
    let e = unpack2x16snorm(w);
    var n = vec3<f32>(e.x, e.y, 1.0 - abs(e.x) - abs(e.y));
    if n.z < 0.0 {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), e >= vec2<f32>(0.0));
        n = vec3<f32>((1.0 - abs(e.yx)) * s, n.z);
    }
    return normalize(n);
}

fn effect(p: vec2<i32>) -> vec4<f32> {
    let c = source(p);
    let count = ghost_word(p, 0u);
    let cell = ghost_word(p, 1u);
    if count == 0u || cell == NONE {
        return c;
    }
    let n = unpack_normal(ghost_word(p, 3u));
    let cosine = abs(dot(n, ray_dir(p)));
    // As `colours::shade`.
    let shell = cell_colour(cell).rgb * (0.25 + 0.75 * cosine);
    let graze = 1.0 - min(cosine, 1.0);
    let one = min(F.params[0].x + F.params[0].y * graze * graze, F.params[0].z);
    let a = 1.0 - pow(1.0 - one, f32(min(count, 64u)));
    return vec4<f32>(mix(c.rgb, shell, a), c.a);
}
