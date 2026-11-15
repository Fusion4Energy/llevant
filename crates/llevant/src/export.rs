//! A flat copy of a compiled model for other evaluators (the GPU renderer).
//!
//! Everything is plain arrays of numbers and indices, so it can be packed into
//! GPU buffers without knowing llevant's types. MCNP semantics stay in the
//! compiler: transforms, `LIKE BUT`, `TRCL`, `#n` and macrobodies are already
//! resolved. Values are f64 as compiled; boxes are conservative in f64, so a
//! consumer that rounds them must round outward.
//!
//! Indices into one array are absolute (not relative to a unit), and
//! [`NONE`] marks a missing index.
//!
//! Cell programs ([`Export::code`]) are `u32` words in prefix order,
//! evaluated with short-circuit and three-valued (Kleene) logic:
//! - leaf: [`CODE_NODE`] clear; [`CODE_POS`] set when the leaf is true on
//!   the surface's positive side; `w & CODE_MASK` the shape index.
//! - node: [`CODE_NODE`] set; [`CODE_OR`] clear for AND, set for OR;
//!   `w & CODE_MASK` the number of words that follow it (its box word and
//!   its children), so a decided node skips the rest. The word after the
//!   header is a box word: [`CODE_NO_BOX`], or `CODE_NODE | k` for entry
//!   `node_start + k` of [`Export::node_boxes`], where `node_start` is that
//!   of the cell the program belongs to ([`ExportCell::node_start`]; the
//!   same for its residual programs in the octrees).
//!
//! There is no NOT: complements are pushed down to the leaves.
//!
//! The format is versioned by [`EXPORT_VERSION`]: a consumer checks it
//! against the version it was written for (`llevant-gpu` does at compile
//! time), so a change here cannot go unnoticed by it.

use rayon::prelude::*;

use crate::bbox::Aabb;
use crate::expr::{MASK, NODE};
use crate::grid::{INNER, PROG_FULL, PROG_TRUE};
use crate::lattice::{LatticeFill, Target};
use crate::model::{Fill, Model};
use crate::skip::Skip;
use crate::surface::Shape;
use crate::vec3::Vec3;

pub use crate::transform::Affine;

/// Version of this format: raised whenever the layout or the meaning of
/// anything exported changes (fields, word encodings, constants, how boxes
/// or programs are to be read).
pub const EXPORT_VERSION: u32 = 3;

/// The leaves of a cell program (`code[code_start..][..code_len]`): the
/// compiled shape of each and whether the cell wants its positive side.
pub fn leaves(code: &[u32]) -> impl Iterator<Item = (u32, bool)> + '_ {
    crate::expr::leaves(code)
}

/// A missing index.
pub const NONE: u32 = u32::MAX;

/// Program word bits (see the module documentation).
pub const CODE_NODE: u32 = crate::expr::NODE;
pub const CODE_OR: u32 = crate::expr::OR;
pub const CODE_POS: u32 = crate::expr::POS;
pub const CODE_MASK: u32 = MASK;
pub const CODE_NO_BOX: u32 = crate::expr::NO_BOX;

/// `n·p - d`, data `[nx, ny, nz, d]` with unit `n`.
pub const SHAPE_PLANE: u32 = 0;
/// `(p-o)ᵀ A (p-o) + b·(p-o) + c`, data
/// `[ox, oy, oz, axx, ayy, azz, axy, ayz, azx, bx, by, bz, c]`.
pub const SHAPE_QUADRIC: u32 = 1;
/// A one-sheet cone: the quadric's 13 values, then the apex and the unit
/// axis into the selected nappe (19 values). Negative inside the double cone
/// on the nappe side of the plane through the apex.
pub const SHAPE_CONE: u32 = 2;
/// A torus, data `[r00 … r22 (row-major), dx, dy, dz, axis, a, b, c]` (16
/// values): `q = R p + d` maps into its frame, where the axis is coordinate
/// `axis` and `(w/b)² + ((ρ - a)/c)² - 1` is the surface function.
pub const SHAPE_TORUS: u32 = 3;

