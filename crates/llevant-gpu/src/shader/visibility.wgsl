// ---- visibility ----------------------------------------------------------

fn visible(c: u32) -> bool {
    return ((vis[c >> 5u] >> (c & 31u)) & 1u) != 0u;
}

// After the cell bits, `vis` holds what `Model::skip` found: per unit the
// box of its cells worth entering (6 f32), then a bit per octree node
// (`Model::export_skip`); then a bit per ghost cell, and three words: the
// level shown (NONE: the deepest cell of every path), whether any cell
// is a ghost and whether gaps are veils (`llevant::Visibility::veil`). Where each starts: `P.vis_*` (parameters, not constants, so
// that the shader is the same for every model).

fn veils_gaps() -> bool {
    return vis[P.vis_flags + 2u] != 0u;
}

fn is_ghost(c: u32) -> bool {
    return vis[P.vis_flags + 1u] != 0u && ((vis[P.vis_ghosts + (c >> 5u)] >> (c & 31u)) & 1u) != 0u;
}

// Status of the visible cell `c` at level `l` where nothing fills it (or
// at the level shown): a ghost is no solid, and with a level shown only
// cells at it are (`llevant::Visibility`).
fn solid(c: u32, l: u32) -> u32 {
    let shown = vis[P.vis_flags];
    if is_ghost(c) || (shown != NONE && l != shown) {
        return ST_HIDDEN;
    }
    return ST_INSIDE;
}

// The innermost ghost cell of the first `depth` levels of the path, NONE
// if none.
fn innermost_ghost(depth: u32) -> u32 {
    var g = NONE;
    for (var l = 0u; l < depth; l++) {
        let c = lv_cell[l];
        if c != NONE && is_ghost(c) {
            g = c;
        }
    }
    return g;
}
// Hidden stretches are skipped where nothing worth entering can be; false:
// stepped through cell by cell (A/B runs).
override USE_SKIP: bool = true;
// Empty octree nodes one skip crosses before handing back to the steps.
const SKIP_NODES: u32 = 256u;

fn node_worth(n: u32) -> bool {
    return ((vis[P.vis_nodes + (n >> 5u)] >> (n & 31u)) & 1u) != 0u;
}

// The jump to `face` (where something visible may start), stopped two
// steps short, so that the step after it stays in the hidden stretch and
// the visible cell is entered through its own surface (found by the cell's
// exit, which then also gives the normal). -1 when already that close.
fn skip_to(face: f32, t: f32) -> f32 {
    let to = face - 2.0 * (P.eps_abs + P.eps_rel * abs(face));
    if to <= t {
        return -1.0;
    }
    return to;
}

// Level `l` is in a hidden cell at `t`: how far the ray can jump at that
// level with nothing visible on the way (`Model::skip`). Beyond the box of
// the unit's cells worth entering, to near the box's entry (BIG if the ray
// misses it); in the unit's octree, across nodes that hold none of them,
// up to `lim` (where a level above exits). -1 where the cell's own exit is
// needed: in a node that may hold something visible, outside the octree
// within the box, or near the end of a jump. Gaps and overlaps in what is
// jumped are not seen.
fn skip_exit(l: u32, t: f32, lim: f32) -> f32 {
    if !USE_SKIP {
        return -1.0;
    }
    let unit = lv_unit[l];
    let o = lv_o[l];
    let d = lv_d[l];
    let inv = safe_inv(d);
    let vb = P.vis_boxes + 6u * unit;
    let blo = vec3<f32>(bitcast<f32>(vis[vb]), bitcast<f32>(vis[vb + 1u]), bitcast<f32>(vis[vb + 2u]));
    let bhi = vec3<f32>(bitcast<f32>(vis[vb + 3u]), bitcast<f32>(vis[vb + 4u]), bitcast<f32>(vis[vb + 5u]));
    if !in_box(blo, bhi, o + t * d) {
        let r = ray_box(blo, bhi, o, inv, t, BIG);
        if r.x <= r.y {
            return skip_to(r.x, t);
        }
        return BIG;
    }
    // Located in a leaf that may hold something visible (the usual case
    // in solid views): no descent.
    if g_found_leaf != NONE && node_worth(g_found_leaf) {
        return -1.0;
    }
    let ub = P.units + UNIT_WORDS * unit;
    let root = word(ub + 17u);
    if root == NONE {
        return -1.0;
    }
    let glo = mv3(ub + 6u);
    let ghi = mv3(ub + 9u);
    var s = t;
    // The far face of the last empty node crossed.
    var face = t;
    for (var i = 0u; i < SKIP_NODES; i++) {
        let p = o + s * d;
        if s >= lim || !in_box(glo, ghi, p) || !in_box(blo, bhi, p) {
            break;
        }
        var lo = glo;
        var hi = ghi;
        var n = root;
        loop {
            g_work += 1u;
            if !node_worth(n) || gw(2u * n + 1u) != NONE {
                break;
            }
            let c = 0.5 * (lo + hi);
            var k = 0u;
            for (var a = 0u; a < 3u; a++) {
                if p[a] >= c[a] {
                    k |= 1u << a;
                    lo[a] = c[a];
                } else {
                    hi[a] = c[a];
                }
            }
            n = gw(2u * n) + k;
        }
        if node_worth(n) {
            // A leaf that may hold something visible: from here, steps.
            break;
        }
        // Past the empty node's far face (never back).
        face = max(ray_box(lo, hi, o, inv, s, BIG).y, s);
        s = face + P.eps_abs + P.eps_rel * abs(face);
    }
    if s == t {
        return -1.0;
    }
    return skip_to(face, t);
}
