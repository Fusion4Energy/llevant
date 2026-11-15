// First hits of rays in a compiled llevant model, in f32.
//
// Per ray: locate the start point through the universe levels (octree leaf or
// BVH, then the first candidate whose program is true); while the path is
// hidden, jump to the nearest exit among the path's cells (the first crossing
// of a cell's own surfaces, or of a lattice element's faces) and locate again
// from the level that changed. Levels above it keep their cells and exits.
// In an octree, candidates are evaluated with their programs specialized to
// the leaf (`Export::grid_progs`), and a cell's exit walks the leaves along
// the ray with the cell's program in each, until one of its surfaces is
// crossed (or the cell or the octree ends).
//
// The shader is the files of `shader/` joined in order (`SHADER` in
// `pack.rs`); `{{NAME}}` placeholders are filled in there from the model.

const MAXL: u32 = {{MAXL}}u;
// Debug pipeline: ray 0 also writes one record per step into the hit slots
// after its own (see `GpuModel::debug_steps`).
override DEBUG_STEPS: bool = false;
// Statistics pipeline: the normal of each hit is replaced by per-ray work
// counters (program words walked for exits, words evaluated while locating,
// candidate cells tried), as f32, and its instance by the most program
// words one locate step walked.
override STATS: bool = false;
var<private> n_exit_words: u32;
var<private> n_eval_words: u32;
var<private> n_candidates: u32;
var<private> n_step_start: u32;
var<private> n_step_max: u32;
// Program words walked by the current ray, against `P.max_work`.
var<private> g_work: u32;
const CODE_STACK: u32 = {{CODE_STACK}}u;
const BVH_STACK: u32 = {{BVH_STACK}}u;

const NONE: u32 = 0xffffffffu;
const NODE: u32 = 0x80000000u;
const OR: u32 = 0x40000000u;
const POS: u32 = 0x40000000u;
const MASK: u32 = 0x3fffffffu;
const FILL_LATTICE: u32 = 0x80000000u;
const ELEMENT_OWN: u32 = 0xfffffffeu;
// `grid_progs` entry of a candidate true over its whole leaf (NONE: its
// whole program counts).
const PROG_TRUE: u32 = 0xfffffffeu;
// Leaves a cell's exit walks with residual programs before it takes the
// rest with the whole program (one long jump, as across the void around a
// model, where leaves are many and small).
override MAX_LEAVES: u32 = 64u;
// Programs specialized to octree leaves; false: whole programs (A/B runs).
override USE_RESIDUALS: bool = true;
// A cell's exit where its program can first turn false (`exit_bound`), not
// at the first crossing of any of its surfaces; false: the first crossing
// (A/B runs).
override EXIT_BOUND: bool = true;
const CELL_UNSUPPORTED: u32 = 1u;
// `find_cell` result: no supported cell, but this unsupported one may hold it.
const PROBLEM: u32 = 0x80000000u;
const BIG: f32 = 1e30;

// Hit statuses (low byte of word 3 of a hit).
const ST_INSIDE: u32 = 0u;
const ST_GAP: u32 = 1u;
const ST_PROBLEM: u32 = 2u;
const ST_MISS: u32 = 3u;
const ST_STEPS: u32 = 4u;
const ST_TOO_DEEP: u32 = 5u;
// Internal: the path ends in a hidden cell.
const ST_HIDDEN: u32 = 6u;

// Record sizes in `mdata` words (see `lib.rs`).
const CELL_WORDS: u32 = 13u;
const UNIT_WORDS: u32 = 20u;
const BVH_NODE_WORDS: u32 = 8u;
const BVH_ITEM_WORDS: u32 = 8u;
const TARGET_WORDS: u32 = 16u;
const LATTICE_WORDS: u32 = 42u;
const PLANE_WORDS: u32 = 5u;
// Words per hit (`llevant_gpu::HIT_WORDS`).
const HIT_WORDS: u32 = 9u;