/// [`ExportCell::flags`]: the cell cannot be evaluated.
pub const CELL_UNSUPPORTED: u32 = 1;
/// [`ExportCell::flags`]: `IMP:N=0`.
pub const CELL_IMP_ZERO: u32 = 2;
/// [`ExportCell::fill`]: the low bits index [`Export::lattices`] instead of
/// [`Export::targets`].
pub const FILL_LATTICE: u32 = 1 << 31;

/// [`Export::lattice_elements`]: an element filled by the lattice cell itself.
pub const ELEMENT_OWN: u32 = NONE - 1;
/// [`Export::lattice_elements`]: an element outside the fill array.
pub const ELEMENT_MISSING: u32 = NONE;

/// [`ExportTarget::instance_offset`], [`ExportCell::instance_base`]: no
/// instance can be numbered there.
pub const NO_INSTANCE: u64 = u64::MAX;

#[derive(Clone, Copy, Debug)]
pub struct ExportCell {
    /// MCNP cell number.
    pub id: i64,
    pub unit: u32,
    pub material: i64,
    /// Program words `code[code_start .. code_start + code_len]` (encoding
    /// in the [module documentation](self)).
    pub code_start: u32,
    pub code_len: u32,
    /// The first of the cell's [`Export::node_boxes`]: its box words are
    /// relative to it.
    pub node_start: u32,
    pub bbox: Aabb,
    /// [`NONE`], a target index, or `FILL_LATTICE | lattice index`.
    pub fill: u32,
    pub flags: u32,
    /// The first id of the cell's instances ([`crate::instance`]): the id
    /// of the instance holding a point is `instance_base + instance of the
    /// cell's universe · E + element`, where for a lattice cell `E` and
    /// `element` come from its lattice's [`ExportLattice::numbered`]
    /// (`E = 1`, no element otherwise).
    pub instance_base: u64,
}

