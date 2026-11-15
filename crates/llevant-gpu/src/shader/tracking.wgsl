// ---- tracking ------------------------------------------------------------

struct Located {
    status: u32,
    depth: u32,
}

// Locate the point at `t` from level `start_level` down; levels above keep their
// state.
fn locate(start_level: u32, t: f32) -> Located {
    var l = start_level;
    loop {
        if l >= MAXL {
            return Located(ST_TOO_DEEP, l);
        }
        let o = lv_o[l];
        let p = o + t * lv_d[l];
        let r = find_cell(lv_unit[l], p);
        if r == NONE {
            lv_cell[l] = NONE;
            return Located(ST_GAP, l + 1u);
        }
        if (r & PROBLEM) != 0u {
            lv_cell[l] = r & ~PROBLEM;
            return Located(ST_PROBLEM, l + 1u);
        }
        lv_cell[l] = r;
        if !visible(r) {
            return Located(ST_HIDDEN, l + 1u);
        }
        let fill = word(cell_base(r) + 8u);
        // At the level shown, a filled cell is a cell (a lattice cell
        // keeps its elements).
        let last = l == vis[P.vis_flags];
        if fill == NONE || (last && (fill & FILL_LATTICE) == 0u) {
            return Located(solid(r, l), l + 1u);
        }
        var tgt = fill;
        var oe = o;
        // The element's index among the numbered ones (lattices).
        var numbered = 0u;
        if (fill & FILL_LATTICE) != 0u {
            let lb = lat_base(fill);
            let ix = lat_index(lb, p);
            numbered = lat_numbered(lb, ix);
            lv_ix[l] = ix;
            let el = lat_element(lb, ix);
            if el == NONE {
                return Located(ST_GAP, l + 1u);
            }
            if el == ELEMENT_OWN || last {
                return Located(solid(r, l), l + 1u);
            }
            oe = o - lat_shift(lb, ix);
            tgt = el;
        }
        let tb = P.targets + TARGET_WORDS * tgt;
        let m = transpose(mat3x3<f32>(mv3(tb + 1u), mv3(tb + 4u), mv3(tb + 7u)));
        lv_o[l + 1u] = m * oe + mv3(tb + 10u);
        lv_d[l + 1u] = m * lv_d[l];
        lv_r[l + 1u] = m * lv_r[l];
        lv_unit[l + 1u] = word(tb);
        // offset + host · stride (+ element, for a lattice filled with one
        // universe, stride > 1).
        let offset = word(tb + 13u);
        let stride = word(tb + 14u);
        var inst = NONE;
        if offset != NONE && lv_inst[l] != NONE {
            if stride == 1u {
                inst = offset + lv_inst[l];
            } else if numbered != NONE {
                inst = offset + lv_inst[l] * stride + numbered;
            }
        }
        lv_inst[l + 1u] = inst;
        l += 1u;
    }
    return Located(ST_TOO_DEEP, l);
}