struct Params {
    shape_info: u32,
    cells: u32,
    units: u32,
    bvh_nodes: u32,
    bvh_items: u32,
    targets: u32,
    lattices: u32,
    planes: u32,
    elements: u32,
    n_rays: u32,
    max_steps: u32,
    // Camera mode: regions of `cut` (region r's planes end at cut_end(r)).
    cut_regions: u32,
    eps_abs: f32,
    eps_rel: f32,
    width: u32,
    height: u32,
    // Where the shapes, node boxes, programs and octree start in the pool
    // (the records start at 0).
    shapes: u32,
    nboxes: u32,
    code: u32,
    grid: u32,
    // Rays in flight at most (a multiple of 64): the size of each half of
    // `states`.
    capacity: u32,
    // Program words a ray walks in one slice before it pauses (its state
    // kept in `states` for the next slice); u32 max: no pause.
    slice_work: u32,
    // Camera mode: blocks of 8 × 8 pixels in the job (see `ctl`).
    n_blocks: u32,
    tail_work: u32,
    // Program words a ray may walk (locating and exits) before it gives up
    // like after `max_steps`: bounds the time of one ray whatever the
    // model, far below the GPU timeout.
    max_work: u32,
    // Programs of the octree candidates: entry `grid_progs + first + i` for
    // candidate `first + i` of a leaf (see `lib.rs`).
    grid_progs: u32,
    // Where each ray's ghost record (`write_ghost`) goes in `hits`, after
    // the hits; NONE: no ghosts, nothing written.
    ghost_out: u32,
    // Where the unit boxes, octree-node bits, ghost bits and the flags
    // start in `vis` (see visibility.wgsl).
    vis_boxes: u32,
    vis_nodes: u32,
    vis_ghosts: u32,
    vis_flags: u32,
    pad0: u32,
    // Camera mode: eye (w: reach), forward (w: tan of half the vertical field
    // of view), right (w: half the height of an orthographic view, 0 through
    // a pinhole) and up unit vectors.
    eye: vec4<f32>,
    fwd: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    // The cut (`llevant::Cutaway`): planes region after region, each a unit
    // normal (into the space removed) and, w, the eye's signed distance
    // from the plane; where each region's planes end.
    cut: array<vec4<f32>, 16>,
    cut_ends: array<vec4<u32>, 2>,
}

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> vis: array<u32>;
// 8 floats per ray: origin, t_min, direction, t_max.
@group(0) @binding(2) var<storage, read> rays: array<f32>;
// 9 words per ray: t, cell, surface, status | depth << 8 | veiled gap << 15
// | steps << 16,
// normal (3 × f32), level-0 cell, cell instance. With ghosts, from
// `P.ghost_out` on, 4 more per ray (`write_ghost`).
@group(0) @binding(3) var<storage, read_write> hits: array<u32>;
// The job's control words (`C_*`; the first three are the next slice's
// indirect dispatch), then per block of 8 × 8 pixels the rays of the camera
// job left in it, then the blocks in the order traced (x | y << 16).
@group(0) @binding(4) var<storage, read_write> ctl: array<atomic<u32>>;
// Paused rays: two halves of `capacity` states of STATE_WORDS words, one
// read by a slice while the other takes the rays it pauses.
@group(0) @binding(5) var<storage, read_write> states: array<u32>;
// The model: one pool of words (records, shapes, node boxes, programs,
// octrees) split into bindings of 2^POOL_SHIFT words, so that no binding
// exceeds the device's limit.
{{POOL_BINDINGS}}
fn word(i: u32) -> u32 {
{{POOL_BODY}}
}