/// A range `start .. start + len` of some array.
#[derive(Clone, Copy, Debug, Default)]
pub struct Span {
    pub start: u32,
    pub len: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct ExportUnit {
    /// MCNP universe number.
    pub universe: i64,
    /// The cells checked when a point lies outside `roi` (only possible at
    /// level 0), in [`Export::unit_cells`]: every cell of the universe, or
    /// beyond the problem box its `IMP:N=0` cells.
    pub cells: Span,
    /// Region reachable through the cells filled with this universe. Node
    /// boxes are only exact inside it.
    pub roi: Aabb,
    /// Root of the unit's BVH in [`Export::bvh_nodes`], or [`NONE`].
    pub bvh_root: u32,
    /// Root of the unit's octree in [`Export::grid_nodes`], or [`NONE`].
    /// Inside `grid_box` the octree leaf replaces the BVH.
    pub grid_root: u32,
    pub grid_box: Aabb,
    /// Cells checked for every point (lattice cells), in
    /// [`Export::always`].
    pub always: Span,
}

/// A BVH node: a leaf when `count > 0` (items
/// `bvh_items[start .. start + count]`), otherwise an inner node whose
/// children are `start` and `start + 1`.
#[derive(Clone, Copy, Debug)]
pub struct ExportBvhNode {
    pub bbox: Aabb,
    pub start: u32,
    pub count: u32,
}

/// An octree node: inner when `len == GRID_INNER`, with children
/// `first .. first + 8` (bit `a` of the child number set: upper half along
/// axis `a`, split at the centre of the box as `(lo + hi) / 2`); otherwise a
/// leaf with candidates `grid_items[first .. first + len]`.
#[derive(Clone, Copy, Debug)]
pub struct ExportGridNode {
    pub first: u32,
    pub len: u32,
}

pub const GRID_INNER: u32 = INNER;
/// [`Export::grid_progs`] entry of a candidate evaluated with its whole
/// program.
pub const GRID_PROG_FULL: u32 = NONE;
/// [`Export::grid_progs`] entry of a candidate true over its whole leaf.
pub const GRID_PROG_TRUE: u32 = NONE - 1;

/// A universe seen through a transform: `local = rot · p + disp` (identity
/// when there is no transform).
#[derive(Clone, Copy, Debug)]
pub struct ExportTarget {
    pub unit: u32,
    pub transform: Affine,
    /// How the instance of the cell holding this target maps to that of
    /// the universe it holds: `offset + host · stride + element`, where
    /// `element` is the lattice element's index in
    /// [`ExportLattice::numbered`] when `stride > 1` (a lattice filled with
    /// one universe), 0 otherwise. [`NO_INSTANCE`] where none can be numbered.
    pub instance_offset: u64,
    pub instance_stride: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct ExportLattice {
    pub hexagonal: bool,
    /// Centre of element `[0,0,0]`; element `ix` is shifted by
    /// `Σ ix[i] · basis[i]`.
    pub center: Vec3,
    pub basis: [Vec3; 3],
    /// `dual[i] · basis[j] = δ_ij`: `ix[i] = floor(dual[i] · (p - center) + ½)`
    /// on bounded axes (then the nearest hexagon by `sides`).
    pub dual: [Vec3; 3],
    pub bounded: [bool; 3],
    /// Outward planes `(n, d)` of element `[0,0,0]` (`n·x ≤ d` inside) in
    /// [`Export::lattice_planes`]: the sides, and every face.
    pub sides: Span,
    pub faces: Span,
    /// Fill array: `lo`, `size`, and elements (i fastest) in
    /// [`Export::lattice_elements`]. A uniform lattice has one element and
    /// `size` `[0, 0, 0]`.
    pub lo: [i32; 3],
    pub size: [i32; 3],
    pub elements: u32,
    /// The elements numbered as instances of the lattice cell (`lo`,
    /// `size`): the fill array, or a range around the lattice cell's box;
    /// size zero when none are.
    pub numbered: ([i32; 3], [i32; 3]),
}

/// See the module documentation.
pub struct Export {
    pub tolerance: f64,
    /// Per shape: one of the `SHAPE_*` kinds and where its data starts.
    pub shape_kind: Vec<u32>,
    pub shape_start: Vec<u32>,
    pub shape_data: Vec<f64>,
    pub code: Vec<u32>,
    /// `[may-be-true, may-be-false]` boxes of large program nodes: outside
    /// the first the node is false, outside the second true.
    pub node_boxes: Vec<[Aabb; 2]>,
    pub cells: Vec<ExportCell>,
    pub units: Vec<ExportUnit>,
    pub unit_cells: Vec<u32>,
    pub always: Vec<u32>,
    pub bvh_nodes: Vec<ExportBvhNode>,
    pub bvh_items: Vec<u32>,
    /// Item boxes, parallel to `bvh_items`.
    pub bvh_boxes: Vec<Aabb>,
    pub grid_nodes: Vec<ExportGridNode>,
    pub grid_items: Vec<u32>,
    /// Parallel to `grid_items`: where in `code` the candidate's program
    /// specialized to its leaf starts (valid only for points in that leaf;
    /// its length is in its first word, as for any program), or
    /// [`GRID_PROG_FULL`] / [`GRID_PROG_TRUE`]. These programs follow the
    /// cells' in `code`.
    pub grid_progs: Vec<u32>,
    pub targets: Vec<ExportTarget>,
    pub lattices: Vec<ExportLattice>,
    pub lattice_planes: Vec<(Vec3, f64)>,
    /// Compiled shape of each lattice plane: the face's surface, [`NONE`] for
    /// the side planes.
    pub lattice_plane_shapes: Vec<u32>,
    /// A target index, [`ELEMENT_OWN`] or [`ELEMENT_MISSING`].
    pub lattice_elements: Vec<u32>,
    /// Number of cell instance ids ([`crate::Instances::total`]).
    pub instances: u64,
    /// Largest nesting of program nodes, BVH depth and octree depth, for
    /// fixed-size stacks.
    pub max_code_depth: u32,
    pub max_bvh_depth: u32,
    pub max_grid_depth: u32,
    /// Universe levels ([`Model::levels`]): a ray's path is never deeper,
    /// for per-level state sized to the model.
    pub levels: u32,
}

impl Model {
    /// A flat copy of the compiled model (see [`crate::export`]).
    pub fn export(&self) -> Export {
        let (mut e, bulk) = self.export_lean();
        e.code.reserve_exact(bulk.code_len());
        bulk.code(|c| e.code.extend_from_slice(c));
        bulk.grid_nodes(|n| e.grid_nodes.extend_from_slice(n));
        e.grid_items.reserve_exact(bulk.grid_item_count());
        bulk.grid_items(|c| e.grid_items.extend_from_slice(c));
        e.grid_progs.reserve_exact(bulk.grid_item_count());
        bulk.grid_progs(|c| e.grid_progs.extend_from_slice(c));
        e
    }

