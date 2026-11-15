// ---- slices --------------------------------------------------------------
//
// A job's rays are traced a slice at a time: each slice runs every ray in
// flight until it ends or has walked `P.slice_work` more program words
// (checked between steps; more when few rays are left, `plan`), so that no
// dispatch runs long whatever the rays.
// Rays go by blocks of 64 (8 × 8 pixels of a camera's image), one block
// per workgroup, each ray keeping its lane from slice to slice: rays of a
// block take alike paths, and a lane whose ray ended costs nothing while
// the GPU has room. Each slice runs only the workgroups with rays left; a
// workgroup whose rays all ended takes the job's next block. A paused ray
// keeps the state of its trace loop in `states` and, resumed, locates
// itself again from level 0 at the point it had reached.

// Control words of a job (`ctl`).
const C_COUNT: u32 = 3u; // workgroups in list 0 and list 1 (2 words)
const C_PARITY: u32 = 5u; // the list the current slice runs
const C_CURSOR: u32 = 6u; // the next block
const C_TOTAL: u32 = 7u; // rays in the job (blocks: rounded up to 64)
const C_WORK: u32 = 8u; // words each ray may walk in the current slice
const C_DONE: u32 = 11u; // rays finished
const C_BLOCKS: u32 = 12u;
// A workgroup's block starts in the next slice.
const FRESH: u32 = 0x80000000u;

// Words of a paused ray's state.
const STATE_WORDS: u32 = 20u;

// Where things are in `ctl` after the blocks: two lists of workgroups (the
// one the slice runs, the one it fills for the next), then each
// workgroup's block.
fn ctl_list(p: u32) -> u32 {
    return C_BLOCKS + 2u * P.n_blocks + p * (P.capacity / 64u);
}

fn ctl_group_block(g: u32) -> u32 {
    return C_BLOCKS + 2u * P.n_blocks + 2u * (P.capacity / 64u) + g;
}

// The trace loop's state (see `trace_run`): where the ray is located, the
// crossing before it, the level to locate from, the level and shape of the
// surface last crossed, and the locate steps taken.
var<private> r_t: f32;
var<private> r_tc: f32;
var<private> r_start: u32;
var<private> r_hl: u32;
var<private> r_hs: u32;
var<private> r_step: u32;
// The ray was just resumed: its next locate starts from level 0.
var<private> r_resumed: bool;

// Level 0 of the ray `o + t·d`, located from.
fn ray_root(o: vec3<f32>, d: vec3<f32>) {
    lv_o[0] = o;
    lv_d[0] = d;
    lv_r[0] = mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0));
    lv_unit[0] = 0u;
    lv_inst[0] = 0u;
}

// Start tracing `o + t·d` from `tmin` to `tmax`.
fn trace_begin(o: vec3<f32>, d: vec3<f32>, tmin: f32, tmax: f32) {
    g_tmax = tmax;
    n_exit_words = 0u;
    n_eval_words = 0u;
    n_candidates = 0u;
    n_step_start = 0u;
    n_step_max = 0u;
    g_work = 0u;
    g_veil_on = veils_gaps();
    ray_root(o, d);
    r_t = tmin;
    r_tc = tmin;
    r_start = 0u;
    r_hl = 0u;
    r_hs = NONE;
    r_step = 0u;
    r_resumed = false;
}

