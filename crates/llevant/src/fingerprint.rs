//! Fingerprints: hashes of cells and universes that do not depend on how the
//! input is numbered or written, so that two versions of a model can be
//! compared without tracing (design, section 7).
//!
//! Each cell and each universe gets two digests:
//! - `geometry`: what segments depend on. Equal geometry digests mean the
//!   cell (with everything that fills it) gives the same segments, whatever
//!   its cell, surface, transform, universe and material numbers. This is
//!   the key for cached segments.
//! - `content`: the geometry plus materials (composition, not number) and
//!   densities.
//!
//! Canonical forms, built from the compiled model (transforms, `LIKE BUT`,
//! `TRCL`, `#n` and macrobodies already resolved):
//! - A surface is its compiled shape in its universe's frame, written in a
//!   canonical way (planes and quadrics with a sign convention, which flips
//!   the sense of the leaves that use them; quadrics about a canonical
//!   centre and scaled; tori by centre and axis), with every value rounded
//!   to a relative precision.
//! - A cell program hashes the children of each AND/OR node as a sorted set,
//!   except in lattice cells, where the order of the faces numbers the
//!   elements.
//! - A filled cell includes its fill transform and the digest of the filling
//!   universe (a Merkle tree, so a digest covers everything inside).
//! - A universe is the sorted set of its cells' digests.
//!
//! Rounding can only make equal geometry hash differently (a value next to
//! a rounding boundary), which costs a re-trace; it cannot make a change
//! larger than the precision hash the same.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::bbox;
use crate::error::{Error, Result};
use crate::expr::{MASK, NODE, OR, POS};
use crate::lattice::{Lattice, LatticeFill, Target};
use crate::model::{topo_units, Fill};
use crate::surface::{Quadric, Shape};
use crate::transform::Affine;
use crate::vec3::{dot, Vec3};
use crate::Model;

/// Bumped whenever a change to llevant could change the segments of an
/// unchanged input, or the canonical forms: digests of different versions
/// never match.
pub const FINGERPRINT_VERSION: u32 = 3;

/// Default relative precision of every hashed value.
pub const DEFAULT_PRECISION: f64 = 1e-9;

/// A BLAKE3 digest.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest(pub [u8; 32]);

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Digest({self})")
    }
}

/// The two digests of a cell or universe (see the module documentation).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    pub geometry: Digest,
    pub content: Digest,
}

/// Fingerprints of every cell and every universe of a model.
#[derive(Clone, Debug)]
pub struct Fingerprints {
    pub version: u32,
    pub precision: f64,
    /// By cell index ([`Model::cells`]).
    pub cells: Vec<Fingerprint>,
    /// By unit index ([`Model::units`]).
    pub units: Vec<Fingerprint>,
}

/// How a cell or universe (by MCNP number) differs between two models.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change {
    /// Same geometry, other materials or densities.
    Content(i64),
    /// Other geometry.
    Geometry(i64),
    /// Gone from the first model, and the second has one with the same
    /// content under a number the first does not use.
    Renumbered { from: i64, to: i64 },
    /// Only in the first model.
    Removed(i64),
    /// Only in the second model.
    New(i64),
}

/// The differences of one kind of thing (cells or universes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// Numbers in both models with equal digests.
    pub unchanged: usize,
    /// Changed numbers in order, then removed or renumbered ones, then new
    /// ones.
    pub changes: Vec<Change>,
}

impl Changes {
    fn between(a: &BTreeMap<i64, Fingerprint>, b: &BTreeMap<i64, Fingerprint>) -> Changes {
        let mut out = Changes::default();
        let mut gone = Vec::new();
        for (&n, f) in a {
            match b.get(&n) {
                Some(g) if g == f => out.unchanged += 1,
                Some(g) if g.geometry == f.geometry => out.changes.push(Change::Content(n)),
                Some(_) => out.changes.push(Change::Geometry(n)),
                None => gone.push(n),
            }
        }
        let new: Vec<i64> = b.keys().copied().filter(|n| !a.contains_key(n)).collect();
        // A removed and a new one with the same content: renumbered.
        let mut by_content: BTreeMap<Digest, Vec<i64>> = BTreeMap::new();
        for &n in &new {
            by_content.entry(b[&n].content).or_default().push(n);
        }
        let mut renumbered = BTreeSet::new();
        for n in gone {
            match by_content.get_mut(&a[&n].content).and_then(|v| v.pop()) {
                Some(to) => {
                    renumbered.insert(to);
                    out.changes.push(Change::Renumbered { from: n, to });
                }
                None => out.changes.push(Change::Removed(n)),
            }
        }
        for n in new.into_iter().filter(|n| !renumbered.contains(n)) {
            out.changes.push(Change::New(n));
        }
        out
    }
}

