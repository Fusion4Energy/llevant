//! Work counters per universe level, for benchmarks: how many candidates,
//! surface evaluations and crossings a batch of points or lines costs. They
//! run the same code as [`Model::classify`] and [`Model::trace`], with
//! counting switched on. Behind the `bench` feature.

#[cfg(feature = "bench")]
use {
    crate::error::Result,
    crate::trace::{check_lines, chunk_len, Line, Tracer},
    crate::vec3::Vec3,
    crate::Model,
    rayon::prelude::*,
};

/// Work at one universe level (depth 0 is universe 0), summed over a batch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LevelWork {
    /// Points or line pieces handled at this level.
    pub visits: u64,
    /// Candidate cells considered.
    pub candidates: u64,
    /// Surface side tests (points: while evaluating candidates; lines: at
    /// midpoints between crossings).
    pub surface_evals: u64,
    /// Surfaces intersected with a line (lines only; once per surface,
    /// line and level).
    pub surfaces_crossed: u64,
    /// Cell evaluations at midpoints between crossings (lines only).
    pub midpoints: u64,
}

/// Work of a batch, per level.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Work {
    /// Points or lines in the batch.
    pub items: u64,
    pub levels: Vec<LevelWork>,
}

impl Work {
    pub(crate) fn level(&mut self, depth: usize) -> &mut LevelWork {
        if self.levels.len() <= depth {
            self.levels.resize(depth + 1, LevelWork::default());
        }
        &mut self.levels[depth]
    }

    fn merge(mut self, o: Work) -> Work {
        self.items += o.items;
        for (d, l) in o.levels.iter().enumerate() {
            let s = self.level(d);
            s.visits += l.visits;
            s.candidates += l.candidates;
            s.surface_evals += l.surface_evals;
            s.surfaces_crossed += l.surfaces_crossed;
            s.midpoints += l.midpoints;
        }
        self
    }

    /// Sum over all levels.
    pub fn total(&self) -> LevelWork {
        self.levels
            .iter()
            .fold(LevelWork::default(), |a, l| LevelWork {
                visits: a.visits + l.visits,
                candidates: a.candidates + l.candidates,
                surface_evals: a.surface_evals + l.surface_evals,
                surfaces_crossed: a.surfaces_crossed + l.surfaces_crossed,
                midpoints: a.midpoints + l.midpoints,
            })
    }

    /// One line per level, values per item of the batch.
    pub fn report(&self) -> String {
        let n = self.items.max(1) as f64;
        let mut s = String::new();
        for (d, l) in self.levels.iter().enumerate() {
            s.push_str(&format!(
                "  level {d}: visits {:.2}, candidates {:.1}, surface evals {:.1}, surfaces crossed {:.1}, midpoints {:.1}\n",
                l.visits as f64 / n,
                l.candidates as f64 / n,
                l.surface_evals as f64 / n,
                l.surfaces_crossed as f64 / n,
                l.midpoints as f64 / n,
            ));
        }
        s
    }
}

#[cfg(feature = "bench")]
impl Model {
    /// Work of classifying `points` (same results as [`Model::classify`]).
    pub fn work_points(&self, points: &[Vec3]) -> Work {
        points
            .par_chunks(2048)
            .map(|chunk| {
                let mut w = Work::default();
                for &p in chunk {
                    self.classify_counted(p, &mut w);
                }
                w.items = chunk.len() as u64;
                w
            })
            .reduce(Work::default, Work::merge)
    }

    /// Work of tracing `lines` (same segments as [`Model::trace`]).
    pub fn work_lines(&self, lines: &[Line]) -> Result<Work> {
        check_lines(lines)?;
        Ok(lines
            .par_chunks(chunk_len(lines.len()))
            .map(|chunk| {
                let mut tr = Tracer::new(self, false);
                tr.work = Some(Work::default());
                for l in chunk {
                    tr.trace_one(l);
                }
                let mut w = tr.work.take().unwrap();
                w.items = chunk.len() as u64;
                w
            })
            .reduce(Work::default, Work::merge))
    }
}
