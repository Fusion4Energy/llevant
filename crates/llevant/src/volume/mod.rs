//! Volumes from random lattices of parallel lines (docs/internals/engine.md, "Volumes").
//!
//! A grid is a number of probes: lattices of parallel lines, each with its
//! own random direction, rotation about it and offset, all drawn from a
//! seed. A label's volume from one probe is the total length of its
//! segments times the area per line: exact along the lines, sampled across
//! them, and unbiased over the random shift. The estimate is the mean over
//! the probes and `sigma` the standard error of that mean: the probes are
//! independent, so it is an honest error bar. Random directions keep flat
//! faces off the lines; lines parallel to a face make errors of order `h`
//! that do not average out (measured 2026-10-23).
//!
//! Lines are identified by integers (probe, row, column), so two models
//! traced with the same grid share their lines. Lengths are added up per
//! chunk of lines and the segments dropped, so whole-model grids need no
//! memory for segments. Chunks are merged in order: results are identical
//! for any number of threads.

use std::sync::Mutex;

use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::bbox::{Aabb, Bounder};
use crate::bvh::Bvh;
use crate::classify::{Label, Status};
use crate::error::{Error, Result};
use crate::model::Fill;
use crate::progress::Progress;
use crate::trace::Line;
use crate::transform::Affine;
use crate::vec3::{add, dot, scale, Vec3};
use crate::Model;