// First crossing after `t` of the surfaces of level `l`'s cell, skipping
// program nodes whose boxes the stretch misses (they are constant along it).
fn cell_exit(l: u32, t: f32) -> vec3<u32> {
    let c = lv_cell[l];
    let cb = cell_base(c);
    let o = lv_o[l];
    let d = lv_d[l];
    let inv = safe_inv(d);
    var best = g_tmax;
    // The cell lies in its box.
    let rb = ray_box(mv3(cb), mv3(cb + 3u), o, inv, t, best);
    if rb.x <= rb.y {
        best = rb.y;
    }
    // Node boxes are exact only inside the unit's region.
    let ub = P.units + UNIT_WORDS * lv_unit[l];
    let use_boxes = in_box(mv3(ub), mv3(ub + 3u), o + t * d);
    if use_boxes {
        best = min(best, ray_box(mv3(ub), mv3(ub + 3u), o, inv, t, best).y);
    }
    // A surface on a box face can round to just past the box's exit: look a
    // little further, so the exit keeps its surface (and the hit its normal).
    let box_exit = best;
    // In the octree: leaf after leaf along the ray, the cell's program
    // specialized to the leaf, until one of its surfaces is crossed. Where
    // the cell has no residual in a leaf (or the ray leaves the octree), the
    // exit is that leaf's face and the ray relocates there; in the first
    // leaf, or after `MAX_LEAVES` leaves, the whole program from there on.
    var walk_end = t;
    if USE_RESIDUALS && use_boxes && word(ub + 17u) != NONE {
        let glo = mv3(ub + 6u);
        let ghi = mv3(ub + 9u);
        var cur = t;
        var probe = t;
        var whole = false;
        for (var k = 0u; k < MAX_LEAVES; k++) {
            let p = o + probe * d;
            var pr = NONE;
            var found = false;
            if in_box(glo, ghi, p) {
                let leaf = grid_leaf(ub, p);
                for (var i = 0u; i < leaf.y; i++) {
                    if gw(leaf.x + i) == c {
                        pr = word(P.grid_progs + leaf.x + i);
                        found = true;
                        break;
                    }
                }
            }
            if !found || pr == NONE {
                if k == 0u {
                    whole = true;
                    break;
                }
                return vec3<u32>(bitcast<u32>(cur), NONE, 0u);
            }
            g_work += 1u;
            // Rounding can put the point on (or just past) the leaf's far
            // face: never step back.
            let le = max(ray_box(g_llo, g_lhi, o, inv, cur, box_exit).y, cur);
            let lim = min(le, box_exit);
            if pr != PROG_TRUE {
                let e = program_exit(
                    pr, pr + prog_len(pr), word(cb + 12u), o, d, inv, cur,
                    lim + P.eps_abs + P.eps_rel * abs(lim), use_boxes);
                if e.y != NONE {
                    return e;
                }
            }
            if le >= box_exit {
                return vec3<u32>(bitcast<u32>(box_exit), NONE, 0u);
            }
            cur = le;
            probe = le + P.eps_abs + P.eps_rel * abs(le);
        }
        if !whole {
            walk_end = cur;
        }
    }
    let e = program_exit(
        word(cb + 6u), word(cb + 6u) + word(cb + 7u), word(cb + 12u), o, d, inv, walk_end,
        best + P.eps_abs + P.eps_rel * abs(best), use_boxes);
    if e.y == NONE {
        return vec3<u32>(bitcast<u32>(box_exit), NONE, 0u);
    }
    return e;
}

// First crossing after `t` and before `limit` of the surfaces of the program
// in `code[start..end]`, skipping nodes whose boxes the stretch misses (they
// are constant along it): `(t, shape, error)` as `cell_exit` gives, the
// shape `NONE` when there is none.
fn first_crossing(start: u32, end: u32, nbase: u32, o: vec3<f32>, d: vec3<f32>, inv: vec3<f32>, t: f32, limit: f32, use_boxes: bool) -> vec3<u32> {
    var best = limit;
    var surf = NONE;
    var err = 0.0;
    var pos = start;
    while pos < end {
        g_work += 1u;
        if STATS {
            n_exit_words += 1u;
        }
        let w = cw(pos);
        if (w & NODE) == 0u {
            let s = w & MASK;
            let tn = shape_next(s, o, d, t, best);
            if tn < best {
                best = tn;
                surf = s;
                err = g_root_err;
            }
            pos += 1u;
            continue;
        }
        let bw = cw(pos + 1u);
        if use_boxes && bw != NONE {
            let b = (nbase + (bw & ~NODE)) * 12u;
            let rt = ray_box(nv3(b), nv3(b + 3u), o, inv, t, best);
            let rf = ray_box(nv3(b + 6u), nv3(b + 9u), o, inv, t, best);
            if rt.x > rt.y || rf.x > rf.y {
                pos += 1u + (w & MASK);
                continue;
            }
        }
        pos += 2u;
    }
    return vec3<u32>(bitcast<u32>(best), surf, bitcast<u32>(err));
}

// Where a cell's program (in `code[start..end]`) may stop holding the ray
// after `t`, before `limit`: `exit_bound`, or the first crossing of any of
// its surfaces (`EXIT_BOUND` off).
fn program_exit(start: u32, end: u32, nbase: u32, o: vec3<f32>, d: vec3<f32>, inv: vec3<f32>, t: f32, limit: f32, use_boxes: bool) -> vec3<u32> {
    if EXIT_BOUND {
        return exit_bound(start, end, nbase, o, d, inv, t, limit, use_boxes);
    }
    return first_crossing(start, end, nbase, o, d, inv, t, limit, use_boxes);
}