// Trace on until the ray ends (its hit written for ray `i`: true) or has
// walked to `work_end` words (false: paused, to resume with `trace_load`).
fn trace_run(i: u32, d: vec3<f32>, work_end: u32) -> bool {
    for (; r_step < P.max_steps; r_step++) {
        if g_work >= P.max_work {
            break;
        }
        if STATS {
            n_step_max = max(n_step_max, g_work - n_step_start);
            n_step_start = g_work;
        }
        if g_work >= work_end {
            return false;
        }
        // A step costs a word even when it walks none.
        g_work += 1u;
        // Locating a resumed ray again from level 0 costs it nothing, so
        // that slicing does not change where `max_work` stops a ray.
        let work_before = g_work;
        let loc = locate(r_start, r_t);
        if r_resumed {
            g_work = work_before;
            r_resumed = false;
        }
        // A gap is seen through with veils (`llevant::Visibility::veil`):
        // stepped through as a hidden stretch, and said with the hit.
        let veiled = g_veil_on && loc.status == ST_GAP;
        g_veiled = g_veiled || veiled;
        // A change of the innermost ghost cell is a ghost boundary (the
        // first step only says where the ray starts).
        if P.ghost_out != NONE {
            let g = innermost_ghost(loc.depth);
            if r_step > 0u && g != g_ghost_in {
                if g_ghosts == 0u {
                    g_ghost_cell = select(g_ghost_in, g, g != NONE);
                    g_ghost_t = r_tc;
                    g_ghost_n = surface_normal(r_hs, r_hl, r_tc, d);
                }
                g_ghosts += 1u;
            }
            g_ghost_in = g;
        }
        if DEBUG_STEPS && i == 0u && HIT_WORDS * (r_step + 2u) <= arrayLength(&hits) {
            let b = HIT_WORDS * (r_step + 1u);
            hits[b] = bitcast<u32>(r_t);
            hits[b + 1u] = bitcast<u32>(r_tc);
            hits[b + 2u] = r_start | (loc.depth << 8u) | (loc.status << 16u);
            hits[b + 3u] = r_hs;
            for (var l = 0u; l < 4u; l++) {
                hits[b + 4u + l] = select(NONE, lv_cell[l], l < loc.depth);
            }
        }
        if loc.status != ST_HIDDEN && !veiled {
            write_hit(i, r_tc, loc.status, loc.depth, r_step, r_hl, r_hs, d);
            return true;
        }
        let t = r_t;
        let tmax = g_tmax;
        // Exits of the levels that changed; a hidden lattice cell at the
        // bottom fills all of its universe, so its parent bounds it.
        for (var l = r_start; l < loc.depth; l++) {
            var e = vec3<u32>(bitcast<u32>(BIG), NONE, 0u);
            if lv_cell[l] == NONE {
                // A veiled gap.
                e = gap_exit(l, t);
            } else if is_lattice(lv_cell[l]) {
                // A visible lattice cell at the bottom is in an element that
                // shows nothing itself (its own material, a ghost or above
                // the level shown): the next element may.
                if l + 1u < loc.depth || visible(lv_cell[l]) {
                    e = element_exit(l, t);
                }
            } else {
                // The hidden cell at the bottom: jump over what holds nothing
                // visible, else its own exit. Levels above are visible cells
                // (or lattice elements) whose exits bound the jump.
                var s = -1.0;
                if l + 1u == loc.depth {
                    var lim = tmax;
                    for (var u = 0u; u < l; u++) {
                        lim = min(lim, lv_exit[u]);
                    }
                    s = skip_exit(l, t, lim);
                }
                if s >= 0.0 {
                    e = vec3<u32>(bitcast<u32>(s), NONE, 0u);
                } else {
                    e = cell_exit(l, t);
                }
            }
            lv_exit[l] = bitcast<f32>(e.x);
            lv_exit_surf[l] = e.y;
            lv_exit_err[l] = bitcast<f32>(e.z);
        }
        var best = BIG;
        for (var l = 0u; l < loc.depth; l++) {
            best = min(best, lv_exit[l]);
        }
        if best >= tmax {
            write_hit(i, tmax, ST_MISS, 0u, r_step, 0u, NONE, d);
            return true;
        }
        // The surface crossed: of coincident ones, the highest level's.
        let eps = P.eps_abs + P.eps_rel * abs(best);
        var root_err = 0.0;
        for (var l = 0u; l < loc.depth; l++) {
            if lv_exit[l] <= best + eps {
                r_hl = l;
                r_hs = lv_exit_surf[l];
                root_err = lv_exit_err[l];
                break;
            }
        }
        // Step `eps` past the surface along its normal: a grazing ray needs
        // a longer step to get clear of the surface's f32 rounding.
        var along = eps;
        if r_hs != NONE {
            let g = gradient(r_hs, lv_o[r_hl] + best * lv_d[r_hl]);
            let gl = length(g);
            if gl > 0.0 {
                along = eps / max(abs(dot(g, lv_d[r_hl])) / gl, 0.05);
            }
        }
        // Also clear of the crossing's own f32 error (along the ray).
        along = max(along, 4.0 * root_err);
        r_tc = best;
        r_t = best + along;
        // Relocate from the highest level whose cell the step leaves.
        for (var l = 0u; l < loc.depth; l++) {
            if lv_exit[l] <= r_t {
                r_start = l;
                break;
            }
        }
    }
    write_hit(i, r_t, ST_STEPS, 0u, r_step, 0u, NONE, d);
    return true;
}

