//! Refining volumes to a target: rounds of finer lines over the boxes of the
//! groups that have not reached it (`Model::volumes_to`).

use super::*;

/// Whether `v`, from `probes` probes, is known well enough: within
/// `rel_sigma` of itself with every probe through it, or (`abs_sigma` > 0)
/// within `abs_sigma` cm³ with at least two probes through it (a spread
/// from fewer says nothing).
pub(super) fn reached(v: &Volume, rel_sigma: f64, abs_sigma: f64, probes: usize) -> bool {
    let hit = v.probes_hit();
    (v.sigma <= rel_sigma * v.estimate && hit == probes)
        || (abs_sigma > 0.0 && v.sigma <= abs_sigma && hit >= 2)
}

/// Probes at one spacing that [`BinTarget::new`] allows (`max_probes`):
/// more probes cost less than a finer spacing for groups a little above
/// their target, and beyond this a finer spacing is cheaper anyway.
pub const MAX_PROBES: usize = 48;
/// Probes are added in blocks of this many, so that groups wanting more at
/// the same spacing share their lines.
pub(super) const PROBE_BLOCK: usize = 6;
/// How a group's spread per probe falls with the spacing, σ ∝ h^α, until
/// two of its levels have measured it (1.5–3 by shape: the low end asks
/// for finer lines than needed rather than too coarse ones).
pub(super) const ALPHA: f64 = 1.5;
/// The largest α measured; what giving up assumes.
pub(super) const ALPHA_MAX: f64 = 3.0;

/// What a group being refined does next ([`Levels::next_step`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Step {
    /// At its target.
    Done,
    /// Short of it, and no level down to the finest would get it there even
    /// at the best rate measured (or nothing met it although lines this
    /// close crossed all its boxes).
    Stop,
    /// Trace probes `probes.0..probes.1` at `level`: from 0, a fresh start
    /// there; else added to the probes it has at that level.
    Trace { level: u32, probes: (usize, usize) },
}

/// A group being refined: its volume so far, from the probes of one level.
#[derive(Clone, Debug)]
pub(super) struct Track {
    /// Level of `volume`; `None` before its first trace.
    pub(super) level: Option<u32>,
    pub(super) volume: Volume,
    /// Level and spread per probe (σ √n) of its last other level, for α.
    pub(super) before: Option<(u32, f64)>,
    pub(super) next: Step,
}

/// The boxes to trace over for groups `at` of a refinement, for a grid.
pub(super) type BoxesOf<'a> = dyn FnMut(&[Track], &[usize], &VolumeGrid) -> Vec<Aabb> + 'a;
/// The volume of each group `at` from what was traced (`n` probes).
pub(super) type Assign<'a> = dyn FnMut(&Volumes, &[usize], usize) -> Vec<Volume> + 'a;

/// The levels of a refinement ([`Model::refine`]): the spacing is halved
/// from one to the next.
pub(super) struct Levels<'a> {
    pub(super) grid: &'a dyn Fn(u32) -> VolumeGrid,
    /// The finest level.
    pub(super) last: u32,
    pub(super) rel_sigma: f64,
    pub(super) abs_sigma: f64,
    /// Probes a level starts with.
    pub(super) base: usize,
    /// Probes a group may have at one level (at most `base`: none added).
    pub(super) max_probes: usize,
}