/// What [`Model::fingerprint_diff`] finds. `Display` gives a report, one
/// line per change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FingerprintDiff {
    pub cells: Changes,
    pub universes: Changes,
}

impl FingerprintDiff {
    /// Whether the two models are the same, up to numbering.
    pub fn is_empty(&self) -> bool {
        self.cells.changes.is_empty() && self.universes.changes.is_empty()
    }
}

impl fmt::Display for FingerprintDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (what, c) in [("cell", &self.cells), ("universe", &self.universes)] {
            writeln!(
                f,
                "{what}s: {} unchanged, {} differences",
                c.unchanged,
                c.changes.len()
            )?;
            for ch in &c.changes {
                match ch {
                    Change::Content(n) => writeln!(f, "  {what} {n}: material or density changed"),
                    Change::Geometry(n) => writeln!(f, "  {what} {n}: geometry changed"),
                    Change::Renumbered { from, to } => {
                        writeln!(f, "  {what} {from}: renumbered {to}")
                    }
                    Change::Removed(n) => writeln!(f, "  {what} {n}: removed"),
                    Change::New(n) => writeln!(f, "  {what} {n}: new"),
                }?;
            }
        }
        Ok(())
    }
}

// Tags that keep the different things hashed apart.
const T_PLANE: u8 = 1;
const T_QUADRIC: u8 = 2;
const T_CONE: u8 = 3;
const T_TORUS: u8 = 4;
const T_NODE: u8 = 10;
const T_PROGRAM: u8 = 11;
const T_CELL: u8 = 12;
const T_UNIT: u8 = 13;
const T_MATERIAL: u8 = 14;
const T_TRANSFORM: u8 = 15;

/// One child of a program node: a sub-node, or a leaf (a surface and the
/// side the leaf is true on).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Entry {
    And(Digest),
    Or(Digest),
    Leaf(Digest, bool),
}

impl Entry {
    fn write(&self, h: &mut blake3::Hasher) {
        let (k, d) = match self {
            Entry::And(d) => (0u8, d),
            Entry::Or(d) => (1, d),
            Entry::Leaf(d, false) => (2, d),
            Entry::Leaf(d, true) => (3, d),
        };
        h.update(&[k]);
        h.update(&d.0);
    }
}

/// Writes rounded values into hashers.
struct Hashing {
    key: [u8; 32],
    rel: f64,
}

impl Hashing {
    fn new(precision: f64) -> Result<Self> {
        if !(precision > 0.0 && precision < 1e-2) {
            return Err(Error::Invalid(format!(
                "fingerprint precision must be in (0, 0.01), not {precision}"
            )));
        }
        let mut material = Vec::new();
        material.extend_from_slice(&FINGERPRINT_VERSION.to_le_bytes());
        material.extend_from_slice(&precision.to_le_bytes());
        Ok(Hashing {
            key: blake3::derive_key("llevant fingerprint", &material),
            rel: precision,
        })
    }

    fn hasher(&self, tag: u8) -> blake3::Hasher {
        let mut h = blake3::Hasher::new_keyed(&self.key);
        h.update(&[tag]);
        h
    }

    /// `v` rounded to `rel` of its own magnitude (of 1, if smaller): the
    /// binary exponent of the magnitude and the multiple of the step.
    /// Symmetric: `round(-v)` is `round(v)` negated.
    fn round(&self, v: f64) -> (i32, i64) {
        if !v.is_finite() {
            return (i32::MAX, if v.is_nan() { 0 } else { v.signum() as i64 });
        }
        let m = v.abs().max(1.0);
        let mut e = m.log2().floor() as i32;
        while 2f64.powi(e + 1) <= m {
            e += 1;
        }
        while 2f64.powi(e) > m {
            e -= 1;
        }
        let step = self.rel * 2f64.powi(e);
        (e, (v / step).round() as i64)
    }

    fn values(&self, h: &mut blake3::Hasher, vs: &[f64]) {
        for &v in vs {
            let (e, k) = self.round(v);
            h.update(&e.to_le_bytes());
            h.update(&k.to_le_bytes());
        }
    }