// Per level of the current path: the ray in the level's frame
// (`p = o + t·d`), the rotation from the global frame, the unit, the cell
// (NONE: a gap), the lattice element entered (lattice cells), and the cached
// exit of the cell (or lattice element) with the surface crossed there.
var<private> lv_o: array<vec3<f32>, MAXL>;
var<private> lv_d: array<vec3<f32>, MAXL>;
var<private> lv_r: array<mat3x3<f32>, MAXL>;
var<private> lv_unit: array<u32, MAXL>;
var<private> lv_cell: array<u32, MAXL>;
var<private> lv_ix: array<vec3<i32>, MAXL>;
// The instance of the level's unit (`llevant::instance`), NONE when it
// cannot be numbered.
var<private> lv_inst: array<u32, MAXL>;
var<private> lv_exit: array<f32, MAXL>;
var<private> lv_exit_surf: array<u32, MAXL>;
var<private> g_tmax: f32;
// Estimated f32 error (cm) of the last crossing a `*_next` function
// accepted; the exits keep the one of their crossing.
var<private> g_root_err: f32;
var<private> lv_exit_err: array<f32, MAXL>;
// Box of the octree leaf the last `grid_leaf` found.
var<private> g_llo: vec3<f32>;
var<private> g_lhi: vec3<f32>;
// Its node.
var<private> g_lnode: u32;
// The octree leaf node of the last `find_cell`'s point, NONE if it used no
// octree: the bottom level's after `locate`.
var<private> g_found_leaf: u32;
// The ghost boundaries crossed so far (`llevant::Ghost`): how many, and the
// first one's ghost cell, t and normal; the innermost ghost cell of the
// path where the ray is (NONE: in none).
var<private> g_ghosts: u32;
var<private> g_ghost_cell: u32;
var<private> g_ghost_t: f32;
var<private> g_ghost_n: vec3<f32>;
var<private> g_ghost_in: u32;
// Whether the ray went through a veiled gap (`llevant::Veils`), written
// with its hit; whether gaps are veils at all (`veils_gaps`, read once a
// ray, not at every step).
var<private> g_veiled: bool;
var<private> g_veil_on: bool;

// Per open program node: where it ends, with `STACK_OR` set for an OR (one
// word: private arrays cost every ray of a dispatch).
var<private> cstack_end: array<u32, CODE_STACK>;
const STACK_OR: u32 = 0x80000000u;
// `exit_bound`'s per open node: until where it is surely true from the
// ray's start (`NOT_SURE`: not surely true); the surface crossed there goes
// in `bstack`, free while a cell's exit is found. One word a node: every
// word here is held for every ray of a dispatch (four cost the views that
// never step ~20 %).
var<private> cstack_bound: array<f32, CODE_STACK>;
const NOT_SURE: f32 = -1e30;
// BVH nodes to visit; also `exit_bound`'s surfaces per open program node
// (`BVH_STACK` is at least `CODE_STACK`).
var<private> bstack: array<u32, BVH_STACK>;

fn cw(i: u32) -> u32 {
    return word(P.code + i);
}

fn gw(i: u32) -> u32 {
    return word(P.grid + i);
}

fn sf(i: u32) -> f32 {
    return bitcast<f32>(word(P.shapes + i));
}

fn nf(i: u32) -> f32 {
    return bitcast<f32>(word(P.nboxes + i));
}

fn mf(i: u32) -> f32 {
    return bitcast<f32>(word(i));
}

fn mv3(i: u32) -> vec3<f32> {
    return vec3<f32>(mf(i), mf(i + 1u), mf(i + 2u));
}

fn sv3(i: u32) -> vec3<f32> {
    return vec3<f32>(sf(i), sf(i + 1u), sf(i + 2u));
}

fn nv3(i: u32) -> vec3<f32> {
    return vec3<f32>(nf(i), nf(i + 1u), nf(i + 2u));
}

fn in_box(lo: vec3<f32>, hi: vec3<f32>, p: vec3<f32>) -> bool {
    return all(p >= lo) && all(p <= hi);
}

// 1/d with zero components replaced by a huge value of either sign, so slab
// tests never produce NaN.
fn safe_inv(d: vec3<f32>) -> vec3<f32> {
    return select(1.0 / d, vec3<f32>(BIG), d == vec3<f32>(0.0));
}

// Range of t in [a, b] where `o + t·d` is inside the box (empty: x > y).
fn ray_box(lo: vec3<f32>, hi: vec3<f32>, o: vec3<f32>, inv: vec3<f32>, a: f32, b: f32) -> vec2<f32> {
    let t1 = (lo - o) * inv;
    let t2 = (hi - o) * inv;
    let tn = min(t1, t2);
    let tf = max(t1, t2);
    return vec2<f32>(max(a, max(tn.x, max(tn.y, tn.z))), min(b, min(tf.x, min(tf.y, tf.z))));
}
