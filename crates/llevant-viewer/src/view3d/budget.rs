//! What the 3D view traces next, so that each frame's tracing fits a time
//! budget whatever the GPU (or CPU) and model.
//!
//! A view is traced in two levels. **Low**: the whole view at a reduced
//! scale `s`, chosen from the measured cost per ray so that it fits the
//! budget; shown upscaled while the camera moves. **Full**: once the camera
//! stops, the whole view at full resolution in one batch, its tiles (from
//! the centre outwards) giving the order traced; the backend lands its
//! blocks as they are done. One batch pays the latency of the slowest rays
//! once, where a batch per tile paid it per tile (1.3–1.9× slower on the
//! reference model). Any change of camera or visibility starts again from
//! Low.
//!
//! Pure logic: the caller submits each [`Batch`] and reports its time.

use std::collections::VecDeque;

use llevant_gpu::Tile;

/// Tiles of the full level are multiples of this many pixels (the
/// refinement mask's block size), except at the image's edges.
pub const BLOCK: u32 = 8;

/// Side of the full level's tiles, which only order its blocks.
const FULL_TILE: u32 = 64;

/// Longest a batch should take by the measured cost beyond its fixed part,
/// whatever the budget: a new camera's batches wait behind the ones in
/// flight on the CPU. The fixed part (the latency of the slowest rays) is
/// paid whatever the batch, so smaller batches would only pay it more
/// often. (The GPU traces in short slices, so a long batch holds neither
/// the window's frames nor, cancelled, a new camera's.)
pub const MAX_BATCH_SECONDS: f64 = 0.25;

/// Most a batch may grow over the largest one measured of its level.
const GROWTH: f64 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// The whole view at a reduced size.
    Low { width: u32, height: u32 },
    /// Tiles of the view at full size.
    Full,
}

/// Work for one frame: tiles of one level's image.
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    /// The generation (camera and visibility) traced.
    pub generation: u64,
    pub level: Level,
    pub tiles: Vec<Tile>,
    /// The last batch of its level: a Low image is complete with it.
    pub completes_level: bool,
}

impl Batch {
    pub fn rays(&self) -> u64 {
        self.tiles.iter().map(Tile::pixels).sum()
    }
}

/// Time of a batch of `n` rays: `fixed + per_ray · n`. Small batches do
/// not fill a GPU, so their time is mostly the latency of their slowest
/// rays (`fixed`), not their number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cost {
    pub fixed: f64,
    pub per_ray: f64,
}

impl Cost {
    pub fn seconds(&self, rays: f64) -> f64 {
        self.fixed + self.per_ray * rays
    }

    /// Rays that fit `seconds` (none when the fixed part is over it).
    pub fn rays_within(&self, seconds: f64) -> f64 {
        ((seconds - self.fixed) / self.per_ray).max(0.0)
    }

    /// Least squares over `(rays, seconds)` samples; a proportional fit
    /// when their sizes are too alike to tell the parts apart, or when the
    /// line would not have both parts positive.
    pub fn fit(samples: &VecDeque<(f64, f64)>) -> Cost {
        let n = samples.len() as f64;
        let (sx, sy) = samples
            .iter()
            .fold((0.0, 0.0), |(a, b), &(x, y)| (a + x, b + y));
        let proportional = Cost {
            fixed: 0.0,
            per_ray: (sy / sx).max(1e-15),
        };
        let (lo, hi) = samples
            .iter()
            .fold((f64::MAX, 0.0f64), |(l, h), &(x, _)| (l.min(x), h.max(x)));
        if samples.len() < 3 || hi < 1.5 * lo {
            return proportional;
        }
        let (mx, my) = (sx / n, sy / n);
        let (sxx, sxy) = samples.iter().fold((0.0, 0.0), |(a, b), &(x, y)| {
            (a + (x - mx) * (x - mx), b + (x - mx) * (y - my))
        });
        let per_ray = sxy / sxx;
        let fixed = my - per_ray * mx;
        if !(per_ray > 0.0) || !(fixed >= 0.0) {
            return proportional;
        }
        Cost { fixed, per_ray }
    }
}

#[derive(Clone, Debug)]
struct Job {
    level: Level,
    tiles: VecDeque<Tile>,
    done: u64,
}