impl Levels<'_> {
    /// What a group does after measuring `v` at `level` (`reach`: the
    /// longest side of its boxes). The cheaper of more probes at this level
    /// (the variance falls as 1/n) and the coarsest finer level predicted
    /// to reach the target (the spread per probe falls as 2^(-jα)), costs
    /// in probes at this spacing (a level finer has 4× the lines).
    pub(super) fn next_step(
        &self,
        v: &Volume,
        level: u32,
        before: Option<(u32, f64)>,
        reach: f64,
    ) -> Step {
        let n = v.lengths.len();
        if reached(v, self.rel_sigma, self.abs_sigma, n) {
            return Step::Done;
        }
        let finer = |j: u32| match level.checked_add(j) {
            Some(l) if l <= self.last => Step::Trace {
                level: l,
                probes: (0, self.base),
            },
            _ => Step::Stop,
        };
        if v.segments == 0 {
            // Nothing met: finer lines while they are coarse next to its
            // boxes, then it is left as not found.
            return if (self.grid)(level).spacing() * UNMET_LINES_ACROSS > reach {
                finer(1)
            } else {
                Step::Stop
            };
        }
        if v.probes_hit() < n {
            return finer(1);
        }
        let want = if self.abs_sigma > 0.0 {
            (self.rel_sigma * v.estimate).max(self.abs_sigma)
        } else {
            self.rel_sigma * v.estimate
        };
        let spread = v.sigma * (n as f64).sqrt();
        let need = (n as f64 * (v.sigma / want).powi(2) * 1.1).ceil();
        let more = (need.min(1e9) as usize)
            .max(n + 1)
            .next_multiple_of(PROBE_BLOCK);
        let probes = (more <= self.max_probes).then_some((more - n) as f64);
        let alpha = before
            .filter(|&(l, s)| l < level && s > spread)
            .map_or(ALPHA, |(l, s)| {
                ((s / spread).log2() / f64::from(level - l)).clamp(1.0, ALPHA_MAX)
            });
        let levels_for = |alpha: f64| {
            (1..=self.last.saturating_sub(level)).find(|&j| {
                spread * (-f64::from(j) * alpha).exp2() / (self.base as f64).sqrt() <= 0.9 * want
            })
        };
        // Predicted out of reach at α: tried at the best rate measured
        // before giving up, as far as that says.
        let levels = levels_for(alpha).or_else(|| levels_for(ALPHA_MAX));
        let cost = |j: u32| self.base as f64 * 4f64.powi(j as i32);
        match (probes, levels) {
            (Some(a), Some(j)) if a <= cost(j) => Step::Trace {
                level,
                probes: (n, more),
            },
            (Some(_), None) => Step::Trace {
                level,
                probes: (n, more),
            },
            (_, Some(j)) => finer(j),
            (None, None) => Step::Stop,
        }
    }
}

/// Lines per probe across the longest side of a group's boxes beyond which
/// a group no line has met is not refined further ([`Model::volumes_to`]):
/// whatever it holds is then far smaller than its boxes, about the cube of
/// the spacing or less.
pub(super) const UNMET_LINES_ACROSS: f64 = 32.0;

/// Drop the boxes inside another one: they add nothing to the union, only
/// lines to try ([`union_pieces`]).
pub(super) fn drop_contained(boxes: &mut Vec<Aabb>) {
    if boxes.len() < 2 {
        return;
    }
    let prims: Vec<(u32, Aabb)> = boxes
        .iter()
        .enumerate()
        .map(|(i, b)| (i as u32, *b))
        .collect();
    let bvh = Bvh::build(&prims);
    let inside = |a: &Aabb, b: &Aabb| (0..3).all(|k| b.lo[k] <= a.lo[k] && a.hi[k] <= b.hi[k]);
    let keep: Vec<bool> = (0..boxes.len())
        .into_par_iter()
        .map(|i| {
            let a = &boxes[i];
            let mut covered = false;
            bvh.query(a.center(), |j| {
                let j = j as usize;
                // Identical boxes are gone, so only one of two can hold the other.
                covered |= j != i && inside(a, &boxes[j]);
            });
            !covered
        })
        .collect();
    let mut k = keep.iter();
    boxes.retain(|_| *k.next().unwrap());
}

/// Key of a group: its number, or for [`GroupBy::Label`] and groups without
/// one, its name; and its status.
pub(super) type GroupKey = (Option<i64>, String, Status);

pub(super) fn group_key(g: &Group) -> GroupKey {
    (g.number, g.name.clone(), g.status)
}

