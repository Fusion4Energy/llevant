// ---- lattices ------------------------------------------------------------

fn lat_base(fill: u32) -> u32 {
    return P.lattices + LATTICE_WORDS * (fill & ~FILL_LATTICE);
}

fn lat_shift(lb: u32, ix: vec3<i32>) -> vec3<f32> {
    return mv3(lb + 5u) * f32(ix.x) + mv3(lb + 8u) * f32(ix.y) + mv3(lb + 11u) * f32(ix.z);
}

fn lat_index(lb: u32, p: vec3<f32>) -> vec3<i32> {
    let d = p - mv3(lb + 2u);
    let bounded = word(lb + 1u);
    var ix = vec3<i32>(0);
    for (var i = 0u; i < 3u; i++) {
        if ((bounded >> i) & 1u) != 0u {
            ix[i] = i32(floor(dot(mv3(lb + 14u + 3u * i), d) + 0.5));
        }
    }
    if word(lb) != 0u {
        // Hexagons: of the nearby elements, the one holding p most deeply.
        let s0 = word(lb + 23u);
        let ns = word(lb + 24u);
        var best = -BIG;
        var bi = ix.x;
        var bj = ix.y;
        for (var di = -1; di <= 1; di++) {
            for (var dj = -1; dj <= 1; dj++) {
                let i = ix.x + di;
                let j = ix.y + dj;
                let q = p - mv3(lb + 5u) * f32(i) - mv3(lb + 8u) * f32(j);
                var margin = BIG;
                for (var k = 0u; k < ns; k++) {
                    let pb = P.planes + PLANE_WORDS * (s0 + k);
                    margin = min(margin, mf(pb + 3u) - dot(mv3(pb), q));
                }
                if margin > best {
                    best = margin;
                    bi = i;
                    bj = j;
                }
            }
        }
        ix.x = bi;
        ix.y = bj;
    }
    return ix;
}

// Target index of element `ix`, ELEMENT_OWN, or NONE (outside the array).
fn lat_element(lb: u32, ix: vec3<i32>) -> u32 {
    let el = word(lb + 33u);
    let size = vec3<i32>(bitcast<i32>(word(lb + 30u)), bitcast<i32>(word(lb + 31u)), bitcast<i32>(word(lb + 32u)));
    if size.x == 0 {
        return word(P.elements + el);
    }
    let lo = vec3<i32>(bitcast<i32>(word(lb + 27u)), bitcast<i32>(word(lb + 28u)), bitcast<i32>(word(lb + 29u)));
    let k = ix - lo;
    if any(k < vec3<i32>(0)) || any(k >= size) {
        return NONE;
    }
    let flat = u32((k.z * size.y + k.y) * size.x + k.x);
    return word(P.elements + el + flat);
}

// Index of element `ix` among the lattice's numbered elements (`i`
// fastest), or NONE outside them.
fn lat_numbered(lb: u32, ix: vec3<i32>) -> u32 {
    let lo = vec3<i32>(bitcast<i32>(word(lb + 36u)), bitcast<i32>(word(lb + 37u)), bitcast<i32>(word(lb + 38u)));
    let size = vec3<i32>(bitcast<i32>(word(lb + 39u)), bitcast<i32>(word(lb + 40u)), bitcast<i32>(word(lb + 41u)));
    let k = ix - lo;
    if any(k < vec3<i32>(0)) || any(k >= size) {
        return NONE;
    }
    return u32((k.z * size.y + k.y) * size.x + k.x);
}

// Elements numbered for the lattice.
fn lat_numbered_count(lb: u32) -> u32 {
    return word(lb + 39u) * word(lb + 40u) * word(lb + 41u);
}

// Instance id of what level `l`'s cell holds there (`llevant::Instances`):
// the cell's first id, plus the instance of its unit times its elements
// and the element for a lattice cell.
fn cell_instance(l: u32) -> u32 {
    let c = lv_cell[l];
    let base = word(cell_base(c) + 11u);
    let u = lv_inst[l];
    if base == NONE || u == NONE {
        return NONE;
    }
    let fill = word(cell_base(c) + 8u);
    if fill != NONE && (fill & FILL_LATTICE) != 0u {
        let lb = lat_base(fill);
        let e = lat_numbered(lb, lv_ix[l]);
        if e == NONE {
            return NONE;
        }
        return base + u * lat_numbered_count(lb) + e;
    }
    return base + u;
}