// The last word a slice with `left` words to go lets the trace reach.
fn work_end(left: u32) -> u32 {
    return select(g_work + left, 0xffffffffu, left > 0xffffffffu - g_work);
}

// The thread's place in `states`, and whether its ray paused this slice.
var<private> s_slot: u32;
var<private> s_paused: bool;

// Pause the ray `r` of the job: its trace state, `extra` (the camera's cut
// part and cap) and `t_end` (the end of the part traced) into its lane's
// state.
fn trace_save(r: u32, extra: u32, t_end: f32) {
    s_paused = true;
    let b = s_slot * STATE_WORDS;
    states[b] = r;
    states[b + 1u] = bitcast<u32>(r_t);
    states[b + 2u] = bitcast<u32>(r_tc);
    states[b + 3u] = bitcast<u32>(t_end);
    states[b + 4u] = r_hl | (extra << 8u);
    states[b + 5u] = r_hs;
    states[b + 6u] = r_step;
    states[b + 7u] = g_work;
    states[b + 8u] = g_ghosts;
    states[b + 9u] = g_ghost_cell;
    states[b + 10u] = bitcast<u32>(g_ghost_t);
    states[b + 11u] = bitcast<u32>(g_ghost_n.x);
    states[b + 12u] = bitcast<u32>(g_ghost_n.y);
    states[b + 13u] = bitcast<u32>(g_ghost_n.z);
    states[b + 14u] = g_ghost_in;
    states[b + 15u] = n_exit_words;
    states[b + 16u] = n_eval_words;
    states[b + 17u] = n_candidates;
    states[b + 18u] = n_step_max;
    states[b + 19u] = u32(g_veiled);
}

// The lane's paused ray: its trace state back (located again from level 0
// by the next step), and its ray index, the `extra` word and the end of
// its part.
struct Saved {
    r: u32,
    extra: u32,
    t_end: f32,
}

fn trace_load() -> Saved {
    let b = s_slot * STATE_WORDS;
    r_t = bitcast<f32>(states[b + 1u]);
    r_tc = bitcast<f32>(states[b + 2u]);
    g_tmax = bitcast<f32>(states[b + 3u]);
    r_hl = states[b + 4u] & 0xffu;
    r_hs = states[b + 5u];
    r_step = states[b + 6u];
    g_work = states[b + 7u];
    g_ghosts = states[b + 8u];
    g_ghost_cell = states[b + 9u];
    g_ghost_t = bitcast<f32>(states[b + 10u]);
    g_ghost_n = vec3<f32>(bitcast<f32>(states[b + 11u]), bitcast<f32>(states[b + 12u]), bitcast<f32>(states[b + 13u]));
    g_ghost_in = states[b + 14u];
    n_exit_words = states[b + 15u];
    n_eval_words = states[b + 16u];
    n_candidates = states[b + 17u];
    n_step_max = states[b + 18u];
    g_veiled = states[b + 19u] != 0u;
    g_veil_on = veils_gaps();
    n_step_start = g_work;
    r_start = 0u;
    r_resumed = true;
    return Saved(states[b], states[b + 4u] >> 8u, g_tmax);
}

// Before each slice: the list of workgroups the last slice left is the
// one this slice runs.
@compute @workgroup_size(1)
fn plan() {
    let p = 1u - atomicLoad(&ctl[C_PARITY]);
    atomicStore(&ctl[C_COUNT + 1u - p], 0u);
    atomicStore(&ctl[C_PARITY], p);
    let groups = atomicLoad(&ctl[C_COUNT + p]);
    atomicStore(&ctl[0], groups);
    // A slice takes about (rays in flight) × (words each) once the rays
    // fill the GPU: with fewer, each may walk further in the same time (up
    // to `P.tail_work`), so that the tail of a job takes fewer slices.
    let more = clamp(FULL_GROUPS / max(groups, 1u), 1u, 64u);
    // (No limit stays none; `min`: no overflow, where NVIDIA's compiler
    // failed on a division.)
    let words = max(min(min(P.slice_work, 0x3ffffffu) * more, P.tail_work), P.slice_work);
    atomicStore(&ctl[C_WORK], select(words, NONE, P.slice_work == NONE));
}

// Workgroups that keep a GPU busy (32 768 rays; an RTX 3060 Laptop runs
// 46 080 threads at once).
const FULL_GROUPS: u32 = 512u;

// Words each ray may walk in the current slice.
fn slice_words() -> u32 {
    return atomicLoad(&ctl[C_WORK]);
}