/// Every group of inside labels the model can hold, with the cells whose
/// instances make it up; `None` for [`GroupBy::Label`] with too many
/// instances to list.
pub(super) fn expected_groups(m: &Model, by: GroupBy) -> Option<Vec<(Group, Vec<u32>)>> {
    // Cells points can end in: not filled, or a lattice whose elements can
    // be filled by the lattice cell itself.
    let holds = |c: u32| {
        let cell = &m.cells[c as usize];
        cell.unsupported.is_none()
            && match &cell.fill {
                None => true,
                Some(Fill::Universe(_)) => false,
                Some(Fill::Lattice(_)) => m.fill_targets(c).any(|t| t.unit() == cell.unit),
            }
    };
    let group = |number: Option<i64>, name: String, label: Option<Label>| Group {
        number,
        name,
        status: Status::Inside,
        label,
        volume: Volume::empty(0),
    };
    let cells = (0..m.cells.len() as u32).filter(|&c| holds(c));
    if by == GroupBy::Label {
        let inst = m.instances();
        let total: u64 = (0..m.cells.len() as u32)
            .filter(|&c| holds(c))
            .map(|c| inst.count(c))
            .sum();
        if total > MAX_LISTED_INSTANCES {
            return None;
        }
        let mut out = Vec::new();
        for c in cells {
            for id in inst.ids(c) {
                let Some(path) = m.instance_path(id) else {
                    continue;
                };
                let label = Label {
                    status: Status::Inside,
                    path,
                    involved: Vec::new(),
                };
                out.push((group(None, m.describe(&label), Some(label)), vec![c]));
            }
        }
        return Some(out);
    }
    let mut by_number: std::collections::BTreeMap<i64, Vec<u32>> = Default::default();
    for c in cells {
        let cell = &m.cells[c as usize];
        let n = match by {
            GroupBy::Cell => cell.id,
            GroupBy::Material => cell.material,
            GroupBy::Universe => m.units[cell.unit as usize].universe,
            GroupBy::Label => unreachable!(),
        };
        by_number.entry(n).or_default().push(c);
    }
    Some(
        by_number
            .into_iter()
            .map(|(n, cells)| (group(Some(n), n.to_string(), None), cells))
            .collect(),
    )
}