// Where the program in `code[start..end]` (one node or leaf, true along the
// ray just after `t`) can first turn false, before `limit`: `(t, shape,
// error)` as `first_crossing` gives, the shape `NONE` for none.
//
// A leaf keeps its side from `t` to its next crossing, so one surely true
// there (tested at the midpoint, away from both ends) is true until that
// crossing; an AND of surely true children until the first of theirs, an
// OR with a surely true child until the last of those. That is never past
// where the program turns false, so the ray stops for the crossings that can
// take it out of the cell, not for every surface of a long void program that
// it passes by (most steps of the slowest rays of the reference model). When
// the program is not surely true (a crossing within the tolerance of `t`),
// the first crossing of any surface, as `first_crossing`.
fn exit_bound(start: u32, end: u32, nbase: u32, o: vec3<f32>, d: vec3<f32>, inv: vec3<f32>, t: f32, limit: f32, use_boxes: bool) -> vec3<u32> {
    let tol = P.eps_abs + P.eps_rel * abs(t);
    // The first crossing of any surface.
    var first = limit;
    var first_surf = NONE;
    var first_err = 0.0;
    // The value handed up: surely true from `t` until `vb` (`NOT_SURE`: not
    // surely true), crossing `vs` there. With `NOT_SURE` below every bound, an
    // OR is the max of its children and an AND the min.
    var vb = NOT_SURE;
    var vs = NONE;
    var pos = start;
    var sp = 0u;
    loop {
        g_work += 1u;
        if STATS {
            n_exit_words += 1u;
        }
        let w = cw(pos);
        if (w & NODE) == 0u {
            let s = w & MASK;
            let tn = shape_next(s, o, d, t, limit);
            if tn < first {
                first = tn;
                first_surf = s;
                first_err = g_root_err;
            }
            let r = min(tn, limit);
            let sure = r - t > tol && side_pos(s, o + (0.5 * (t + r)) * d) == ((w & POS) != 0u);
            vb = select(NOT_SURE, r, sure);
            vs = select(NONE, s, tn < limit);
            pos += 1u;
        } else {
            let node_end = pos + 1u + (w & MASK);
            let bw = cw(pos + 1u);
            var decided = false;
            if use_boxes && bw != NONE {
                let b = (nbase + (bw & ~NODE)) * 12u;
                let rt = ray_box(nv3(b), nv3(b + 3u), o, inv, t, limit);
                let rf = ray_box(nv3(b + 6u), nv3(b + 9u), o, inv, t, limit);
                if rt.x > rt.y {
                    // False all along the stretch.
                    vb = NOT_SURE;
                    decided = true;
                } else if rf.x > rf.y {
                    // True all along it.
                    vb = limit;
                    decided = true;
                }
                vs = NONE;
            }
            if decided {
                pos = node_end;
            } else {
                let is_or = (w & OR) != 0u;
                cstack_end[sp] = node_end | select(0u, STACK_OR, is_or);
                cstack_bound[sp] = select(limit, NOT_SURE, is_or);
                // The BVH stack is free while a cell's exit is found.
                bstack[sp] = NONE;
                sp += 1u;
                pos += 2u;
                continue;
            }
        }
        // Hand the value to the enclosing nodes; a node whose children are
        // all done hands its own on.
        loop {
            if sp == 0u {
                break;
            }
            let k = sp - 1u;
            let is_or = (cstack_end[k] & STACK_OR) != 0u;
            let kb = cstack_bound[k];
            if (is_or && vb > kb) || (!is_or && vb < kb) {
                cstack_bound[k] = vb;
                bstack[k] = vs;
            }
            if pos < (cstack_end[k] & ~STACK_OR) {
                break;
            }
            vb = cstack_bound[k];
            vs = bstack[k];
            sp = k;
        }
        if sp == 0u {
            break;
        }
    }
    if vb == NOT_SURE {
        return vec3<u32>(bitcast<u32>(first), first_surf, bitcast<u32>(first_err));
    }
    if vs == NONE {
        return vec3<u32>(bitcast<u32>(vb), NONE, 0u);
    }
    // Its root error (not kept per node): the same root again.
    _ = shape_next(vs, o, d, t, limit);
    return vec3<u32>(bitcast<u32>(vb), vs, bitcast<u32>(g_root_err));
}