/// Index of a level's cost model (the levels may run on different
/// hardware).
fn kind(level: Level) -> usize {
    match level {
        Level::Low { .. } => 0,
        Level::Full => 1,
    }
}

pub struct Scheduler {
    /// Seconds a preview image (and a batch of tiles) should take.
    pub budget: f64,
    /// Smallest scale of the Low level.
    pub min_scale: f64,
    /// Recent batches per level: rays and seconds.
    samples: [VecDeque<(f64, f64)>; 2],
    cost: [Option<Cost>; 2],
    width: u32,
    height: u32,
    generation: u64,
    /// Scale of the last Low level.
    scale: f64,
    job: Option<Job>,
    /// A restart asked for while a multi-batch Low job runs: done when it
    /// ends, so that slow devices still show complete images.
    restart_pending: bool,
    /// The fastest Low batch since the costs were last forgotten: mostly
    /// the latency of its slowest rays (the first previews are the
    /// smallest), which bigger previews pay too.
    floor: Option<f64>,
}

impl Scheduler {
    pub fn new(budget: f64) -> Scheduler {
        Scheduler {
            budget,
            min_scale: 1.0 / 16.0,
            samples: [VecDeque::new(), VecDeque::new()],
            cost: [None, None],
            width: 1,
            height: 1,
            generation: 0,
            // The first preview is the smallest: nothing is measured yet.
            scale: 1.0 / 16.0,
            job: None,
            restart_pending: false,
            floor: None,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Time of a batch of `level`, as measured so far.
    pub fn cost(&self, level: Level) -> Option<Cost> {
        self.cost[kind(level)]
    }

    /// Scale of the last Low level.
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Forget the measured costs (another backend, another model).
    pub fn reset_cost(&mut self) {
        self.cost = [None, None];
        self.samples = [VecDeque::new(), VecDeque::new()];
        self.scale = self.min_scale;
        self.floor = None;
    }

    /// What the next preview's size is chosen by: the latency floor as the
    /// fixed part, and the cost per ray as the slope of the samples about
    /// it. A least-squares line needs samples of different sizes to tell
    /// the parts apart; previews of one size (as they settle) made it
    /// proportional, and a latency-bound GPU then kept them at 37×69 on
    /// the reference model, where 528×985 took 2× the time, not 200×.
    fn low_cost(&self) -> Option<Cost> {
        let fixed = self.floor?;
        let (sxy, sxx) = self.samples[0].iter().fold((0.0, 0.0), |(a, b), &(n, t)| {
            (a + n * (t - fixed), b + n * n)
        });
        Some(Cost {
            fixed,
            per_ray: (sxy / sxx).max(1e-15),
        })
    }

    /// Rays a Low batch should hold, by `c`: those that fit the budget, but
    /// at least half the budget's worth beyond the fixed part, even when
    /// that part (slow single rays) is most of the budget: much more done
    /// for a little more time. Never more than [`MAX_BATCH_SECONDS`]' worth
    /// beyond the fixed part, nor more than [`GROWTH`] times the largest
    /// batch measured: GPU time grows faster than the fit beyond what it
    /// saw (copper isolated on the reference model: 128² rays in 0.23 s,
    /// 420² in 0.73 s).
    fn rays_for(&self, c: Cost) -> f64 {
        let seen = self.samples[0]
            .iter()
            .fold(1.0f64, |m, &(rays, _)| m.max(rays));
        c.rays_within(self.budget)
            .max(0.5 * self.budget / c.per_ray)
            .min(MAX_BATCH_SECONDS / c.per_ray)
            .min(GROWTH * seen)
            .max(1.0)
    }

    /// The view changed (camera, visibility, size): trace it again from
    /// the Low level. Returns the new generation, or the current one when
    /// the restart waits for a Low image in progress.
    pub fn restart(&mut self, width: u32, height: u32) -> u64 {
        let (width, height) = (width.max(1), height.max(1));
        let busy = matches!(
            &self.job,
            Some(j) if matches!(j.level, Level::Low { .. }) && j.done > 0 && !j.tiles.is_empty()
        );
        if busy && (width, height) == (self.width, self.height) {
            self.restart_pending = true;
            return self.generation;
        }
        self.width = width;
        self.height = height;
        self.generation += 1;
        self.restart_pending = false;
        self.job = Some(self.low_job());
        self.generation
    }

    fn low_job(&mut self) -> Job {
        let pixels = self.width as f64 * self.height as f64;
        let cost = self.cost[0];
        if let Some(c) = self.low_cost() {
            let s = (self.rays_for(c) / pixels)
                .sqrt()
                .clamp(self.min_scale, 1.0);
            // Hysteresis: keep the scale unless it is off by over 15 %, so
            // that the resolution does not flicker.
            if !((s / self.scale - 1.0).abs() < 0.15) {
                self.scale = s;
            }
        }
        let w = ((self.width as f64 * self.scale).round() as u32).clamp(1, self.width);
        let h = ((self.height as f64 * self.scale).round() as u32).clamp(1, self.height);
        // One batch, even over the budget (only when even the smallest scale
        // is): each batch waits for a frame to be sent, so strips that fit
        // the budget made previews many frames late (2 a second instead of
        // 26 at 4 ms on the reference model). Strips only within the
        // longest a batch may take.
        let rows = match cost {
            Some(c) if c.per_ray * w as f64 * h as f64 > MAX_BATCH_SECONDS => {
                ((MAX_BATCH_SECONDS / c.per_ray / w as f64) as u32).clamp(1, h)
            }
            _ => h,
        };
        let tiles: VecDeque<Tile> = (0..h)
            .step_by(rows as usize)
            .map(|y| Tile {
                x: 0,
                y,
                width: w,
                height: rows.min(h - y),
            })
            .collect();
        Job {
            level: Level::Low {
                width: w,
                height: h,
            },
            done: 0,
            tiles,
        }
    }

    fn full_job(&self) -> Job {
        let side = FULL_TILE;
        let (w, h) = (self.width, self.height);
        let mut tiles: Vec<Tile> = Vec::new();
        for y in (0..h).step_by(side as usize) {
            for x in (0..w).step_by(side as usize) {
                tiles.push(Tile {
                    x,
                    y,
                    width: side.min(w - x),
                    height: side.min(h - y),
                });
            }
        }
        // From the centre out.
        let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
        let dist = |t: &Tile| {
            let dx = t.x as f64 + t.width as f64 / 2.0 - cx;
            let dy = t.y as f64 + t.height as f64 / 2.0 - cy;
            dx * dx + dy * dy
        };
        tiles.sort_by(|a, b| dist(a).total_cmp(&dist(b)));
        Job {
            level: Level::Full,
            done: 0,
            tiles: tiles.into(),
        }
    }

    /// Level of the next batch, `None` when all is done.
    pub fn next_level(&self) -> Option<Level> {
        self.job
            .as_ref()
            .filter(|j| !j.tiles.is_empty())
            .map(|j| j.level)
    }

    /// The next batch to trace, `None` when all is done: a tile of the Low
    /// level (strips only on devices too slow for the whole of it), or the
    /// whole Full level.
    pub fn next_batch(&mut self) -> Option<Batch> {
        let job = self.job.as_mut()?;
        let tiles: Vec<Tile> = match job.level {
            Level::Low { .. } => vec![job.tiles.pop_front()?],
            Level::Full => job.tiles.drain(..).collect(),
        };
        if tiles.is_empty() {
            return None;
        }
        job.done += tiles.iter().map(Tile::pixels).sum::<u64>();
        let completes_level = job.tiles.is_empty();
        let level = job.level;
        let generation = self.generation;
        if completes_level {
            self.job = match level {
                Level::Low { .. } if self.restart_pending => {
                    // The next batch starts the new generation.
                    self.restart_pending = false;
                    self.generation += 1;
                    Some(self.low_job())
                }
                Level::Low { .. } => Some(self.full_job()),
                Level::Full => None,
            };
        }
        Some(Batch {
            generation,
            level,
            tiles,
            completes_level,
        })
    }

    /// A batch of `level` with `rays` took `seconds`.
    pub fn record(&mut self, level: Level, rays: u64, seconds: f64) {
        if rays == 0 || !(seconds > 0.0) {
            return;
        }
        let k = kind(level);
        if k == 0 {
            self.floor = Some(self.floor.map_or(seconds, |f| f.min(seconds)));
        }
        self.samples[k].push_back((rays as f64, seconds));
        if self.samples[k].len() > 32 {
            self.samples[k].pop_front();
        }
        self.cost[k] = Some(Cost::fit(&self.samples[k]));
    }

    /// Whether everything of the current generation is traced.
    pub fn idle(&self) -> bool {
        self.job.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOW: Level = Level::Low {
        width: 1,
        height: 1,
    };

    /// Run batches until idle on a device of `cost` seconds per ray;
    /// returns the batches.
    fn run(s: &mut Scheduler, cost: f64) -> Vec<Batch> {
        let mut out = Vec::new();
        while let Some(b) = s.next_batch() {
            s.record(b.level, b.rays(), b.rays() as f64 * cost);
            out.push(b);
            assert!(out.len() < 100_000);
        }
        out
    }

    fn covers(tiles: &[Tile], w: u32, h: u32) -> bool {
        let mut seen = vec![0u8; (w * h) as usize];
        for t in tiles {
            for y in t.y..t.y + t.height {
                for x in t.x..t.x + t.width {
                    seen[(y * w + x) as usize] += 1;
                }
            }
        }
        seen.iter().all(|&n| n == 1)
    }

    #[test]
    fn scale_converges_to_the_budget() {
        // 1080p on a device of 1/3e6 s per ray (the P4000 on the reference
        // model): ~6e4 rays fit 20 ms.
        let cost = 1.0 / 2.9e6;
        let mut s = Scheduler::new(0.02);
        for _ in 0..10 {
            s.restart(1920, 1080);
            let b = s.next_batch().unwrap();
            s.record(b.level, b.rays(), b.rays() as f64 * cost);
        }
        s.restart(1920, 1080);
        let b = s.next_batch().unwrap();
        let t = b.rays() as f64 * cost;
        assert!((0.02 * 0.6..=0.02 * 1.4).contains(&t), "{t}");
        assert!(matches!(b.level, Level::Low { .. }));
        assert!(b.completes_level);
    }

    #[test]
    fn full_level_is_one_batch_covering_the_image_from_the_centre() {
        let cost = 1e-7;
        let mut s = Scheduler::new(0.02);
        s.restart(333, 201);
        let batches = run(&mut s, cost);
        let low: Vec<Tile> = batches
            .iter()
            .filter(|b| matches!(b.level, Level::Low { .. }))
            .flat_map(|b| b.tiles.clone())
            .collect();
        let Level::Low { width, height } = batches[0].level else {
            panic!()
        };
        assert!(covers(&low, width, height));
        let full: Vec<&Batch> = batches.iter().filter(|b| b.level == Level::Full).collect();
        assert_eq!(full.len(), 1);
        assert!(full[0].completes_level);
        let tiles = &full[0].tiles;
        assert!(covers(tiles, 333, 201));
        for t in tiles {
            assert_eq!(t.x % BLOCK, 0);
            assert_eq!(t.y % BLOCK, 0);
        }
        // The first tile holds the centre.
        let t = tiles[0];
        assert!(t.x <= 166 && 166 < t.x + t.width && t.y <= 100 && 100 < t.y + t.height);
        assert!(s.idle());
    }

    #[test]
    fn latency_bound_previews_grow() {
        // 40 ms however few the rays, over the 33 ms budget, and almost
        // nothing per ray (the reference model on the laptop's GPU with
        // the preview work cap): the whole view, not the smallest scale.
        let time = |n: u64| 0.04 + 1e-9 * n as f64;
        let mut s = Scheduler::new(0.033);
        for _ in 0..12 {
            s.restart(1920, 1080);
            let b = s.next_batch().unwrap();
            s.record(b.level, b.rays(), time(b.rays()));
        }
        assert!(s.scale() >= 0.9, "{}", s.scale());
        // and the batch stays whole and within the longest a batch takes.
        s.restart(1920, 1080);
        let b = s.next_batch().unwrap();
        assert!(b.completes_level);
        assert!(time(b.rays()) <= MAX_BATCH_SECONDS);
    }

    #[test]
    fn previews_of_one_size_keep_it() {
        // Settled previews all of one size: the scale stays (a proportional
        // fit of such samples would shrink it, then grow it again).
        let time = |n: u64| 0.02 + 2e-8 * n as f64;
        let mut s = Scheduler::new(0.033);
        let mut scales = Vec::new();
        for _ in 0..80 {
            s.restart(1600, 1000);
            let b = s.next_batch().unwrap();
            s.record(b.level, b.rays(), time(b.rays()));
            scales.push(s.scale());
        }
        let last = &scales[40..];
        assert!(last.iter().all(|&x| x == last[0]), "{last:?}");
    }

    #[test]
    fn fixed_latency_is_measured_apart() {
        // 8 ms however few the rays, then 1e-7 s per ray.
        let time = |n: u64| 0.008 + 1e-7 * n as f64;
        let mut s = Scheduler::new(0.02);
        for k in 0..20 {
            s.restart(1600, 1000 + k % 2);
            let b = s.next_batch().unwrap();
            s.record(b.level, b.rays(), time(b.rays()));
        }
        // Batches of one size cannot tell the parts apart, but the scale
        // still converges to where the time is the budget.
        s.restart(1600, 1000);
        let b = s.next_batch().unwrap();
        let t = time(b.rays());
        assert!((0.02 * 0.7..=0.02 * 1.3).contains(&t), "{t}");
    }

    #[test]
    fn fit_separates_latency_from_throughput() {
        let samples: VecDeque<(f64, f64)> = [1e3, 5e3, 2e4, 1e5, 4e5]
            .iter()
            .map(|&n| (n, 0.008 + 1e-7 * n))
            .collect();
        let c = Cost::fit(&samples);
        assert!((c.fixed - 0.008).abs() < 1e-9, "{c:?}");
        assert!((c.per_ray - 1e-7).abs() < 1e-12, "{c:?}");
        assert!((c.rays_within(0.02) - 1.2e5).abs() < 1.0);
        assert_eq!(c.rays_within(0.005), 0.0);
    }

    #[test]
    fn hysteresis_keeps_the_scale() {
        let mut s = Scheduler::new(0.02);
        s.record(LOW, 1000, 1000.0 * 1e-6);
        s.restart(800, 600);
        let first = s.scale();
        // 5 % slower: same scale.
        s.record(LOW, 1000, 1000.0 * 1.05e-6);
        s.restart(800, 600);
        assert_eq!(s.scale(), first);
    }

    #[test]
    fn restart_waits_for_a_low_image_in_progress() {
        // So slow that even the smallest Low image takes several batches.
        let cost = 1e-3;
        let mut s = Scheduler::new(0.02);
        s.record(LOW, 1, cost);
        let g = s.restart(640, 480);
        let b = s.next_batch().unwrap();
        assert!(!b.completes_level);
        assert_eq!(b.generation, g);
        // The camera moves: the Low image in progress is finished first.
        assert_eq!(s.restart(640, 480), g);
        let mut last = None;
        while let Some(b) = s.next_batch() {
            if b.generation != g {
                break;
            }
            last = Some(b.completes_level);
        }
        assert_eq!(last, Some(true));
        assert_eq!(s.generation(), g + 1);
    }

    #[test]
    fn a_preview_over_the_budget_is_one_batch() {
        // The smallest preview takes ~80 ms, far over a 4 ms budget but
        // under the longest batch: one batch, not strips.
        let cost = 1e-5;
        let mut s = Scheduler::new(0.004);
        s.record(LOW, 1000, 1000.0 * cost);
        s.restart(1920, 1080);
        let b = s.next_batch().unwrap();
        assert!(matches!(b.level, Level::Low { .. }));
        assert!(b.completes_level);
        assert!(b.rays() as f64 * cost > 0.004);
        assert!(b.rays() as f64 * cost <= MAX_BATCH_SECONDS);
    }

    #[test]
    fn resize_restarts_at_once() {
        let mut s = Scheduler::new(0.02);
        s.record(LOW, 1, 1e-3);
        let g = s.restart(640, 480);
        s.next_batch().unwrap();
        assert_eq!(s.restart(800, 480), g + 1);
    }
}