// The workgroup of the slice: its place among the job's (NONE: none, the
// slice has fewer) and its block.
var<workgroup> wg_group: u32;
var<workgroup> wg_block: u32;
var<workgroup> wg_live: atomic<u32>;

// What a lane does this slice: start ray `index` of the job (`fresh`),
// resume its paused ray, or nothing.
struct Lane {
    ok: bool,
    fresh: bool,
    index: u32,
}

// The slice's workgroup `w` and lane `lid`: what to trace.
fn lane(w: u32, lid: u32) -> Lane {
    if lid == 0u {
        wg_group = NONE;
        let p = atomicLoad(&ctl[C_PARITY]);
        if w < atomicLoad(&ctl[C_COUNT + p]) {
            let g = atomicLoad(&ctl[ctl_list(p) + w]);
            wg_group = g;
            wg_block = atomicLoad(&ctl[ctl_group_block(g)]);
        }
    }
    let g = workgroupUniformLoad(&wg_group);
    if g == NONE {
        return Lane(false, false, 0u);
    }
    s_slot = 64u * g + lid;
    let blk = wg_block;
    if (blk & FRESH) != 0u {
        let r = 64u * (blk & ~FRESH) + lid;
        return Lane(r < atomicLoad(&ctl[C_TOTAL]), true, r);
    }
    let r = states[s_slot * STATE_WORDS];
    return Lane(r != NONE, false, r);
}

// After every lane's ray, in uniform control: a ray that ended frees its
// lane; a workgroup with paused rays runs again in the next slice, one
// whose rays all ended takes the job's next block.
fn slice_end(lid: u32, l: Lane) {
    if l.ok && !s_paused {
        states[s_slot * STATE_WORDS] = NONE;
    }
    if !l.ok && l.fresh {
        // No ray past the job's end.
        states[s_slot * STATE_WORDS] = NONE;
    }
    if s_paused {
        atomicAdd(&wg_live, 1u);
    }
    workgroupBarrier();
    if lid != 0u || wg_group == NONE {
        return;
    }
    let p = atomicLoad(&ctl[C_PARITY]);
    var blk = wg_block & ~FRESH;
    if atomicLoad(&wg_live) == 0u {
        blk = atomicAdd(&ctl[C_CURSOR], 1u);
        if blk >= (atomicLoad(&ctl[C_TOTAL]) + 63u) / 64u {
            return;
        }
        blk |= FRESH;
    }
    atomicStore(&ctl[ctl_group_block(wg_group)], blk);
    let k = atomicAdd(&ctl[C_COUNT + 1u - p], 1u);
    atomicStore(&ctl[ctl_list(1u - p) + k], wg_group);
}

@compute @workgroup_size(64)
fn trace_rays(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) lid: u32, @builtin(num_workgroups) nw: vec3<u32>) {
    s_paused = false;
    let l = lane(wid.x + wid.y * nw.x, lid);
    if l.ok {
        trace_listed(l);
    }
    slice_end(lid, l);
}

fn trace_listed(l: Lane) {
    var r = l.index;
    if !l.fresh {
        r = trace_load().r;
    }
    let b = 8u * r;
    let o = vec3<f32>(rays[b], rays[b + 1u], rays[b + 2u]);
    let d = vec3<f32>(rays[b + 4u], rays[b + 5u], rays[b + 6u]);
    if l.fresh {
        reset_ghosts();
        trace_begin(o, d, rays[b + 3u], rays[b + 7u]);
    } else {
        ray_root(o, d);
    }
    if !trace_run(r, d, work_end(slice_words())) {
        trace_save(r, 0u, g_tmax);
        return;
    }
    write_ghost(r);
    atomicAdd(&ctl[C_DONE], 1u);
}

// Camera mode: ray `r` of the job is pixel `r % 64` (row-major) of block
// `r / 64` of the job's order.
fn job_pixel(r: u32) -> vec2<u32> {
    let blk = atomicLoad(&ctl[C_BLOCKS + P.n_blocks + r / 64u]);
    let q = r % 64u;
    return vec2<u32>((blk & 0xffffu) * 8u + q % 8u, (blk >> 16u) * 8u + q / 8u);
}

// Ray `r` of a camera job is done: one fewer left in its block.
fn finish_camera_ray(r: u32) {
    atomicSub(&ctl[C_BLOCKS + r / 64u], 1u);
    atomicAdd(&ctl[C_DONE], 1u);
}