    /// [`Model::export`] without its largest arrays, `code`, `grid_nodes`,
    /// `grid_items` and `grid_progs` (left empty; the offsets into them are
    /// as in the full export), which the [`Bulk`] hands out piece by piece
    /// from the model itself: an upload need not hold a copy of them all.
    pub fn export_lean(&self) -> (Export, Bulk<'_>) {
        let mut e = Export {
            tolerance: self.tolerance,
            shape_kind: Vec::with_capacity(self.shapes.len()),
            shape_start: Vec::with_capacity(self.shapes.len()),
            shape_data: Vec::new(),
            code: Vec::new(),
            node_boxes: self.node_boxes.clone(),
            cells: Vec::with_capacity(self.cells.len()),
            units: Vec::with_capacity(self.units.len()),
            unit_cells: Vec::new(),
            always: Vec::new(),
            bvh_nodes: Vec::new(),
            bvh_items: Vec::new(),
            bvh_boxes: Vec::new(),
            grid_nodes: Vec::new(),
            grid_items: Vec::new(),
            grid_progs: Vec::new(),
            targets: Vec::new(),
            lattices: Vec::new(),
            lattice_planes: Vec::new(),
            lattice_plane_shapes: Vec::new(),
            lattice_elements: Vec::new(),
            instances: self.instances().total(),
            max_code_depth: 0,
            max_bvh_depth: 0,
            max_grid_depth: 0,
            levels: self.levels() as u32,
        };
        for s in &self.shapes {
            export_shape(s, &mut e);
        }
        let inst = self.instances();
        let target = |e: &mut Export, t: &Target, site: Option<(u64, u64)>| {
            let (instance_offset, instance_stride) = site.unwrap_or((NO_INSTANCE, 1));
            e.targets.push(ExportTarget {
                unit: t.unit,
                transform: t.transform.unwrap_or(Affine::IDENTITY),
                instance_offset,
                instance_stride,
            });
            e.targets.len() as u32 - 1
        };
        // The cell of each lattice, for its instances.
        let mut lattice_cell = vec![NONE; self.lattices.len()];
        for (k, c) in self.cells.iter().enumerate().rev() {
            if let Some(Fill::Lattice(l)) = c.fill {
                lattice_cell[l as usize] = k as u32;
            }
        }
        for (i, lat) in self.lattices.iter().enumerate() {
            debug_assert_eq!(e.lattices.len(), i);
            let span = |e: &mut Export, planes: &[(Vec3, f64)], shapes: Option<&[u32]>| {
                let start = e.lattice_planes.len() as u32;
                e.lattice_planes.extend_from_slice(planes);
                match shapes {
                    Some(s) => e.lattice_plane_shapes.extend_from_slice(s),
                    None => e.lattice_plane_shapes.extend(planes.iter().map(|_| NONE)),
                }
                Span {
                    start,
                    len: planes.len() as u32,
                }
            };
            let sides = span(&mut e, &lat.sides, None);
            let faces = span(&mut e, &lat.faces, Some(&lat.face_surfaces));
            let cell = lattice_cell[i];
            let site = |k: Option<u64>| (cell != NONE).then(|| inst.site(cell, k)).flatten();
            let (lo, size, elements) = match &lat.fill {
                LatticeFill::Uniform(t) => {
                    let t = target(&mut e, t, site(None));
                    e.lattice_elements.push(t);
                    ([0; 3], [0; 3], e.lattice_elements.len() as u32 - 1)
                }
                LatticeFill::Array { lo, size, elements } => {
                    let ids: Vec<u32> = elements
                        .iter()
                        .enumerate()
                        .map(|(k, el)| match el {
                            Some(t) => target(&mut e, t, site(Some(k as u64))),
                            None => ELEMENT_OWN,
                        })
                        .collect();
                    let start = e.lattice_elements.len() as u32;
                    e.lattice_elements.extend(ids);
                    (*lo, *size, start)
                }
            };
            e.lattices.push(ExportLattice {
                hexagonal: lat.hexagonal,
                center: lat.center,
                basis: lat.basis,
                dual: lat.dual,
                bounded: lat.bounded,
                sides,
                faces,
                lo,
                size,
                elements,
                numbered: match (cell != NONE).then(|| inst.elements(cell)).flatten() {
                    Some(r) => (r.lo, r.size),
                    None => ([0; 3], [0; 3]),
                },
            });
        }
        for (k, c) in self.cells.iter().enumerate() {
            let k = k as u32;
            let fill = match &c.fill {
                None => NONE,
                Some(Fill::Universe(t)) => target(&mut e, t, inst.site(k, None)),
                Some(Fill::Lattice(l)) => FILL_LATTICE | l,
            };
            let mut flags = 0;
            if c.unsupported.is_some() {
                flags |= CELL_UNSUPPORTED;
            }
            if c.imp_zero {
                flags |= CELL_IMP_ZERO;
            }
            e.cells.push(ExportCell {
                id: c.id,
                unit: c.unit,
                material: c.material,
                code_start: c.code_start as u32,
                code_len: c.code_len,
                node_start: c.node_start,
                bbox: c.bbox,
                fill,
                flags,
                instance_base: if inst.count(k) > 0 {
                    inst.ids(k).start
                } else {
                    NO_INSTANCE
                },
            });
            e.max_code_depth = e.max_code_depth.max(code_depth(
                &self.code[c.code_start..c.code_start + c.code_len as usize],
            ));
        }
        // Where each unit's octree goes in the full arrays.
        let (mut grid_nodes, mut code_len) = (0u32, self.code.len());
        for u in &self.units {
            let cells = Span {
                start: e.unit_cells.len() as u32,
                len: u.outside().len() as u32,
            };
            e.unit_cells.extend_from_slice(u.outside());
            let always = Span {
                start: e.always.len() as u32,
                len: u.always.len() as u32,
            };
            e.always.extend_from_slice(&u.always);
            let bvh_root = if u.bvh.nodes.is_empty() {
                NONE
            } else {
                let base = e.bvh_nodes.len() as u32;
                let items = e.bvh_items.len() as u32;
                e.bvh_nodes
                    .extend(u.bvh.nodes.iter().map(|n| ExportBvhNode {
                        bbox: n.bbox,
                        start: n.start + if n.count > 0 { items } else { base },
                        count: n.count,
                    }));
                e.bvh_items.extend_from_slice(&u.bvh.items);
                e.bvh_boxes.extend_from_slice(&u.bvh.boxes);
                e.max_bvh_depth = e.max_bvh_depth.max(u.bvh.depth() as u32);
                base
            };
            let (grid_root, grid_box) = match &u.grid {
                None => (NONE, Aabb::EMPTY),
                Some(g) => {
                    let base = grid_nodes;
                    grid_nodes += g.nodes.len() as u32;
                    code_len += g.arena.len();
                    e.max_grid_depth = e.max_grid_depth.max(g.depth() as u32);
                    (base, g.root)
                }
            };
            e.units.push(ExportUnit {
                universe: u.universe,
                cells,
                roi: u.roi,
                bvh_root,
                grid_root,
                grid_box,
                always,
            });
        }
        let bulk = Bulk {
            m: self,
            code_len,
            grid_nodes: grid_nodes as usize,
        };
        (e, bulk)
    }

