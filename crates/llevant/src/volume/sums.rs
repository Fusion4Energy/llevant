//! Lengths per label, added up per chunk of lines and merged in order (the
//! same totals for any number of threads).

use super::*;

/// Lengths of one chunk of lines (all of one probe), by local label id
/// (labels in order of first occurrence, as encoded keys of `keys`).
pub(super) struct ChunkSums {
    pub(super) probe: usize,
    pub(super) keys: crate::intern::Chunk,
    /// Length (in `FIXED` units), segments and box per label.
    pub(super) sums: Vec<(u128, u64, Aabb)>,
    /// Local ids grouped by shard; shard `s` ends at `shard_ends[s]`.
    pub(super) by_shard: Vec<u32>,
    pub(super) shard_ends: Vec<u32>,
}

/// Lengths are added up in units of 2⁻⁶⁴ cm, as integers: exact and in any
/// order, so chunks fold in as they finish and the sums still do not depend
/// on the thread count. A segment's length is already rounded far coarser by
/// f64 (`t1 - t0`), so this adds nothing: 2⁻³² cm did, on thin cells (bricks.i
/// cell 32001, 0.064 cm³: 3e-5 off, its sigma up a third). A u128 holds 1.8e19
/// cm per label and probe.
pub(super) const FIXED: f64 = 18_446_744_073_709_551_616.0;

impl ChunkSums {
    pub(super) fn shard(&self, s: usize) -> &[u32] {
        if self.shard_ends.is_empty() {
            return &[];
        }
        let start = if s == 0 { 0 } else { self.shard_ends[s - 1] };
        &self.by_shard[start as usize..self.shard_ends[s] as usize]
    }
}

/// Labels are summed over chunks in shards by hash, each behind its lock.
pub(super) const SHARDS: usize = 64;

pub(super) fn shard_of(hash: u64) -> usize {
    (hash >> 58) as usize
}

/// The sums of the labels of one shard so far.
#[derive(Default)]
pub(super) struct Shard {
    pub(super) index: FxHashMap<Box<[u32]>, u32>,
    pub(super) keys: Vec<Box<[u32]>>,
    /// Where each label was first met: the least chunk (high half) and
    /// local id there, whatever order chunks come in.
    pub(super) first: Vec<u64>,
    /// Per label, its length on each probe (`FIXED` units).
    pub(super) lengths: Vec<u128>,
    pub(super) segments: Vec<u64>,
    pub(super) bboxes: Vec<Aabb>,
}

impl Shard {
    /// Fold in the labels of shard `s` of chunk number `chunk`.
    pub(super) fn add(&mut self, c: &ChunkSums, s: usize, chunk: u32, probes: usize) {
        for &i in c.shard(s) {
            let key = c.keys.key(i as usize);
            let at = (chunk as u64) << 32 | i as u64;
            let j = match self.index.get(key) {
                Some(&j) => {
                    let j = j as usize;
                    self.first[j] = self.first[j].min(at);
                    j
                }
                None => {
                    let j = self.keys.len();
                    self.index.insert(key.into(), j as u32);
                    self.keys.push(key.into());
                    self.first.push(at);
                    self.lengths.resize(self.lengths.len() + probes, 0);
                    self.segments.push(0);
                    self.bboxes.push(Aabb::EMPTY);
                    j
                }
            };
            let (len, n, bbox) = &c.sums[i as usize];
            self.lengths[j * probes + c.probe] += len;
            self.segments[j] += n;
            self.bboxes[j] = self.bboxes[j].union(bbox);
        }
    }

    /// The volume of label `j`, its estimates not yet worked out.
    pub(super) fn volume(&self, j: usize, probes: usize) -> Volume {
        Volume {
            lengths: self.lengths[j * probes..(j + 1) * probes]
                .iter()
                .map(|&l| l as f64 / FIXED)
                .collect(),
            segments: self.segments[j],
            bbox: self.bboxes[j],
            ..Volume::empty(0)
        }
    }
}
