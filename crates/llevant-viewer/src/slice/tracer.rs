//! Rows of a slice traced in the background (exact, CPU): every fourth row
//! first, so that a coarse image shows at once, then the rest, in chunks
//! sent back as they finish. A newer job cancels older ones. Gaps and
//! problems are coloured as their patterns ([`colours::marked`]).

use llevant::Depth;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use llevant::{Model, Slice};
use llevant_render::colours::{self, ColourBy, Mark, Paint, Rgb};
use llevant_render::field::Overlay;
use llevant_scene::materials::Looks;

/// Traced rows of one job.
pub struct Rows {
    pub generation: u64,
    /// Row indices (from the top) and per row, `nx` colours, `nx` keys
    /// (equal keys: same label) and `nx` deepest cells (`NONE` where no
    /// single cell holds the point).
    pub rows: Vec<usize>,
    pub colours: Vec<Rgb>,
    pub keys: Vec<u64>,
    pub cells: Vec<u32>,
    /// Gaps, overlaps and other problems met, each once with a point of it.
    pub problems: Vec<(llevant::Label, llevant::Vec3)>,
}

/// No cell.
pub const NONE: u32 = u32::MAX;

fn key(l: &llevant::Label) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    l.hash(&mut h);
    h.finish()
}

/// Rows in the order they are traced: every fourth, then every other,
/// then the rest.
pub fn row_order(ny: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..ny).step_by(4).collect();
    order.extend((2..ny).step_by(4));
    order.extend((1..ny).step_by(2));
    order
}

/// Trace `slice` on the rayon pool, sending chunks of rows to `tx` while
/// `current` still holds `generation`. A pixel of it is `scale` pixels of
/// the screen across and down (for the patterns' size).
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    model: Arc<Model>,
    looks: Arc<Looks>,
    field: Option<Overlay>,
    slice: Slice,
    by: ColourBy,
    level: Option<usize>,
    scale: [usize; 2],
    generation: u64,
    current: Arc<AtomicU64>,
    tx: Sender<Rows>,
    on_rows: Arc<dyn Fn() + Send + Sync>,
) {
    rayon::spawn(move || {
        let lines = slice.rows();
        let order = row_order(slice.ny);
        // Chunks large enough to keep the threads busy, small enough to
        // show progress.
        let chunk = (4 * rayon::current_num_threads()).max(16);
        for rows in order.chunks(chunk) {
            if current.load(Ordering::Relaxed) != generation {
                return;
            }
            let chunk_lines: Vec<_> = rows.iter().map(|&r| lines[r]).collect();
            let Ok(tr) = model.trace(&chunk_lines, Depth::from(level)) else {
                return;
            };
            let mut colours = Vec::with_capacity(rows.len() * slice.nx);
            let mut keys = Vec::with_capacity(rows.len() * slice.nx);
            let mut cells = Vec::with_capacity(rows.len() * slice.nx);
            let paint = Paint::new(&model, &looks, by)
                .with_field(field.as_ref())
                .with_level(level);
            let per_point = paint.per_point();
            let mut label_colour: HashMap<u32, (Rgb, u64, u32, Option<Mark>)> = HashMap::new();
            let mut problems = Vec::new();
            for (n, &row) in rows.iter().enumerate() {
                let range = tr.segments(n);
                let mut s = range.start;
                for i in 0..slice.nx {
                    let t = slice.width * (i as f64 + 0.5) / slice.nx as f64;
                    while s + 1 < range.end && tr.t1[s] <= t {
                        s += 1;
                    }
                    let id = tr.ids[s];
                    let (c, k, cell, mark) = *label_colour.entry(id).or_insert_with(|| {
                        let l = &tr.labels[id as usize];
                        if !matches!(
                            l.status,
                            llevant::Status::Inside | llevant::Status::OnSurface
                        ) {
                            problems.push((l.clone(), slice.pixel_centre(i, row)));
                        }
                        let c = if per_point {
                            paint.label_colour(l)
                        } else {
                            paint.point_colour(l, slice.origin)
                        };
                        // Blank (above the level shown): one key, no cell.
                        if paint.blank(l) {
                            return (c, 0, NONE, None);
                        }
                        (c, key(l), l.cell().unwrap_or(NONE), paint.mark(l))
                    });
                    colours.push(if let Some(m) = mark {
                        let (x, y) = (i * scale[0], row * scale[1]);
                        colours::marked(m, x as i64, y as i64)
                    } else if per_point {
                        paint.point_colour(&tr.labels[id as usize], slice.pixel_centre(i, row))
                    } else {
                        c
                    });
                    keys.push(k);
                    cells.push(cell);
                }
            }
            if tx
                .send(Rows {
                    generation,
                    rows: rows.to_vec(),
                    colours,
                    keys,
                    cells,
                    problems,
                })
                .is_err()
            {
                return;
            }
            on_rows();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_once() {
        for ny in [1, 2, 3, 7, 100, 101] {
            let mut o = row_order(ny);
            o.sort();
            assert_eq!(o, (0..ny).collect::<Vec<_>>(), "{ny}");
        }
    }
}