// Lines traced and their total length, for the debug log of each round.
static PIECES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LENGTH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
// Thread time making the pieces of lines over boxes and tracing them, ns.
static PIECE_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TRACE_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Chunks aimed at in [`Model::volumes_in`] (within [`BAND_MIN`] and
/// [`BAND_MAX`] lines over boxes each): each chunk is traced on one thread.
const CHUNKS: usize = 1024;

mod grid;
mod instances;
mod placed;
mod refine;
mod sums;
#[cfg(test)]
mod tests;

pub use grid::*;
pub use placed::*;
pub use refine::*;
use sums::*;

/// Volume of one label (or a group of labels), in cm³.
#[derive(Clone, Debug, PartialEq)]
pub struct Volume {
    /// Mean of the probes' estimates.
    pub estimate: f64,
    /// Standard error of `estimate`: the spread of the probes' estimates
    /// over the square root of their number. Unreliable when only a few
    /// lines met the label ([`Volume::segments`], [`Volume::probes_hit`]).
    pub sigma: f64,
    /// Estimate of each probe (0 for probes that missed it).
    pub by_probe: Vec<f64>,
    /// Segments found, over all probes.
    pub segments: u64,
    /// Box around the segments found (empty if none).
    pub bbox: Aabb,
    /// Total length per probe.
    lengths: Vec<f64>,
}

impl Volume {
    fn empty(probes: usize) -> Volume {
        Volume {
            estimate: 0.0,
            sigma: 0.0,
            by_probe: Vec::new(),
            segments: 0,
            bbox: Aabb::EMPTY,
            lengths: vec![0.0; probes],
        }
    }

    /// Probes with some line through it.
    pub fn probes_hit(&self) -> usize {
        self.lengths.iter().filter(|&&l| l > 0.0).count()
    }

    fn add(&mut self, o: &Volume) {
        for (a, b) in self.lengths.iter_mut().zip(&o.lengths) {
            *a += b;
        }
        self.segments += o.segments;
        self.bbox = self.bbox.union(&o.bbox);
    }

    /// The probes of `o` after its own (further probes of the same grid).
    fn append(&mut self, o: &Volume) {
        self.lengths.extend_from_slice(&o.lengths);
        self.segments += o.segments;
        self.bbox = self.bbox.union(&o.bbox);
    }

    /// Fill in the estimates from the lengths.
    fn finish(&mut self, grid: &VolumeGrid) {
        let s = grid.spacing();
        self.by_probe = self.lengths.iter().map(|l| l * s * s).collect();
        let n = self.by_probe.len() as f64;
        let mean = self.by_probe.iter().sum::<f64>() / n;
        let ss: f64 = self.by_probe.iter().map(|v| (v - mean) * (v - mean)).sum();
        self.estimate = mean;
        self.sigma = (ss / (n - 1.0) / n).sqrt();
    }
}

/// Volumes per label, labels numbered by first occurrence along the lines
/// (probes in order).
#[derive(Clone, Debug)]
pub struct Volumes {
    pub grid: VolumeGrid,
    pub labels: Vec<Label>,
    pub volumes: Vec<Volume>,
}

impl Volumes {
    /// Add up the volumes of the labels that share a key (cell number,
    /// material, universe...), keys in order of first appearance. Labels
    /// for which `key` gives `None` are left out. Lengths are added per
    /// probe before the spread is taken: labels of one group share lines,
    /// so their errors are not independent.
    pub fn group_by<K: std::hash::Hash + Eq + Clone>(
        &self,
        mut key: impl FnMut(&Label) -> Option<K>,
    ) -> Vec<(K, Volume)> {
        let mut index: FxHashMap<K, usize> = FxHashMap::default();
        let mut out: Vec<(K, Volume)> = Vec::new();
        for (l, v) in self.labels.iter().zip(&self.volumes) {
            let Some(k) = key(l) else { continue };
            // As many probes as traced (`volumes_in` traces a range of them).
            let i = *index.entry(k.clone()).or_insert_with(|| {
                out.push((k, Volume::empty(v.lengths.len())));
                out.len() - 1
            });
            out[i].1.add(v);
        }
        for (_, v) in &mut out {
            v.finish(&self.grid);
        }
        out
    }

    /// Volumes by cell, material or universe number: labels inside a cell
    /// add up by the number, anything else (gaps, overlaps...) stays one
    /// group per label. Groups with a number come first, in number order,
    /// then the others by name ([`GroupBy::Label`]: inside labels first).
    pub fn grouped(&self, m: &Model, by: GroupBy) -> Vec<Group> {
        // Sort key: (not inside, number, name).
        let key = |l: &Label| -> (bool, Option<i64>, String) {
            let inside = l.status == Status::Inside;
            let number = match (l.cell(), by) {
                (Some(c), GroupBy::Cell) => Some(m.cell(c).id),
                (Some(c), GroupBy::Material) => Some(m.cell(c).material),
                (Some(c), GroupBy::Universe) => Some(m.units()[m.cell(c).unit as usize].universe),
                _ => None,
            };
            let name = match number {
                Some(n) => n.to_string(),
                None => m.describe(l),
            };
            (!inside, number, name)
        };
        // A group without a number is one label: keep it.
        let mut groups: Vec<(_, Volume)> = self.group_by(|l| {
            let k = key(l);
            let label = k.1.is_none().then(|| l.clone());
            Some((k, l.status, label))
        });
        groups.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
        groups
            .into_iter()
            .map(|(((_, number, name), status, label), volume)| Group {
                number,
                name,
                status,
                label,
                volume,
            })
            .collect()
    }

    /// Label ids of gaps, overlaps, unsupported cells and too-deep nesting,
    /// largest volume first. On-surface stretches are left out: they are
    /// thinner than the tolerance, or run along a surface.
    pub fn problems(&self) -> Vec<usize> {
        let mut p: Vec<usize> = (0..self.labels.len())
            .filter(|&i| !matches!(self.labels[i].status, Status::Inside | Status::OnSurface))
            .collect();
        p.sort_by(|&a, &b| {
            self.volumes[b]
                .estimate
                .total_cmp(&self.volumes[a].estimate)
                .then(a.cmp(&b))
        });
        p
    }

    /// Sum of all volumes (the region, up to the sampling error).
    pub fn total(&self) -> f64 {
        self.volumes.iter().map(|v| v.estimate).sum()
    }
}

/// What [`Volumes::grouped`] adds up by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GroupBy {
    Cell,
    Material,
    Universe,
    /// Every label apart: the full path through the universes, so every
    /// instance of a cell on its own.
    Label,
}