    /// `skip` in the layout of [`Model::export`]: per unit the box of its
    /// cells worth entering (empty if none), and a bit per node of
    /// [`Export::grid_nodes`] (bit `i % 32` of word `i / 32`), set where
    /// the node may hold a cell worth entering (every node of a unit whose
    /// nodes cannot be skipped).
    pub fn export_skip(&self, skip: &Skip) -> ExportSkip {
        let mut flags: Vec<bool> = Vec::new();
        for (k, u) in self.units.iter().enumerate() {
            let Some(g) = &u.grid else { continue };
            match skip.nodes(k as u32) {
                Some(f) => flags.extend_from_slice(f),
                None => flags.resize(flags.len() + g.nodes.len(), true),
            }
        }
        let node_bits = flags
            .par_chunks(32)
            .map(|c| {
                c.iter()
                    .enumerate()
                    .fold(0u32, |w, (i, &f)| w | (f as u32) << i)
            })
            .collect();
        ExportSkip {
            unit_boxes: skip.boxes.clone(),
            node_bits,
        }
    }
}

/// See [`Model::export_skip`].
#[derive(Clone, Debug)]
pub struct ExportSkip {
    pub unit_boxes: Vec<Aabb>,
    pub node_bits: Vec<u32>,
}

/// The arrays [`Model::export_lean`] leaves out, handed out in pieces (in
/// order, each piece as the full export has it) from the model itself.
pub struct Bulk<'a> {
    m: &'a Model,
    code_len: usize,
    grid_nodes: usize,
}

/// Records per piece of the arrays [`Bulk`] makes as it goes.
const BULK_PIECE: usize = 1 << 16;

impl Bulk<'_> {
    /// Length of [`Export::code`].
    pub fn code_len(&self) -> usize {
        self.code_len
    }