    /// Whether the first value that does not round to zero is negative.
    fn leads_negative(&self, vs: &[f64]) -> bool {
        vs.iter()
            .map(|&v| self.round(v).1)
            .find(|&k| k != 0)
            .is_some_and(|k| k < 0)
    }

    /// A surface's digest, and whether its canonical form is the negated
    /// shape (so leaves using it flip their side).
    fn shape(&self, s: &Shape) -> (Digest, bool) {
        match s {
            Shape::Plane { n, d } => {
                let flip = self.leads_negative(n);
                let g = if flip { -1.0 } else { 1.0 };
                let mut h = self.hasher(T_PLANE);
                self.values(&mut h, &[g * n[0], g * n[1], g * n[2], g * d]);
                (Digest(*h.finalize().as_bytes()), flip)
            }
            Shape::Quadric(q) => {
                let (o, a, b, c) = canonical_quadric(q, None);
                let mut vs: Vec<f64> = a.to_vec();
                vs.extend_from_slice(&b);
                vs.push(c);
                let flip = self.leads_negative(&vs);
                let g = if flip { -1.0 } else { 1.0 };
                let mut h = self.hasher(T_QUADRIC);
                self.values(&mut h, &o);
                for v in vs {
                    self.values(&mut h, &[g * v]);
                }
                (Digest(*h.finalize().as_bytes()), flip)
            }
            Shape::Cone(k) => {
                // The nappe makes the two sides asymmetric: no sign flip.
                let (o, a, b, c) = canonical_quadric(&k.q, Some(k.apex));
                let mut h = self.hasher(T_CONE);
                self.values(&mut h, &o);
                self.values(&mut h, &a);
                self.values(&mut h, &b);
                self.values(&mut h, &[c]);
                self.values(&mut h, &k.axis);
                (Digest(*h.finalize().as_bytes()), false)
            }
            Shape::Torus(t) => {
                // Symmetric about its mid-plane and about its axis: the
                // centre, the axis up to sign, and the radii.
                let center = t.frame.inverse().disp;
                let axis = t.frame.rot[t.axis as usize];
                let g = if self.leads_negative(&axis) {
                    -1.0
                } else {
                    1.0
                };
                let mut h = self.hasher(T_TORUS);
                self.values(&mut h, &center);
                self.values(&mut h, &axis.map(|x| g * x));
                self.values(&mut h, &[t.a, t.b, t.c]);
                (Digest(*h.finalize().as_bytes()), false)
            }
        }
    }

    fn transform(&self, h: &mut blake3::Hasher, t: &Option<Affine>) {
        h.update(&[T_TRANSFORM]);
        match t {
            None => {
                h.update(&[0]);
            }
            Some(t) => {
                h.update(&[1]);
                for row in &t.rot {
                    self.values(h, row);
                }
                self.values(h, &t.disp);
            }
        }
    }

    /// A material's composition: ZAIDs (case-folded) with fractions
    /// normalised the way MCNP does, sorted. The number is ignored.
    fn material(&self, entries: &[(String, f64)], well_formed: bool) -> Digest {
        let mut by_zaid: FxHashMap<String, f64> = FxHashMap::default();
        for (z, f) in entries {
            *by_zaid.entry(z.trim().to_ascii_lowercase()).or_default() += f;
        }
        let total: f64 = by_zaid.values().map(|f| f.abs()).sum();
        let mut v: Vec<(String, f64)> = by_zaid.into_iter().collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        let mut h = self.hasher(T_MATERIAL);
        h.update(&[well_formed as u8]);
        for (z, f) in v {
            h.update(&(z.len() as u64).to_le_bytes());
            h.update(z.as_bytes());
            let f = if total > 0.0 { f / total } else { f };
            self.values(&mut h, &[f]);
        }
        Digest(*h.finalize().as_bytes())
    }
}

