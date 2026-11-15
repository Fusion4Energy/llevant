// The selection and the cell under the pointer, over the finished image
// (after every effect): selected cells tinted and
// outlined, the hovered cell outlined. Neither needs tracing again.
// params[0]: selection colour (rgb) and tint (w); params[1]: hover colour
// (rgb) and whether the selection is outlined at all (w).

fn selected_at(p: vec2<i32>) -> bool {
    if !in_image(p) || hit_status(p) != ST_INSIDE {
        return false;
    }
    return is_selected(hit_cell(p));
}

fn hovered_at(p: vec2<i32>) -> bool {
    if !in_image(p) || hit_status(p) != ST_INSIDE {
        return false;
    }
    let c = hit_cell(p);
    return c != NONE && c == F.size.z;
}

// Whether a pixel within `r` (4-neighbours, then diagonals) differs.
fn border(p: vec2<i32>, inside: bool, sel: bool, r: i32) -> bool {
    for (var k = 1; k <= r; k++) {
        let qs = array<vec2<i32>, 4>(
            p + vec2<i32>(k, 0),
            p - vec2<i32>(k, 0),
            p + vec2<i32>(0, k),
            p - vec2<i32>(0, k),
        );
        for (var i = 0; i < 4; i++) {
            let q = qs[i];
            if !in_image(q) {
                continue;
            }
            var other: bool;
            if sel {
                other = selected_at(q);
            } else {
                other = hovered_at(q);
            }
            if other != inside {
                return true;
            }
        }
    }
    return false;
}

fn effect(p: vec2<i32>) -> vec4<f32> {
    var c = source(p);
    let sel = selected_at(p);
    let sel_rgb = F.params[0].rgb;
    if sel {
        c = vec4<f32>(mix(c.rgb, sel_rgb, F.params[0].w), c.a);
    }
    if F.params[1].w > 0.5 && border(p, sel, true, 2) {
        // Drawn on both sides of the boundary: 2 px inside, 2 outside.
        return vec4<f32>(sel_rgb, 1.0);
    }
    let hov = hovered_at(p);
    if F.size.z != NONE && border(p, hov, false, 1) {
        return vec4<f32>(mix(c.rgb, F.params[1].rgb, 0.85), 1.0);
    }
    return c;
}