// Where the ray leaves the lattice element it is in at level `l`.
fn element_exit(l: u32, t: f32) -> vec3<u32> {
    let lb = lat_base(word(cell_base(lv_cell[l]) + 8u));
    let q = lv_o[l] - lat_shift(lb, lv_ix[l]);
    let d = lv_d[l];
    var best = g_tmax;
    var surf = NONE;
    let f0 = word(lb + 25u);
    let nf = word(lb + 26u);
    for (var k = 0u; k < nf; k++) {
        let pb = P.planes + PLANE_WORDS * (f0 + k);
        let n = mv3(pb);
        let nd = dot(n, d);
        if nd > 0.0 {
            let tt = (mf(pb + 3u) - dot(n, q)) / nd;
            if tt < best {
                best = tt;
                surf = word(pb + 4u);
            }
        }
    }
    return vec3<u32>(bitcast<u32>(max(best, t)), surf, bitcast<u32>(3e-7 * (length(q) + abs(best))));
}

// Where the ray leaves the gap it is in at level `l` (no cell of the unit
// holds it at `t`), to see through it (`veils_gaps`): the first crossing
// after `t` of a surface of a cell that may hold a point further on (the
// candidates of the octree leaf, within it; the BVH items along the ray;
// beyond the unit's region every cell of its list; and the cells checked
// everywhere), the entry into the box of an unsupported one, or the region's
// (or octree's) boundary. Only there can the point's cell change.
// `(t, shape, error)` as `cell_exit`.
fn gap_exit(l: u32, t: f32) -> vec3<u32> {
    let o = lv_o[l];
    let d = lv_d[l];
    let inv = safe_inv(d);
    let p = o + t * d;
    let ub = P.units + UNIT_WORDS * lv_unit[l];
    let in_roi = in_box(mv3(ub), mv3(ub + 3u), p);
    var best = vec3<u32>(bitcast<u32>(g_tmax), NONE, 0u);
    let rb = ray_box(mv3(ub), mv3(ub + 3u), o, inv, t, g_tmax);
    if rb.x <= rb.y {
        if in_roi {
            best.x = bitcast<u32>(max(rb.y, t));
        } else if rb.x > t {
            best.x = bitcast<u32>(rb.x);
        }
    }
    if !in_roi {
        let s = word(ub + 12u);
        let n = word(ub + 13u);
        for (var i = 0u; i < n; i++) {
            best = gap_candidate(word(s + i), NONE, o, d, inv, t, best, false);
        }
    } else if word(ub + 17u) != NONE && in_box(mv3(ub + 6u), mv3(ub + 9u), p) {
        let leaf = grid_leaf(ub, p);
        let le = ray_box(g_llo, g_lhi, o, inv, t, bitcast<f32>(best.x));
        if le.x <= le.y {
            best = vec3<u32>(bitcast<u32>(max(le.y, t)), NONE, 0u);
        }
        for (var i = 0u; i < leaf.y; i++) {
            var pr = NONE;
            if USE_RESIDUALS {
                pr = word(P.grid_progs + leaf.x + i);
            }
            best = gap_candidate(gw(leaf.x + i), pr, o, d, inv, t, best, true);
        }
    } else {
        // Into the octree, the candidates change.
        if word(ub + 17u) != NONE {
            let gb = ray_box(mv3(ub + 6u), mv3(ub + 9u), o, inv, t, bitcast<f32>(best.x));
            if gb.x <= gb.y && gb.x > t {
                best = vec3<u32>(bitcast<u32>(gb.x), NONE, 0u);
            }
        }
        if word(ub + 16u) != NONE {
            bstack[0] = word(ub + 16u);
            var sp = 1u;
            while sp > 0u {
                sp -= 1u;
                let nb = P.bvh_nodes + BVH_NODE_WORDS * bstack[sp];
                let nr = ray_box(mv3(nb), mv3(nb + 3u), o, inv, t, bitcast<f32>(best.x));
                if nr.x > nr.y {
                    continue;
                }
                let start = word(nb + 6u);
                let count = word(nb + 7u);
                if count == 0u {
                    bstack[sp] = start;
                    bstack[sp + 1u] = start + 1u;
                    sp += 2u;
                    continue;
                }
                for (var i = 0u; i < count; i++) {
                    let ib = P.bvh_items + BVH_ITEM_WORDS * (start + i);
                    let ir = ray_box(mv3(ib + 1u), mv3(ib + 4u), o, inv, t, bitcast<f32>(best.x));
                    if ir.x <= ir.y {
                        best = gap_candidate(word(ib), NONE, o, d, inv, t, best, true);
                    }
                }
            }
        }
    }
    let s = word(ub + 14u);
    let n = word(ub + 15u);
    for (var i = 0u; i < n; i++) {
        best = gap_candidate(word(s + i), NONE, o, d, inv, t, best, in_roi);
    }
    return best;
}