/// A quadric written about a canonical origin (the given one, or the
/// point of its centre set nearest the global origin), scaled so that its
/// largest second-order coefficient is 1: `(origin, A, b, c)`.
fn canonical_quadric(q: &Quadric, origin: Option<Vec3>) -> (Vec3, [f64; 6], Vec3, f64) {
    let o = origin.unwrap_or_else(|| match bbox::analyse(q) {
        Some(info) => {
            // Along "soft" (near-zero eigenvalue) directions the centre is
            // arbitrary: take the component the global origin has, zero.
            let mut c = info.center;
            for k in 0..3 {
                if !info.stiff[k] {
                    let s = dot(info.u[k], c);
                    for i in 0..3 {
                        c[i] -= s * info.u[k][i];
                    }
                }
            }
            c
        }
        None => [0.0; 3],
    });
    // f about o: (y + δ)ᵀA(y + δ) + b·(y + δ) + c with δ = o - q.origin.
    let m = q.matrix();
    let delta = [0, 1, 2].map(|i| o[i] - q.origin[i]);
    let ad = [dot(m[0], delta), dot(m[1], delta), dot(m[2], delta)];
    let b = [0, 1, 2].map(|i| q.b[i] + 2.0 * ad[i]);
    let c = q.c + dot(delta, ad) + dot(q.b, delta);
    let mut scale = q.a.iter().fold(0.0f64, |s, x| s.max(x.abs()));
    if scale == 0.0 {
        scale = b.iter().fold(0.0f64, |s, x| s.max(x.abs()));
    }
    if scale == 0.0 {
        scale = 1.0;
    }
    (o, q.a.map(|x| x / scale), b.map(|x| x / scale), c / scale)
}

impl Model {
    /// Fingerprints of every cell and universe, with values rounded to
    /// `precision` (relative; [`DEFAULT_PRECISION`] unless there is a
    /// reason). Fingerprints computed with different precisions or versions
    /// are not comparable. [`Error::Invalid`] unless `precision` is in
    /// `(0, 0.01)`.
    pub fn fingerprints(&self, precision: f64) -> Result<Fingerprints> {
        let hs = Hashing::new(precision)?;
        let shapes: Vec<(Digest, bool)> = self.shapes.par_iter().map(|s| hs.shape(s)).collect();
        let materials: FxHashMap<i64, Digest> = self
            .materials
            .par_iter()
            .map(|(&id, (e, ok))| (id, hs.material(e, *ok)))
            .collect();
        let none = Digest([0; 32]);
        let mut cells = vec![
            Fingerprint {
                geometry: none,
                content: none,
            };
            self.cells.len()
        ];
        let mut units = vec![
            Fingerprint {
                geometry: none,
                content: none,
            };
            self.units.len()
        ];
        // Filled universes before the cells that fill them.
        for u in topo_units(self).into_iter().rev() {
            let unit = &self.units[u as usize];
            let fps: Vec<Fingerprint> = unit
                .cells
                .par_iter()
                .map(|&c| {
                    let local = self.cell_local(&hs, c, &shapes);
                    let fill = |content: bool| self.fill_digest(&hs, c, &units, content);
                    let geometry = {
                        let mut h = hs.hasher(T_CELL);
                        h.update(&local.0);
                        h.update(&fill(false).0);
                        Digest(*h.finalize().as_bytes())
                    };
                    let cell = &self.cells[c as usize];
                    let content = {
                        let mut h = hs.hasher(T_CELL);
                        h.update(&local.0);
                        h.update(&fill(true).0);
                        match materials.get(&cell.material) {
                            _ if cell.material == 0 => h.update(&[0]),
                            Some(m) => h.update(&[1]).update(&m.0),
                            None => h.update(&[2]),
                        };
                        if cell.material != 0 {
                            hs.values(&mut h, &[cell.density]);
                        }
                        Digest(*h.finalize().as_bytes())
                    };
                    Fingerprint { geometry, content }
                })
                .collect();
            for (&c, fp) in unit.cells.iter().zip(&fps) {
                cells[c as usize] = *fp;
            }
            let set = |pick: fn(&Fingerprint) -> Digest| {
                let mut v: Vec<Digest> = fps.iter().map(pick).collect();
                v.sort_unstable();
                let mut h = hs.hasher(T_UNIT);
                for d in v {
                    h.update(&d.0);
                }
                Digest(*h.finalize().as_bytes())
            };
            units[u as usize] = Fingerprint {
                geometry: set(|f| f.geometry),
                content: set(|f| f.content),
            };
        }
        Ok(Fingerprints {
            version: FINGERPRINT_VERSION,
            precision,
            cells,
            units,
        })
    }

