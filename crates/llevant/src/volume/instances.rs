//! Volumes of sets of cell instances (tally bins) and of chosen cells.

use super::*;

impl Model {
    /// The volume of each set of instances ([`crate::instance`]), refined
    /// until `sigma / estimate` is at most `target.rel_sigma` and every
    /// probe met it, tracing lines only around the sets.
    ///
    /// Each instance is bounded on its own: its frame followed down its path
    /// (lattice elements too), its cell's program bounded again there and
    /// covered by boxes about 8 spacings across, as
    /// [`Model::volumes_to`] covers cells. Rounds go from coarse to fine, `h`
    /// = `target.h_min · 2^k`: a set joins at the `h` that puts
    /// 8 lines across its largest box (at most
    /// `target.h_max`), so small sets are not traced at spacings that would
    /// miss them, and stays until it
    /// reaches the target. Each round traces the union of the boxes of the
    /// sets in it once and replaces their volumes. Sets no line has met stop
    /// as in `volumes_to` ([`BinVolumes::unseen`]). Beyond
    /// 2^18 instances, the largest sets use the boxes of
    /// their cells at every placement instead. `progress` counts each
    /// round's chunks. Tally bins are sets of instances
    /// ([`Instances::matching`](crate::Instances::matching) of their chain).
    pub fn instance_volumes(
        &self,
        sets: &[Vec<u64>],
        target: &BinTarget,
        progress: Option<&Progress>,
    ) -> Result<BinVolumes> {
        let none = Progress::new();
        let progress = progress.unwrap_or(&none);
        if !(target.rel_sigma > 0.0
            && target.abs_sigma >= 0.0
            && target.h_min > 0.0
            && target.h_min.is_finite()
            && target.h_max >= target.h_min)
        {
            return Err(Error::Invalid(format!(
                "target relative sigma and h_min must be positive, h_max at least h_min: \
                 {target:?}"
            )));
        }
        let region = target.region;
        if !region.is_finite() || region.is_empty() {
            return Err(Error::Invalid(format!(
                "target region must be finite and not empty: {:?} .. {:?}",
                region.lo, region.hi
            )));
        }
        if target.probes < 2 {
            return Err(Error::Invalid(format!(
                "a grid needs at least 2 probes, not {}",
                target.probes
            )));
        }
        let inst = self.instances();
        let n = sets.len();
        // Each set's instances with a path, and the sets of each instance.
        let sets: Vec<Vec<u64>> = sets
            .iter()
            .map(|s| {
                let mut s: Vec<u64> = s
                    .iter()
                    .copied()
                    .filter(|&id| inst.cell(id).is_some())
                    .collect();
                s.sort_unstable();
                s.dedup();
                s
            })
            .collect();
        let mut of_id: FxHashMap<u64, Vec<u32>> = FxHashMap::default();
        for (b, s) in sets.iter().enumerate() {
            for &id in s {
                of_id.entry(id).or_default().push(b as u32);
            }
        }
        // A filled cell's instance holds what is below it: lines are then
        // added to the sets of every level of their labels.
        let any_filled = of_id.keys().any(|&id| {
            inst.cell(id)
                .is_some_and(|c| self.cells[c as usize].fill.is_some())
        });

        // Boxes: instance by instance for the smallest sets, by cell for
        // the others.
        let bounder = Bounder::new(&self.shapes);
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|&b| sets[b].len());
        let mut boxed = vec![false; n];
        let mut budget = MAX_BOXED_INSTANCES;
        for b in order {
            if sets[b].len() <= budget {
                budget -= sets[b].len();
                boxed[b] = true;
            }
        }
        let mut ids: Vec<u64> = (0..n)
            .filter(|&b| boxed[b])
            .flat_map(|b| sets[b].iter().copied())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let placed_ids: FxHashMap<u64, (u32, Placed)> = ids
            .par_iter()
            .filter_map(|&id| place_instance(self, &bounder, region, id).map(|p| (id, p)))
            .collect();
        let cells_of = |b: usize| -> Vec<u32> {
            let mut cs: Vec<u32> = sets[b].iter().filter_map(|&id| inst.cell(id)).collect();
            cs.sort_unstable();
            cs.dedup();
            cs
        };
        let by_cell = (0..n)
            .any(|b| !boxed[b])
            .then(|| PlacedBoxes::new(self, region));
        // Global boxes of each set (before covering).
        let boxes_of = |b: usize| -> Vec<Aabb> {
            if boxed[b] {
                sets[b]
                    .iter()
                    .filter_map(|id| placed_ids.get(id))
                    .map(|(_, p)| p.global)
                    .filter(|g| !g.is_empty())
                    .collect()
            } else {
                let placed = by_cell.as_ref().unwrap();
                cells_of(b)
                    .into_iter()
                    .flat_map(|c| placed.boxes(c).to_vec())
                    .collect()
            }
        };
        let longest = |bs: &[Aabb]| {
            bs.iter()
                .flat_map(|b| (0..3).map(|k| b.hi[k] - b.lo[k]))
                .fold(0.0, f64::max)
        };
        let set_boxes: Vec<Vec<Aabb>> = (0..n).map(boxes_of).collect();
        let reach: Vec<f64> = set_boxes.iter().map(|bs| longest(bs)).collect();
        let empty: Vec<usize> = (0..n)
            .filter(|&b| !sets[b].is_empty() && set_boxes[b].is_empty())
            .collect();
        drop(set_boxes);
        // Level l is h = h_min 2^(top - l): the coarsest a set can start at
        // is level 0, the finest (h_min) level `top`.
        let top = (target.h_max / target.h_min)
            .log2()
            .floor()
            .clamp(0.0, 60.0) as u32;
        let grid_at = |l: u32| {
            let k = top - l.min(top);
            VolumeGrid {
                region,
                h: target.h_min * f64::from(k).exp2(),
                seed: target.seed ^ u64::from(k + 1).wrapping_mul(0x9e3779b97f4a7c15),
                probes: target.probes,
            }
        };
        // The level each set starts at: the coarsest with START_LINES_ACROSS
        // lines across its largest box.
        let finest = grid_at(top).spacing();
        let start: Vec<u32> = (0..n)
            .map(|b| {
                let k = (reach[b] / START_LINES_ACROSS / finest).log2().floor();
                let k = if k.is_finite() {
                    k.clamp(0.0, f64::from(top)) as u32
                } else {
                    0
                };
                top - k
            })
            .collect();
        let traced: Vec<bool> = (0..n)
            .map(|b| !sets[b].is_empty() && empty.binary_search(&b).is_err())
            .collect();
        let levels = Levels {
            grid: &grid_at,
            last: top,
            rel_sigma: target.rel_sigma,
            abs_sigma: target.abs_sigma,
            base: target.probes,
            max_probes: target.max_probes,
        };
        let mut tracks: Vec<Track> = (0..n)
            .map(|b| Track {
                level: None,
                volume: Volume::empty(target.probes),
                before: None,
                next: if traced[b] {
                    Step::Trace {
                        level: start[b],
                        probes: (0, target.probes),
                    }
                } else {
                    Step::Done
                },
            })
            .collect();
        let mut boxes = |_: &[Track], at: &[usize], grid: &VolumeGrid| {
            let side = COVER_SPACINGS * grid.spacing();
            let mut one_by_one: Vec<u64> = Vec::new();
            let mut cells: Vec<u32> = Vec::new();
            for &b in at {
                if boxed[b] {
                    one_by_one.extend_from_slice(&sets[b]);
                } else {
                    cells.extend(cells_of(b));
                }
            }
            one_by_one.sort_unstable();
            one_by_one.dedup();
            cells.sort_unstable();
            cells.dedup();
            let each = (MAX_COVER_BOXES / one_by_one.len().max(1)).clamp(1, COVER_BOXES);
            let covers: Vec<Vec<Aabb>> = one_by_one
                .par_iter()
                .filter_map(|id| placed_ids.get(id))
                .map(|(c, p)| {
                    let mut out = Vec::new();
                    cover_placed(self, &bounder, *c, p, side, each, &mut out);
                    out
                })
                .chain(cells.par_iter().map(|&c| {
                    let mut out = Vec::new();
                    by_cell.as_ref().unwrap().cover(c, side, &mut out);
                    out
                }))
                .collect();
            covers.into_iter().flatten().collect()
        };
        let mut assign = |found: &Volumes, at: &[usize], probes: usize| {
            // Position of each set in `at`.
            let mut pos = vec![u32::MAX; n];
            for (k, &b) in at.iter().enumerate() {
                pos[b] = k as u32;
            }
            let mut fresh: Vec<Volume> = at.iter().map(|_| Volume::empty(probes)).collect();
            let mut touched: Vec<u32> = Vec::new();
            for (l, v) in found.labels.iter().zip(&found.volumes) {
                if l.status != Status::Inside {
                    continue;
                }
                // The sets of the label's instance and, when sets hold
                // filled cells, of those above it; each set once.
                touched.clear();
                let levels = if any_filled { 1 } else { l.path.len() };
                for k in levels..=l.path.len() {
                    if let Some(id) = inst.of(self, &l.path[..k]) {
                        touched.extend(of_id.get(&id).into_iter().flatten());
                    }
                }
                touched.sort_unstable();
                touched.dedup();
                for &b in &touched {
                    if let Some(f) = fresh.get_mut(pos[b as usize] as usize) {
                        f.add(v);
                    }
                }
            }
            fresh
        };
        self.refine(
            &mut tracks,
            &levels,
            &reach,
            &mut boxes,
            &mut assign,
            progress,
            &mut |_| {},
        )?;
        let mut volumes = Vec::with_capacity(n);
        let mut h = Vec::with_capacity(n);
        for t in tracks.iter_mut() {
            let mut v = std::mem::replace(&mut t.volume, Volume::empty(0));
            match t.level {
                Some(l) => h.push(grid_at(l).h),
                None => {
                    v.finish(&grid_at(top));
                    h.push(0.0);
                }
            }
            volumes.push(v);
        }
        let unconverged: Vec<usize> = (0..n)
            .filter(|&b| traced[b] && tracks[b].next != Step::Done)
            .collect();
        let unseen = unconverged
            .iter()
            .copied()
            .filter(|&b| volumes[b].segments == 0)
            .collect();
        Ok(BinVolumes {
            volumes,
            h,
            instances: sets.iter().map(|s| s.len() as u64).collect(),
            unconverged,
            unseen,
            empty,
        })
    }

    /// The volume of each cell of `cells` (indices into [`Model::cells`]),
    /// all its instances together, traced only around them
    /// ([`Model::instance_volumes`], one set per cell). A filled cell's
    /// volume is that of its region, whatever fills it. As groups by cell,
    /// in the order of `cells`, with the outcomes of [`Model::volumes_to`]
    /// (`listed_all` true). For a single pass at the coarsest spacing, give
    /// an infinite `rel_sigma`.
    pub fn cell_volumes(
        &self,
        cells: &[u32],
        target: &BinTarget,
        progress: Option<&Progress>,
    ) -> Result<Converged> {
        if let Some(&c) = cells.iter().find(|&&c| c as usize >= self.cells.len()) {
            return Err(Error::Invalid(format!(
                "no cell with index {c} ({} cells)",
                self.cells.len()
            )));
        }
        let inst = self.instances();
        let sets: Vec<Vec<u64>> = cells.iter().map(|&c| inst.ids(c).collect()).collect();
        let v = self.instance_volumes(&sets, target, progress)?;
        // Cells with no instance (in a universe nothing fills): empty too.
        let mut empty = v.empty;
        empty.extend((0..cells.len()).filter(|&b| v.instances[b] == 0));
        empty.sort_unstable();
        empty.dedup();
        let groups = cells
            .iter()
            .zip(v.volumes)
            .map(|(&c, volume)| {
                let id = self.cell_id(c);
                Group {
                    number: Some(id),
                    name: id.to_string(),
                    status: Status::Inside,
                    label: None,
                    volume,
                }
            })
            .collect();
        Ok(Converged {
            groups,
            h: v.h,
            unconverged: v.unconverged,
            unseen: v.unseen,
            empty,
            listed_all: true,
        })
    }
}
