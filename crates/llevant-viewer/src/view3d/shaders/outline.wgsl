// Lines where the cell changes from one pixel to the next, where the
// depth jumps and where the surface folds (normals apart).
// params[0]: x strength (0..1), y relative depth jump, z normal cosine;
// params[1].xyz the lines' colour, `strength` of the way over the image.

fn edge_with(p: vec2<i32>, q: vec2<i32>) -> bool {
    if !in_image(q) {
        return false;
    }
    let hp = hit_any(p);
    let hq = hit_any(q);
    if hp != hq {
        return true;
    }
    if !hp {
        return false;
    }
    if hit_cell(p) != hit_cell(q) || hit_status(p) != hit_status(q) {
        return true;
    }
    let tp = hit_t(p);
    let tq = hit_t(q);
    if abs(tp - tq) > F.params[0].y * min(tp, tq) {
        return true;
    }
    return dot(hit_normal(p), hit_normal(q)) < F.params[0].z;
}

fn effect(p: vec2<i32>) -> vec4<f32> {
    let c = source(p);
    if edge_with(p, p + vec2<i32>(1, 0)) || edge_with(p, p + vec2<i32>(0, 1)) {
        return vec4<f32>(mix(c.rgb, F.params[1].xyz, F.params[0].x), c.a);
    }
    return c;
}