    /// Cells and universes of this model and `other` compared by MCNP
    /// number and by digest (at `precision`, as [`Model::fingerprints`]).
    pub fn fingerprint_diff(&self, other: &Model, precision: f64) -> Result<FingerprintDiff> {
        let (fa, fb) = (
            self.fingerprints(precision)?,
            other.fingerprints(precision)?,
        );
        let cells = |m: &Model, f: &Fingerprints| {
            m.cells
                .iter()
                .map(|c| c.id)
                .zip(f.cells.iter().copied())
                .collect()
        };
        let units = |m: &Model, f: &Fingerprints| {
            m.units
                .iter()
                .map(|u| u.universe)
                .zip(f.units.iter().copied())
                .collect()
        };
        Ok(FingerprintDiff {
            cells: Changes::between(&cells(self, &fa), &cells(other, &fb)),
            universes: Changes::between(&units(self, &fa), &units(other, &fb)),
        })
    }

    /// What a cell is by itself: its program, `IMP:N=0`, and why it is
    /// unsupported, if it is.
    fn cell_local(&self, hs: &Hashing, c: u32, shapes: &[(Digest, bool)]) -> Digest {
        let cell = &self.cells[c as usize];
        let ordered = self.is_lattice(c);
        let mut h = hs.hasher(T_PROGRAM);
        let code = self.code(c);
        if !code.is_empty() {
            let mut pos = 0;
            program(hs, code, &mut pos, shapes, ordered).write(&mut h);
        }
        h.update(&[cell.imp_zero as u8]);
        match &cell.unsupported {
            None => {
                h.update(&[0]);
            }
            Some(why) => {
                // Without the "cell N: " prefix, which names the cell.
                let why = why.split_once(": ").map_or(&**why, |(_, w)| w);
                h.update(&[1]);
                h.update(why.as_bytes());
            }
        }
        Digest(*h.finalize().as_bytes())
    }

    /// The fill of cell `c`: transforms and the digests of the filling
    /// universes (`units` must already hold them).
    fn fill_digest(&self, hs: &Hashing, c: u32, units: &[Fingerprint], content: bool) -> Digest {
        let pick = |t: &Target| {
            let f = &units[t.unit as usize];
            if content {
                f.content
            } else {
                f.geometry
            }
        };
        let target = |h: &mut blake3::Hasher, t: &Target| {
            hs.transform(h, &t.transform);
            h.update(&pick(t).0);
        };
        let mut h = hs.hasher(T_CELL);
        match &self.cells[c as usize].fill {
            None => {
                h.update(&[0]);
            }
            Some(Fill::Universe(t)) => {
                h.update(&[1]);
                target(&mut h, t);
            }
            Some(Fill::Lattice(l)) => {
                let lat: &Lattice = &self.lattices[*l as usize];
                h.update(&[2, lat.hexagonal as u8]);
                match &lat.fill {
                    LatticeFill::Uniform(t) => {
                        h.update(&[0]);
                        target(&mut h, t);
                    }
                    LatticeFill::Array { lo, size, elements } => {
                        h.update(&[1]);
                        for k in 0..3 {
                            h.update(&lo[k].to_le_bytes());
                            h.update(&size[k].to_le_bytes());
                        }
                        for e in elements {
                            match e {
                                None => {
                                    h.update(&[0]);
                                }
                                Some(t) => {
                                    h.update(&[1]);
                                    target(&mut h, t);
                                }
                            }
                        }
                    }
                }
            }
        }
        Digest(*h.finalize().as_bytes())
    }
}

/// The program entry starting at `code[*pos]`. AND and OR children are
/// hashed as sorted sets unless `ordered`.
fn program(
    hs: &Hashing,
    code: &[u32],
    pos: &mut usize,
    shapes: &[(Digest, bool)],
    ordered: bool,
) -> Entry {
    let w = code[*pos];
    *pos += 1;
    if w & NODE == 0 {
        let (d, flip) = shapes[(w & MASK) as usize];
        return Entry::Leaf(d, (w & POS != 0) != flip);
    }
    let end = *pos + (w & MASK) as usize;
    *pos += 1; // box word
    let mut kids = Vec::new();
    while *pos < end {
        kids.push(program(hs, code, pos, shapes, ordered));
    }
    if !ordered {
        kids.sort_unstable();
    }
    let mut h = hs.hasher(T_NODE);
    for k in &kids {
        k.write(&mut h);
    }
    let d = Digest(*h.finalize().as_bytes());
    if w & OR != 0 {
        Entry::Or(d)
    } else {
        Entry::And(d)
    }
}
