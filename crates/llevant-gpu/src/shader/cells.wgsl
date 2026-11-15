// ---- cells ---------------------------------------------------------------

fn cell_base(c: u32) -> u32 {
    return P.cells + CELL_WORDS * c;
}

// Value of cell `c`'s program at `p`; node boxes only where they are exact.
fn eval_cell(c: u32, p: vec3<f32>, use_boxes: bool) -> bool {
    let cb = cell_base(c);
    if word(cb + 7u) == 0u {
        return false;
    }
    return eval_prog(word(cb + 6u), word(cb + 12u), p, use_boxes);
}

// Words of the program (a leaf or a node) starting at `pos`.
fn prog_len(pos: u32) -> u32 {
    let w = cw(pos);
    return select(1u, 1u + (w & MASK), (w & NODE) != 0u);
}

// Value at `p` of the program starting at `start`, of a cell whose node
// boxes start at `nbase`.
fn eval_prog(start: u32, nbase: u32, p: vec3<f32>, use_boxes: bool) -> bool {
    var pos = start;
    var sp = 0u;
    var v = false;
    if STATS {
        n_candidates += 1u;
    }
    loop {
        g_work += 1u;
        if STATS {
            n_eval_words += 1u;
        }
        let w = cw(pos);
        if (w & NODE) == 0u {
            v = side_pos(w & MASK, p) == ((w & POS) != 0u);
            pos += 1u;
        } else {
            let end = pos + 1u + (w & MASK);
            let bw = cw(pos + 1u);
            var decided = false;
            if use_boxes && bw != NONE {
                let b = (nbase + (bw & ~NODE)) * 12u;
                if !in_box(nv3(b), nv3(b + 3u), p) {
                    v = false;
                    decided = true;
                } else if !in_box(nv3(b + 6u), nv3(b + 9u), p) {
                    v = true;
                    decided = true;
                }
            }
            if decided {
                pos = end;
            } else {
                cstack_end[sp] = end | select(0u, STACK_OR, (w & OR) != 0u);
                sp += 1u;
                pos += 2u;
                continue;
            }
        }
        // Hand `v` to the enclosing nodes: an OR is decided by a true child,
        // an AND by a false one; a node whose children are all done has the
        // last child's value.
        loop {
            if sp == 0u {
                break;
            }
            let top = sp - 1u;
            let top_end = cstack_end[top] & ~STACK_OR;
            if v == ((cstack_end[top] & STACK_OR) != 0u) {
                pos = top_end;
                sp = top;
                continue;
            }
            if pos >= top_end {
                sp = top;
                continue;
            }
            break;
        }
        if sp == 0u {
            return v;
        }
    }
    return v;
}

// 0: not in the cell, 1: in it, 2: unsupported (cannot tell).
fn try_cell(c: u32, p: vec3<f32>, use_boxes: bool) -> u32 {
    if (word(cell_base(c) + 9u) & CELL_UNSUPPORTED) != 0u {
        return 2u;
    }
    return select(0u, 1u, eval_cell(c, p, use_boxes));
}

fn cell_bbox_has(c: u32, p: vec3<f32>) -> bool {
    let cb = cell_base(c);
    return in_box(mv3(cb), mv3(cb + 3u), p);
}

fn is_lattice(c: u32) -> bool {
    let f = word(cell_base(c) + 8u);
    return f != NONE && (f & FILL_LATTICE) != 0u;
}

// The octree leaf of unit record `ub` holding `p` (inside its box), split
// at box centres as in the build: its first candidate (an offset in the
// octree section) and their number; its box in `g_llo` / `g_lhi`.
fn grid_leaf(ub: u32, p: vec3<f32>) -> vec2<u32> {
    var lo = mv3(ub + 6u);
    var hi = mv3(ub + 9u);
    var n = word(ub + 17u);
    loop {
        if gw(2u * n + 1u) != NONE {
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
    g_llo = lo;
    g_lhi = hi;
    g_lnode = n;
    return vec2<u32>(gw(2u * n), gw(2u * n + 1u));
}

// The cell of `unit` holding `p`: the first candidate whose program is true,
// NONE for a gap, or PROBLEM | c when only an unsupported cell may hold it.
fn find_cell(unit: u32, p: vec3<f32>) -> u32 {
    g_found_leaf = NONE;
    let ub = P.units + UNIT_WORDS * unit;
    let in_roi = in_box(mv3(ub), mv3(ub + 3u), p);
    var unsup = NONE;
    if !in_roi {
        // Beyond the region (level 0 only): every cell, or beyond the problem
        // box its IMP:N=0 cells.
        let s = word(ub + 12u);
        let n = word(ub + 13u);
        for (var i = 0u; i < n; i++) {
            let c = word(s + i);
            let r = try_cell(c, p, false);
            if r == 1u {
                return c;
            }
            if r == 2u {
                unsup = c;
            }
        }
    } else if word(ub + 17u) != NONE && in_box(mv3(ub + 6u), mv3(ub + 9u), p) {
        let leaf = grid_leaf(ub, p);
        g_found_leaf = g_lnode;
        let first = leaf.x;
        let len = leaf.y;
        for (var i = 0u; i < len; i++) {
            let c = gw(first + i);
            if !cell_bbox_has(c, p) {
                continue;
            }
            // The candidate's program specialized to this leaf.
            var pr = NONE;
            if USE_RESIDUALS {
                pr = word(P.grid_progs + first + i);
            }
            var r = 0u;
            if pr == NONE {
                r = try_cell(c, p, true);
            } else if pr == PROG_TRUE {
                r = 1u;
            } else {
                r = select(0u, 1u, eval_prog(pr, word(cell_base(c) + 12u), p, true));
            }
            if r == 1u {
                return c;
            }
            if r == 2u {
                unsup = c;
            }
        }
    } else if word(ub + 16u) != NONE {
        bstack[0] = word(ub + 16u);
        var sp = 1u;
        while sp > 0u {
            sp -= 1u;
            let nb = P.bvh_nodes + BVH_NODE_WORDS * bstack[sp];
            if !in_box(mv3(nb), mv3(nb + 3u), p) {
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
                if !in_box(mv3(ib + 1u), mv3(ib + 4u), p) {
                    continue;
                }
                let c = word(ib);
                let r = try_cell(c, p, true);
                if r == 1u {
                    return c;
                }
                if r == 2u {
                    unsup = c;
                }
            }
        }
    }
    // Cells checked everywhere: lattice cells fill all of their universe.
    let s = word(ub + 14u);
    let n = word(ub + 15u);
    for (var i = 0u; i < n; i++) {
        let c = word(s + i);
        if is_lattice(c) {
            return c;
        }
        let r = try_cell(c, p, in_roi);
        if r == 1u {
            return c;
        }
        if r == 2u {
            unsup = c;
        }
    }
    if unsup != NONE {
        return PROBLEM | unsup;
    }
    return NONE;
}
