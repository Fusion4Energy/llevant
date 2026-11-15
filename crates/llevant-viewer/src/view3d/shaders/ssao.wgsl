// Ambient occlusion in screen space: the share of nearby pixels (a disc of
// params[0].x pixels) whose hit point stands above this pixel's tangent
// plane by more than params[0].z of its depth, but is closer than
// params[0].w of its depth (so that distant foreground does not darken).
// Darkens by up to params[0].y. Measured against the tangent plane, a flat
// surface seen at a slant does not occlude itself; pixels without a normal
// (rays that start inside a cell) are left as they are.

fn effect(p: vec2<i32>) -> vec4<f32> {
    let c = source(p);
    if !hit_any(p) {
        return c;
    }
    let n = hit_normal(p);
    if all(n == vec3<f32>(0.0)) {
        return c;
    }
    let t = hit_t(p);
    let here = hit_point(p);
    let radius = F.params[0].x;
    var occluded = 0.0;
    var total = 0.0;
    // A spiral of 16 samples, turned per pixel.
    let spin = f32((u32(p.x) * 7u + u32(p.y) * 13u) % 16u) * 0.3927;
    for (var k = 0u; k < 16u; k++) {
        let a = f32(k) * 2.3999632 + spin;
        let r = radius * sqrt((f32(k) + 0.5) / 16.0);
        let q = p + vec2<i32>(vec2<f32>(cos(a), sin(a)) * r);
        if !in_image(q) {
            continue;
        }
        total += 1.0;
        if !hit_any(q) {
            continue;
        }
        let d = hit_point(q) - here;
        let height = dot(d, n);
        if height > F.params[0].z * t && length(d) < F.params[0].w * t {
            occluded += 1.0;
        }
    }
    let ao = select(0.0, occluded / total, total > 0.0);
    return vec4<f32>(c.rgb * (1.0 - F.params[0].y * ao), c.a);
}