    /// Length of [`Export::grid_nodes`].
    pub fn grid_node_count(&self) -> usize {
        self.grid_nodes
    }

    /// Length of [`Export::grid_items`] and [`Export::grid_progs`].
    pub fn grid_item_count(&self) -> usize {
        self.grids().map(|g| g.items.len()).sum()
    }

    /// The program of a cell of the lean export ([`ExportCell::code_start`]
    /// and `code_len` in [`Export::code`]).
    pub fn cell_code(&self, c: &ExportCell) -> &[u32] {
        &self.m.code[c.code_start as usize..(c.code_start + c.code_len) as usize]
    }

    fn grids(&self) -> impl Iterator<Item = &crate::grid::Octree> {
        self.m.units.iter().filter_map(|u| u.grid.as_deref())
    }

    /// [`Export::code`]: the cells' programs, then each octree's residual
    /// programs.
    pub fn code(&self, mut f: impl FnMut(&[u32])) {
        f(&self.m.code);
        for g in self.grids() {
            f(&g.arena);
        }
    }

    /// [`Export::grid_nodes`].
    pub fn grid_nodes(&self, mut f: impl FnMut(&[ExportGridNode])) {
        let (mut base, mut items) = (0u32, 0u32);
        let mut piece = Vec::with_capacity(BULK_PIECE);
        for g in self.grids() {
            for c in g.nodes.chunks(BULK_PIECE) {
                piece.clear();
                piece.extend(c.iter().map(|n| ExportGridNode {
                    first: n.first + if n.len == INNER { base } else { items },
                    len: n.len,
                }));
                f(&piece);
            }
            base += g.nodes.len() as u32;
            items += g.items.len() as u32;
        }
    }

    /// [`Export::grid_items`].
    pub fn grid_items(&self, mut f: impl FnMut(&[u32])) {
        for g in self.grids() {
            f(&g.items);
        }
    }

    /// [`Export::grid_progs`].
    pub fn grid_progs(&self, mut f: impl FnMut(&[u32])) {
        let mut arena = self.m.code.len() as u32;
        let mut piece = Vec::with_capacity(BULK_PIECE);
        for g in self.grids() {
            for start in (0..g.items.len()).step_by(BULK_PIECE) {
                piece.clear();
                piece.extend((start..(start + BULK_PIECE).min(g.items.len())).map(|k| {
                    match g.progs.get(k).copied().unwrap_or(PROG_FULL) {
                        PROG_FULL => GRID_PROG_FULL,
                        PROG_TRUE => GRID_PROG_TRUE,
                        p => arena + p,
                    }
                }));
                f(&piece);
            }
            arena += g.arena.len() as u32;
        }
    }
}