/// Volume of a group of labels ([`Volumes::grouped`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// The cell, material or universe number; `None` for a single label.
    pub number: Option<i64>,
    /// The number, or the label in MCNP terms ([`Model::describe`]).
    pub name: String,
    /// [`Status::Inside`] for groups with a number.
    pub status: Status,
    /// The label, for a group that is one label (no number: gaps,
    /// overlaps..., and every group by [`GroupBy::Label`]).
    pub label: Option<Label>,
    pub volume: Volume,
}

/// What [`Model::volumes_to`] aims for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeTarget {
    /// Relative standard error to reach, `sigma / estimate` (e.g. 1e-3).
    pub rel_sigma: f64,
    /// Smallest `h` to refine to, cm.
    pub h_min: f64,
    /// Standard error, cm³, below which a group is done whatever its
    /// relative one (0: none). Small cells, where `rel_sigma` would need
    /// lines a fraction of their size apart, stop once known this well.
    pub abs_sigma: f64,
    /// Probes a group may have at one spacing: groups a little above the
    /// target get more probes there, when cheaper than a finer spacing. At
    /// most the grid's probes (the usual choice for a whole model): none are
    /// added, groups go to finer spacings or stop. [`MAX_PROBES`] converges
    /// more groups for more time.
    pub max_probes: usize,
    /// Groups the target applies to.
    pub by: GroupBy,
}

/// Groups refined towards a [`VolumeTarget`] ([`Model::volumes_to`]).
#[derive(Clone, Debug)]
pub struct Converged {
    /// As [`Volumes::grouped`] on the first grid, refined groups replaced,
    /// with every group the model can hold that no line met added (volume
    /// 0) in its place in the order.
    pub groups: Vec<Group>,
    /// The `h` of the grid that gave each group its volume.
    pub h: Vec<f64>,
    /// Groups wanted that did not reach the target by `h_min`, in order:
    /// those found but still too uncertain, and those not found at all
    /// ([`Converged::unseen`]).
    pub unconverged: Vec<usize>,
    /// Groups wanted that no line met down to `h_min`, though their boxes
    /// say they may hold something (also in `unconverged`). Their volume is
    /// small, not known to be 0.
    pub unseen: Vec<usize>,
    /// Groups that hold nothing in the region: the conservative boxes of
    /// their cells are empty there (or nothing fills their universe), so
    /// their volume is exactly 0.
    pub empty: Vec<usize>,
    /// Whether `groups` lists every group the model can hold. Only false
    /// for [`GroupBy::Label`] with more than [`MAX_LISTED_INSTANCES`]
    /// instances, where the groups are those some line met.
    pub listed_all: bool,
}

/// Instances up to which [`Model::volumes_to`] lists every one by
/// [`GroupBy::Label`].
pub const MAX_LISTED_INSTANCES: u64 = 1 << 20;

/// What [`Model::instance_volumes`] aims for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BinTarget {
    /// Where lines may go (the model's extent, [`Model::extent`]): finite.
    pub region: Aabb,
    /// Relative standard error to reach, `sigma / estimate` (e.g. 1e-3).
    pub rel_sigma: f64,
    /// Smallest `h` to refine to, cm; the `h` of every round is this times a
    /// power of 2.
    pub h_min: f64,
    /// Largest `h` to start at, cm (infinite: none).
    pub h_max: f64,
    /// Standard error, cm³, below which a set is done whatever its relative
    /// one (0: none), as [`VolumeTarget::abs_sigma`].
    pub abs_sigma: f64,
    /// Probes a set may have at one spacing, as
    /// [`VolumeTarget::max_probes`].
    pub max_probes: usize,
    /// Draws the lines of every round.
    pub seed: u64,
    /// Probes of each round's grid, at least 2.
    pub probes: usize,
}