// `best` (as `gap_exit` gives), or cell `c`'s first crossing after `t` if
// earlier: of its program, or of `pr`, its residual in the octree leaf
// (NONE: none; PROG_TRUE: it holds all of the leaf in its box, so it
// starts where the ray enters that box); for an unsupported cell, where
// the ray enters its box.
fn gap_candidate(c: u32, pr: u32, o: vec3<f32>, d: vec3<f32>, inv: vec3<f32>, t: f32, best: vec3<u32>, use_boxes: bool) -> vec3<u32> {
    let cb = cell_base(c);
    let lim = bitcast<f32>(best.x);
    if (word(cb + 9u) & CELL_UNSUPPORTED) != 0u || pr == PROG_TRUE {
        let r = ray_box(mv3(cb), mv3(cb + 3u), o, inv, t, lim);
        if r.x <= r.y && r.x > t && r.x < lim {
            return vec3<u32>(bitcast<u32>(r.x), NONE, 0u);
        }
        return best;
    }
    var start = word(cb + 6u);
    var end = start + word(cb + 7u);
    if pr != NONE {
        start = pr;
        end = pr + prog_len(pr);
    }
    if start == end {
        return best;
    }
    let e = first_crossing(start, end, word(cb + 12u), o, d, inv, t, lim, use_boxes);
    if e.y != NONE && bitcast<f32>(e.x) < lim {
        return e;
    }
    return best;
}

// Unit normal of shape `ssurf` (in level `lsurf`'s frame) at `t`, in the
// global frame and facing the ray (zero without a shape).
fn surface_normal(ssurf: u32, lsurf: u32, t: f32, d: vec3<f32>) -> vec3<f32> {
    if ssurf == NONE {
        return vec3<f32>(0.0);
    }
    let g = gradient(ssurf, lv_o[lsurf] + t * lv_d[lsurf]);
    var n = transpose(lv_r[lsurf]) * g;
    let len = length(n);
    if len > 0.0 {
        n = n / len;
    }
    if dot(n, d) > 0.0 {
        n = -n;
    }
    return n;
}

// The ray's ghost record: count, cell, t, normal (octahedral, 2 × snorm16).
fn write_ghost(i: u32) {
    if P.ghost_out == NONE {
        return;
    }
    let n = g_ghost_n / max(abs(g_ghost_n.x) + abs(g_ghost_n.y) + abs(g_ghost_n.z), 1e-20);
    var e = n.xy;
    if n.z < 0.0 {
        e = (1.0 - abs(n.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
    }
    let b = P.ghost_out + 4u * i;
    hits[b] = g_ghosts;
    hits[b + 1u] = g_ghost_cell;
    hits[b + 2u] = bitcast<u32>(g_ghost_t);
    hits[b + 3u] = pack2x16snorm(e);
}

fn reset_ghosts() {
    g_veiled = false;
    g_ghosts = 0u;
    g_ghost_cell = NONE;
    g_ghost_t = BIG;
    g_ghost_n = vec3<f32>(0.0);
}

fn write_hit(i: u32, t: f32, status: u32, depth: u32, steps: u32, lsurf: u32, ssurf: u32, d: vec3<f32>) {
    var cell = NONE;
    if depth > 0u && status != ST_MISS && status != ST_STEPS {
        cell = lv_cell[depth - 1u];
    }
    var n = vec3<f32>(0.0);
    if status != ST_MISS && status != ST_STEPS {
        n = surface_normal(ssurf, lsurf, t, d);
    }
    if STATS {
        n = vec3<f32>(f32(n_exit_words), f32(n_eval_words), f32(n_candidates));
    }
    var instance = NONE;
    if status == ST_INSIDE && depth > 0u {
        instance = cell_instance(depth - 1u);
    }
    if STATS {
        instance = max(n_step_max, g_work - n_step_start);
    }
    let b = HIT_WORDS * i;
    hits[b] = bitcast<u32>(t);
    hits[b + 1u] = cell;
    hits[b + 2u] = ssurf;
    hits[b + 3u] = status | (min(depth, 63u) << 8u) | (u32(g_veiled) << 15u) | (min(steps, 65535u) << 16u);
    hits[b + 4u] = bitcast<u32>(n.x);
    hits[b + 5u] = bitcast<u32>(n.y);
    hits[b + 6u] = bitcast<u32>(n.z);
    hits[b + 7u] = select(NONE, lv_cell[0], depth > 0u);
    hits[b + 8u] = instance;
}