fn export_shape(s: &Shape, e: &mut Export) {
    e.shape_start.push(e.shape_data.len() as u32);
    let (kind, data) = shape_data(s);
    e.shape_data.extend_from_slice(&data);
    e.shape_kind.push(kind);
}

/// Number of values of a shape in [`Export::shape_data`].
pub fn shape_len(s: &Shape) -> usize {
    match s {
        Shape::Plane { .. } => 4,
        Shape::Quadric(_) => 13,
        Shape::Cone(_) => 19,
        Shape::Torus(_) => 16,
    }
}

/// A shape's kind (`SHAPE_*`) and data as [`Export::shape_data`] holds
/// them.
pub fn shape_data(s: &Shape) -> (u32, Vec<f64>) {
    let mut d = Vec::with_capacity(19);
    let quadric = |d: &mut Vec<f64>, q: &crate::surface::Quadric| {
        d.extend_from_slice(&q.origin);
        d.extend_from_slice(&q.a);
        d.extend_from_slice(&q.b);
        d.push(q.c);
    };
    let kind = match s {
        Shape::Plane { n, d: off } => {
            d.extend_from_slice(n);
            d.push(*off);
            SHAPE_PLANE
        }
        Shape::Quadric(q) => {
            quadric(&mut d, q);
            SHAPE_QUADRIC
        }
        Shape::Cone(k) => {
            quadric(&mut d, &k.q);
            d.extend_from_slice(&k.apex);
            d.extend_from_slice(&k.axis);
            SHAPE_CONE
        }
        Shape::Torus(t) => {
            for row in &t.frame.rot {
                d.extend_from_slice(row);
            }
            d.extend_from_slice(&t.frame.disp);
            d.extend_from_slice(&[t.axis as f64, t.a, t.b, t.c]);
            SHAPE_TORUS
        }
    };
    (kind, d)
}

/// Largest number of nested nodes in a program.
fn code_depth(code: &[u32]) -> u32 {
    let mut ends: Vec<usize> = Vec::new();
    let mut best = 0;
    let mut pos = 0;
    while pos < code.len() {
        while ends.last().is_some_and(|&end| pos >= end) {
            ends.pop();
        }
        let w = code[pos];
        if w & NODE == 0 {
            pos += 1;
            continue;
        }
        ends.push(pos + 1 + (w & MASK) as usize);
        best = best.max(ends.len() as u32);
        pos += 2;
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Options;

    #[test]
    fn nesting_depth_of_programs() {
        // AND(1, OR(2, AND(3, 4)), 5)
        let or = NODE | crate::expr::OR;
        let code = [NODE | 9, NONE, 1, or | 6, NONE, 2, NODE | 3, NONE, 3, 4];
        assert_eq!(code_depth(&code), 3);
        assert_eq!(code_depth(&[1]), 0);
    }

    #[test]
    fn export_keeps_cells_units_and_fills() {
        let text = "t\n1 0 -1 fill=1 (1 0 0)\n2 0 1\n3 0 -2 u=1\n4 0 2 u=1\n\n\
                    1 so 10\n2 sx 1 3\n\n";
        let m = Model::from_text(text, &Options::default());
        let e = m.export();
        assert_eq!(e.cells.len(), 4);
        assert_eq!(e.units.len(), 2);
        assert_eq!(e.shape_kind, vec![SHAPE_QUADRIC, SHAPE_QUADRIC]);
        let c1 = &e.cells[m.cell_index(1).unwrap() as usize];
        let t = &e.targets[c1.fill as usize];
        assert_eq!(e.units[t.unit as usize].universe, 1);
        assert_eq!(t.transform.apply([0.0; 3]), [-1.0, 0.0, 0.0]);
        // Every unit's cells, and every BVH item, are cells of that unit.
        for (ui, u) in e.units.iter().enumerate() {
            let cells = &e.unit_cells[u.cells.start as usize..][..u.cells.len as usize];
            assert!(cells.iter().all(|&c| e.cells[c as usize].unit == ui as u32));
        }
    }
}