impl BinTarget {
    /// A target with no largest `h`, no absolute floor, seed 1,
    /// [`DEFAULT_PROBES`] probes and up to [`MAX_PROBES`] at a spacing.
    pub fn new(region: Aabb, rel_sigma: f64, h_min: f64) -> BinTarget {
        BinTarget {
            region,
            rel_sigma,
            h_min,
            h_max: f64::INFINITY,
            abs_sigma: 0.0,
            max_probes: MAX_PROBES,
            seed: 1,
            probes: DEFAULT_PROBES,
        }
    }
}

/// Volumes of sets of instances ([`Model::instance_volumes`]), one per set.
#[derive(Clone, Debug)]
pub struct BinVolumes {
    /// Each set's volume: its instances' lengths added per probe, so σ is
    /// that of the whole set.
    pub volumes: Vec<Volume>,
    /// The `h` of the grid that gave each its volume (0 if never traced).
    pub h: Vec<f64>,
    /// Instances in each set that have a path ([`Model::instance_path`]).
    /// None: a chain the model does not have, or instances below a lattice
    /// that cannot be numbered.
    pub instances: Vec<u64>,
    /// Sets with instances that did not reach the target by `h_min`, in
    /// order: those found but still too uncertain, and those not found
    /// ([`BinVolumes::unseen`]).
    pub unconverged: Vec<usize>,
    /// Sets no line met, though their boxes say they may hold something
    /// (also in `unconverged`): their volume is small, not known to be 0.
    pub unseen: Vec<usize>,
    /// Sets with instances whose boxes are all empty in the region: volume
    /// exactly 0.
    pub empty: Vec<usize>,
}

impl Model {
    /// Volume of every label met by the grid's lines.
    /// [`Error::Invalid`] unless the spacing is positive, the region finite
    /// and not empty, and there are at least 2 probes. With `progress`,
    /// chunks of lines are counted in it, and it stops with
    /// [`Error::Cancelled`] soon after it is cancelled.
    pub fn volumes(&self, grid: &VolumeGrid, progress: Option<&Progress>) -> Result<Volumes> {
        let none = Progress::new();
        self.grid_volumes(grid, progress.unwrap_or(&none))
    }

    /// [`Model::volumes`] on the lines of `grid` within the union of
    /// `boxes` alone (each clipped to the grid's region; every stretch of
    /// the union traced once): the same lines as over the whole region, so
    /// what they find inside the boxes is what the whole check finds there.
    /// With [`Model::instance_boxes`] of the cells an edit changed (in the
    /// old model and in the new one), it checks what the edit can have
    /// opened or closed.
    pub fn volumes_within(
        &self,
        grid: &VolumeGrid,
        boxes: &[Aabb],
        progress: Option<&Progress>,
    ) -> Result<Volumes> {
        let none = Progress::new();
        self.check_grid(grid)?;
        if boxes
            .iter()
            .any(|b| b.lo.iter().chain(&b.hi).any(|v| v.is_nan()))
        {
            return Err(Error::Invalid("a box to check has NaN bounds".into()));
        }
        let boxes: Vec<Aabb> = boxes
            .iter()
            .map(|b| b.intersect(&grid.region))
            .filter(|b| !b.is_empty())
            .collect();
        self.volumes_in(grid, 0..grid.probes, &boxes, progress.unwrap_or(&none))
    }

    fn check_grid(&self, grid: &VolumeGrid) -> Result<()> {
        if !(grid.h > 0.0 && grid.h.is_finite()) {
            return Err(Error::Invalid(format!(
                "grid spacing must be positive, not {}",
                grid.h
            )));
        }
        if !grid.region.is_finite() || grid.region.is_empty() {
            return Err(Error::Invalid(format!(
                "grid region must be finite and not empty: {:?} .. {:?}",
                grid.region.lo, grid.region.hi
            )));
        }
        if grid.probes < 2 {
            return Err(Error::Invalid(format!(
                "a grid needs at least 2 probes, not {}",
                grid.probes
            )));
        }
        Ok(())
    }