impl Model {
    /// Refine `tracks` level by level ([`Levels::next_step`]), coarsest level
    /// first. At a level, the probes each group wants are traced in jobs of
    /// consecutive probes wanted by the same groups, over the union of their
    /// boxes (`boxes`: the groups' boxes for a grid). `assign` gives each
    /// group of a job its volume from what was traced (one length per probe
    /// traced, `n` of them). `report` sees the tracks before every level.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn refine(
        &self,
        tracks: &mut [Track],
        levels: &Levels,
        reach: &[f64],
        boxes: &mut BoxesOf,
        assign: &mut Assign,
        progress: &Progress,
        report: &mut dyn FnMut(&[Track]),
    ) -> Result<()> {
        let wants = |t: &Track| match t.next {
            Step::Trace { level, probes } => Some((level, probes)),
            _ => None,
        };
        while let Some(level) = tracks.iter().filter_map(|t| wants(t).map(|w| w.0)).min() {
            report(tracks);
            let grid = (levels.grid)(level);
            let at: Vec<usize> = (0..tracks.len())
                .filter(|&i| wants(&tracks[i]).is_some_and(|w| w.0 == level))
                .collect();
            let wanted: FxHashMap<usize, (usize, usize)> = at
                .iter()
                .map(|&i| (i, wants(&tracks[i]).unwrap().1))
                .collect();
            let range = |i: usize| wanted[&i];
            // Jobs: between consecutive ends of the ranges wanted, the groups
            // wanting those probes; neighbours with the same groups merged.
            let mut ends: Vec<usize> = at.iter().flat_map(|&i| [range(i).0, range(i).1]).collect();
            ends.sort_unstable();
            ends.dedup();
            let mut jobs: Vec<(std::ops::Range<usize>, Vec<usize>)> = Vec::new();
            for w in ends.windows(2) {
                let (a, b) = (w[0], w[1]);
                let who: Vec<usize> = at
                    .iter()
                    .copied()
                    .filter(|&i| range(i).0 <= a && b <= range(i).1)
                    .collect();
                match jobs.last_mut() {
                    Some((r, g)) if r.end == a && *g == who => r.end = b,
                    _ if !who.is_empty() => jobs.push((a..b, who)),
                    _ => {}
                }
            }
            let mut parts: FxHashMap<usize, Vec<Volume>> = FxHashMap::default();
            for (probes, who) in jobs {
                let t = std::time::Instant::now();
                let mut b = boxes(tracks, &who, &grid);
                let boxed = t.elapsed().as_secs_f64();
                let bits = |b: &Aabb| [b.lo, b.hi].map(|v| v.map(f64::to_bits));
                b.retain(|b| !b.is_empty());
                b.sort_by_key(bits);
                b.dedup_by_key(|b| bits(b));
                drop_contained(&mut b);
                let traced = self.volumes_in(&grid, probes.clone(), &b, progress)?;
                for (i, v) in who.iter().zip(assign(&traced, &who, probes.len())) {
                    parts.entry(*i).or_default().push(v);
                }
                if log::log_enabled!(log::Level::Debug) {
                    let lines: usize = probes
                        .clone()
                        .map(|p| {
                            b.iter()
                                .map(|x| grid.lattice_in(p, x).count())
                                .sum::<usize>()
                        })
                        .sum();
                    let ns = |a: &std::sync::atomic::AtomicU64| {
                        a.swap(0, std::sync::atomic::Ordering::Relaxed) as f64 * 1e-9
                    };
                    log::debug!(
                        "level {level} (h {}) probes {}..{}: groups {} (unmet {}) boxes {} ({:.3e} cm3) \
                         lines {lines} pieces {} length {:.3e} in {:.2} s (boxes {:.2} s; thread time: \
                         pieces {:.1} s, tracing {:.1} s)",
                        grid.h,
                        probes.start,
                        probes.end,
                        who.len(),
                        who.iter().filter(|&&i| tracks[i].volume.segments == 0).count(),
                        b.len(),
                        b.iter().map(|x| x.volume()).sum::<f64>(),
                        PIECES.swap(0, std::sync::atomic::Ordering::Relaxed),
                        LENGTH.swap(0, std::sync::atomic::Ordering::Relaxed) as f64,
                        t.elapsed().as_secs_f64(),
                        boxed,
                        ns(&PIECE_NS),
                        ns(&TRACE_NS),
                    );
                }
            }
            for &i in &at {
                let fresh = range(i).0 == 0;
                let t = &mut tracks[i];
                let mut v = if fresh {
                    if let Some(l) = t.level.filter(|_| t.volume.segments > 0) {
                        t.before =
                            Some((l, t.volume.sigma * (t.volume.lengths.len() as f64).sqrt()));
                    }
                    Volume::empty(0)
                } else {
                    t.volume.clone()
                };
                for p in parts.remove(&i).unwrap_or_default() {
                    v.append(&p);
                }
                v.finish(&grid);
                t.volume = v;
                t.level = Some(level);
                t.next = levels.next_step(&t.volume, level, t.before, reach[i]);
            }
        }
        Ok(())
    }

    /// Volumes grouped by `target.by`, those for which `wanted` holds
    /// refined until `sigma / estimate` is at most `target.rel_sigma` and
    /// every probe met them.
    ///
    /// The groups are those the model can hold, not only those the first
    /// lines met: every cell, material or universe with a cell that may
    /// hold something in the region (by label, every instance, up to
    /// [`MAX_LISTED_INSTANCES`]) is listed, met or not. Each round halves
    /// `h` and traces new lines (seeds drawn from `grid.seed`) over the
    /// conservative boxes of the cells of the groups still above the
    /// target, every instance placed in the global frame, so the lines
    /// cover all of each group however little of it the coarser lines
    /// met; overlapping boxes are traced once. A group's volume is replaced
    /// by the one from the finer lines. Gaps, overlaps and other groups
    /// without cells use the box of their segments, padded by the old
    /// spacing. Rounds stop when every wanted group is there or `h` would
    /// drop below `target.h_min`: those left are in
    /// [`Converged::unconverged`], those never met also in
    /// [`Converged::unseen`]. Groups whose boxes are empty in the region
    /// are in [`Converged::empty`], with volume 0. `progress` counts each
    /// round's chunks.
    ///
    /// With `each`, it is handed the volumes so far before every further
    /// round (after the first pass and after each round but the last), as
    /// they would be returned if it stopped there: groups yet to be refined
    /// are in [`Converged::unconverged`]. For showing them while it runs,
    /// and keeping them if it is cancelled.
    pub fn volumes_to(
        &self,
        grid: &VolumeGrid,
        target: &VolumeTarget,
        wanted: impl Fn(&Group) -> bool,
        progress: Option<&Progress>,
        mut each: Option<&mut dyn FnMut(&Converged)>,
    ) -> Result<Converged> {
        let none = Progress::new();
        let progress = progress.unwrap_or(&none);
        if !(target.rel_sigma > 0.0 && target.h_min > 0.0 && target.abs_sigma >= 0.0) {
            return Err(Error::Invalid(format!(
                "target relative sigma and h_min must be positive, abs_sigma not negative:                  {target:?}"
            )));
        }
        let by = target.by;
        let first = self.grid_volumes(grid, progress)?;
        let mut groups = first.grouped(self, by);
        let none = |g: &VolumeGrid| {
            let mut v = Volume::empty(g.probes);
            v.finish(g);
            v
        };

        // The cells of each group: listed from the model, or (for groups
        // beyond the listing) from the labels met.
        let expected = expected_groups(self, by);
        let listed_all = expected.is_some();
        let mut cells_of: FxHashMap<GroupKey, Vec<u32>> = FxHashMap::default();
        let mut known: FxHashSet<GroupKey> = groups.iter().map(group_key).collect();
        for (mut g, cells) in expected.unwrap_or_default() {
            let k = group_key(&g);
            if known.insert(k.clone()) {
                g.volume = none(grid);
                groups.push(g);
            }
            cells_of.insert(k, cells);
        }
        let mut from_labels: FxHashMap<GroupKey, Vec<u32>> = FxHashMap::default();
        for l in &first.labels {
            let Some(c) = l.cell().filter(|_| l.status == Status::Inside) else {
                continue;
            };
            let cell = &self.cells[c as usize];
            let number = match by {
                GroupBy::Cell => Some(cell.id),
                GroupBy::Material => Some(cell.material),
                GroupBy::Universe => Some(self.units[cell.unit as usize].universe),
                GroupBy::Label => None,
            };
            let name = number.map_or_else(|| self.describe(l), |n| n.to_string());
            let k = (number, name, Status::Inside);
            if !cells_of.contains_key(&k) {
                from_labels.entry(k).or_default().push(c);
            }
        }
        for (k, mut cells) in from_labels {
            cells.sort_unstable();
            cells.dedup();
            cells_of.insert(k, cells);
        }
        // As `Volumes::grouped` orders them.
        groups.sort_by(|a, b| {
            (a.status != Status::Inside, a.number, &a.name).cmp(&(
                b.status != Status::Inside,
                b.number,
                &b.name,
            ))
        });
        let cells: Vec<Option<&Vec<u32>>> =
            groups.iter().map(|g| cells_of.get(&group_key(g))).collect();

        let t = std::time::Instant::now();
        let placed = PlacedBoxes::new(self, grid.region);
        if log::log_enabled!(log::Level::Debug) {
            log::debug!(
                "placed boxes in {:.2} s: {} boxes",
                t.elapsed().as_secs_f64(),
                placed.boxes.iter().map(Vec::len).sum::<usize>()
            );
        }
        let empty: Vec<usize> = (0..groups.len())
            .filter(|&i| {
                groups[i].volume.segments == 0
                    && cells[i].is_some_and(|cs| cs.iter().all(|&c| placed.boxes(c).is_empty()))
            })
            .collect();
        let want: Vec<bool> = groups
            .iter()
            .enumerate()
            .map(|(i, g)| wanted(g) && empty.binary_search(&i).is_err())
            .collect();
        // Longest side of each group's boxes, for giving up on groups no
        // line meets.
        let reach: Vec<f64> = cells
            .iter()
            .map(|cs| {
                cs.map_or(f64::INFINITY, |cs| {
                    cs.iter()
                        .flat_map(|&c| placed.boxes(c))
                        .flat_map(|b| (0..3).map(|k| b.hi[k] - b.lo[k]))
                        .fold(0.0, f64::max)
                })
            })
            .collect();
        // Level 0 is the first grid; each further one halves h, with its
        // own seed, down to h_min.
        let grid_at = |l: u32| VolumeGrid {
            h: grid.h / f64::from(l).exp2(),
            seed: if l == 0 {
                grid.seed
            } else {
                grid.seed ^ u64::from(l).wrapping_mul(0x9e3779b97f4a7c15)
            },
            ..grid.clone()
        };
        let mut last = 0;
        while last < 60 && grid_at(last + 1).h >= target.h_min {
            last += 1;
        }
        let levels = Levels {
            grid: &grid_at,
            last,
            rel_sigma: target.rel_sigma,
            abs_sigma: target.abs_sigma,
            base: grid.probes,
            max_probes: target.max_probes,
        };
        let mut tracks: Vec<Track> = groups
            .iter()
            .enumerate()
            .map(|(i, g)| Track {
                level: Some(0),
                volume: g.volume.clone(),
                before: None,
                next: if want[i] {
                    levels.next_step(&g.volume, 0, None, reach[i])
                } else {
                    Step::Done
                },
            })
            .collect();
        let state = |tracks: &[Track], groups: &[Group]| {
            let mut groups = groups.to_vec();
            let mut h = vec![grid.h; groups.len()];
            for (i, t) in tracks.iter().enumerate() {
                groups[i].volume = t.volume.clone();
                h[i] = grid_at(t.level.unwrap_or(0)).h;
            }
            let unconverged: Vec<usize> = (0..groups.len())
                .filter(|&i| want[i] && tracks[i].next != Step::Done)
                .collect();
            let unseen = unconverged
                .iter()
                .copied()
                .filter(|&i| groups[i].volume.segments == 0)
                .collect();
            Converged {
                groups,
                h,
                unconverged,
                unseen,
                empty: empty.clone(),
                listed_all,
            }
        };
        // Each cell covered once, by boxes about COVER_SPACINGS of the
        // spacing across; groups without cells (gaps, overlaps) by the box
        // of their segments, padded by the spacing that found them.
        let mut boxes = |tracks: &[Track], at: &[usize], g: &VolumeGrid| {
            let mut todo: Vec<u32> = Vec::new();
            let mut boxes: Vec<Aabb> = Vec::new();
            for &i in at {
                match cells[i] {
                    Some(cs) => todo.extend_from_slice(cs),
                    None => {
                        let pad = grid_at(tracks[i].level.unwrap_or(0)).spacing();
                        let b = tracks[i].volume.bbox;
                        boxes.push(
                            Aabb {
                                lo: b.lo.map(|x| x - pad),
                                hi: b.hi.map(|x| x + pad),
                            }
                            .intersect(&grid.region),
                        );
                    }
                }
            }
            todo.sort_unstable();
            todo.dedup();
            let side = COVER_SPACINGS * g.spacing();
            let covers: Vec<Vec<Aabb>> = todo
                .par_iter()
                .map(|&c| {
                    let mut out = Vec::new();
                    placed.cover(c, side, &mut out);
                    out
                })
                .collect();
            boxes.extend(covers.into_iter().flatten());
            boxes
        };
        let mut assign = |traced: &Volumes, at: &[usize], n: usize| {
            let mut found: FxHashMap<GroupKey, Volume> = traced
                .grouped(self, by)
                .into_iter()
                .map(|g| (group_key(&g), g.volume))
                .collect();
            at.iter()
                .map(|&i| {
                    found
                        .remove(&group_key(&groups[i]))
                        .unwrap_or_else(|| Volume::empty(n))
                })
                .collect()
        };
        let mut report = |tracks: &[Track]| {
            if let Some(each) = each.as_mut() {
                each(&state(tracks, &groups));
            }
        };
        self.refine(
            &mut tracks,
            &levels,
            &reach,
            &mut boxes,
            &mut assign,
            progress,
            &mut report,
        )?;
        Ok(state(&tracks, &groups))
    }
}