    fn grid_volumes(&self, grid: &VolumeGrid, progress: &Progress) -> Result<Volumes> {
        self.check_grid(grid)?;
        self.volumes_in(grid, 0..grid.probes, &[grid.region], progress)
    }

    /// Volumes of what probes `range` of the grid's lines meet inside the
    /// union of `boxes` (finite, within the grid's region): along each line,
    /// every stretch of the union is traced once ([`union_pieces`]).
    /// Probes beyond `grid.probes` are further lattices of the same seed
    /// and spacing, so the volumes of `0..a` and `a..b` together are those
    /// of `0..b`. Each [`Volume`] has one length per probe of `range`.
    fn volumes_in(
        &self,
        grid: &VolumeGrid,
        range: std::ops::Range<usize>,
        boxes: &[Aabb],
        progress: &Progress,
    ) -> Result<Volumes> {
        let n_probes = range.len();
        // Each probe's lattice; over a box only its ranges change, so they
        // are worked out again where needed instead of kept per box (12
        // probes × millions of boxes in the late rounds of `volumes_to`).
        let probes: Vec<Lattice> = range.map(|p| grid.lattice(p)).collect();
        // Rows of lines of each probe over each box, and the lines tried.
        let rows: Vec<Vec<(i64, i64, usize)>> = probes
            .par_iter()
            .map(|l| {
                boxes
                    .iter()
                    .map(|b| {
                        let o = grid.lattice_over(l, b);
                        (o.ka.0, o.ka.1, o.count())
                    })
                    .collect()
            })
            .collect();
        let total: usize = rows.iter().flatten().map(|r| r.2).sum();
        // Chunks: bands of rows of one probe with about `per_band` lines
        // over boxes, from the grid and the boxes alone (not the thread
        // count, so that sums do not depend on it). A chunk makes its
        // pieces as it is traced: memory follows the chunks in flight.
        let per_band = (total / CHUNKS).clamp(BAND_MIN, BAND_MAX);
        let mut chunks: Vec<Band> = Vec::new();
        for (p, rows) in rows.iter().enumerate() {
            let live = rows.iter().filter(|r| r.2 > 0);
            let (Some(lo), Some(hi)) = (live.clone().map(|r| r.0).min(), live.map(|r| r.1).max())
            else {
                continue;
            };
            let tried: usize = rows.iter().map(|r| r.2).sum();
            let span = (hi - lo) as usize;
            let width = (span * per_band).div_ceil(tried.max(1)).clamp(1, span) as i64;
            let mut pairs: Vec<(i64, u32)> = Vec::new();
            for (b, &(a0, a1, n)) in rows.iter().enumerate() {
                if n > 0 {
                    for band in (a0 - lo) / width..=(a1 - 1 - lo) / width {
                        pairs.push((band, b as u32));
                    }
                }
            }
            pairs.sort_unstable();
            for run in pairs.chunk_by(|x, y| x.0 == y.0) {
                let r0 = lo + run[0].0 * width;
                chunks.push(Band {
                    probe: p,
                    rows: (r0, r0 + width),
                    boxes: run.iter().map(|x| x.1).collect(),
                });
            }
        }
        progress.start(chunks.len());
        // Each chunk is traced on one thread and folded into the shards as
        // soon as it is done (integer sums: any order gives the same), so
        // memory stays with the chunks in flight and the distinct labels,
        // not with the lines.
        let acc: Vec<Mutex<Shard>> = (0..SHARDS).map(|_| Mutex::default()).collect();
        let lock = |s: usize| acc[s].lock().unwrap_or_else(|e| e.into_inner());
        chunks.par_iter().enumerate().for_each(|(c, band)| {
            if progress.cancelled() {
                return;
            }
            let sums = self.chunk_sums(grid, boxes, &probes[band.probe], band, progress);
            // Starting at a shard of its own, so that threads seldom queue.
            for k in 0..SHARDS {
                let s = (c + k) % SHARDS;
                if !sums.shard(s).is_empty() {
                    lock(s).add(&sums, s, c as u32, n_probes);
                }
            }
        });
        if progress.cancelled() {
            return Err(Error::Cancelled);
        }
        let acc: Vec<Shard> = acc
            .into_iter()
            .map(|s| s.into_inner().unwrap_or_else(|e| e.into_inner()))
            .collect();
        // Labels in order of first occurrence: by chunk, then local id.
        let mut order: Vec<(u64, usize, usize)> = acc
            .iter()
            .enumerate()
            .flat_map(|(s, shard)| shard.first.iter().enumerate().map(move |(i, &f)| (f, s, i)))
            .collect();
        order.par_sort_unstable();
        let labels = order
            .par_iter()
            .map(|&(_, s, i)| crate::classify::decode(&acc[s].keys[i]))
            .collect();
        let volumes: Vec<Volume> = order
            .par_iter()
            .map(|&(_, s, i)| {
                let mut v = acc[s].volume(i, n_probes);
                v.finish(grid);
                v
            })
            .collect();
        Ok(Volumes {
            grid: grid.clone(),
            labels,
            volumes,
        })
    }

    /// Trace one band of rows of lines (all of one probe) over the union of
    /// its boxes on this thread and sum them by label.
    fn chunk_sums(
        &self,
        grid: &VolumeGrid,
        boxes: &[Aabb],
        probe: &Lattice,
        band: &Band,
        progress: &Progress,
    ) -> ChunkSums {
        let mut out = ChunkSums {
            probe: band.probe,
            keys: Default::default(),
            sums: Vec::new(),
            by_shard: Vec::new(),
            shard_ends: Vec::new(),
        };
        if progress.cancelled() {
            return out;
        }
        let debug = log::log_enabled!(log::Level::Debug);
        let t = debug.then(std::time::Instant::now);
        let mut lines: Vec<Line> = Vec::new();
        union_pieces(grid, probe, boxes, &band.boxes, band.rows, &mut lines);
        if let Some(t) = t {
            PIECES.fetch_add(lines.len() as u64, std::sync::atomic::Ordering::Relaxed);
            let len: f64 = lines.iter().map(|l| l.t_max - l.t_min).sum();
            LENGTH.fetch_add(len as u64, std::sync::atomic::Ordering::Relaxed);
            let ns = t.elapsed().as_nanos() as u64;
            PIECE_NS.fetch_add(ns, std::sync::atomic::Ordering::Relaxed);
        }
        let t = debug.then(std::time::Instant::now);
        let part = self.trace_part(&lines);
        if let Some(t) = t {
            let ns = t.elapsed().as_nanos() as u64;
            TRACE_NS.fetch_add(ns, std::sync::atomic::Ordering::Relaxed);
        }
        let keys = &part.chunk;
        out.sums = vec![(0, 0, Aabb::EMPTY); keys.len()];
        let mut k = 0;
        for (line, &n) in lines.iter().zip(&part.counts) {
            for _ in 0..n {
                let (len, n, bbox) = &mut out.sums[keys.ids[k] as usize];
                *len += ((part.t1[k] - part.t0[k]) * FIXED).round() as u128;
                *n += 1;
                bbox.grow_point(line.at(part.t0[k]));
                bbox.grow_point(line.at(part.t1[k]));
                k += 1;
            }
        }
        // Local ids by shard, in increasing order within each.
        let shard = |i: u32| shard_of(keys.hash_of(i as usize));
        out.by_shard = (0..keys.len() as u32).collect();
        out.by_shard.sort_by_key(|&i| (shard(i), i));
        let mut at = 0;
        out.shard_ends = (0..SHARDS)
            .map(|s| {
                at += out.by_shard[at..].partition_point(|&i| shard(i) == s);
                at as u32
            })
            .collect();
        // Only the keys are folded in: the segments and the tables go now.
        out.keys = part.chunk;
        out.keys.keys_only();
        progress.step();
        out
    }
}
