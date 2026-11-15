//! Cell and surface numbers over the slice, as MCNP's plotter writes them:
//! found once per complete image (in the background, only when asked for)
//! by `llevant_render::labels`, placed so that none overlap, then drawn
//! each frame where that image is on screen.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

use llevant::{Model, Slice};
use llevant_render::labels::{self, Anchor, Grid, Kind};

/// Labels at most on one image.
pub const MAX_LABELS: usize = 400;
/// Text size (points).
pub const FONT_SIZE: f32 = 11.0;

/// Which labels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wanted {
    pub cells: bool,
    pub surfaces: bool,
}

impl Wanted {
    pub fn any(self) -> bool {
        self.cells || self.surfaces
    }
}

/// A label placed: where on its image (pixels), and its text.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub x: f32,
    pub y: f32,
    pub surface: bool,
    pub text: String,
}

/// The labels of one job.
pub struct Done {
    pub generation: u64,
    pub labels: Vec<Placed>,
    /// Time taken (s).
    pub seconds: f64,
}

/// The text of a label: a cell's number, a surface's (with its facet).
pub fn text(m: &Model, kind: Kind) -> String {
    match kind {
        Kind::Cell(c) => m.cell_id(c).to_string(),
        Kind::Surface(s) => m
            .surface_id(s)
            .map(crate::pick::surface_name)
            .unwrap_or_default(),
    }
}

/// Find and place the labels of a traced image on the rayon pool, sending
/// them to `tx` unless `current` has moved past `generation`.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    model: Arc<Model>,
    slice: Slice,
    keys: Vec<u64>,
    cells: Vec<u32>,
    level: Option<usize>,
    wanted: Wanted,
    pixels_per_point: f32,
    generation: u64,
    current: Arc<AtomicU64>,
    tx: Sender<Done>,
    on_done: Arc<dyn Fn() + Send + Sync>,
) {
    rayon::spawn(move || {
        let start = Instant::now();
        let g = Grid {
            nx: slice.nx,
            ny: slice.ny,
            keys: &keys,
            cells: &cells,
        };
        // A label needs about its text's height of room around its centre.
        let char_w = 0.62 * FONT_SIZE * pixels_per_point;
        let line_h = 1.3 * FONT_SIZE * pixels_per_point;
        let o = labels::Options {
            level,
            min_depth: 0.6 * line_h,
            spacing: (8.0 * char_w) as usize,
        };
        let anchors = labels::anchors(&model, &slice, &g, wanted.cells, wanted.surfaces, &o);
        if current.load(Ordering::Relaxed) != generation {
            return;
        }
        let texts: Vec<String> = anchors.iter().map(|a| text(&model, a.kind)).collect();
        // Room for the text and a little margin, in image pixels.
        let size = |k: usize, _: &Anchor| (char_w * (texts[k].len() as f32 + 1.0), line_h);
        let kept = labels::place(&anchors, MAX_LABELS, size);
        let labels = kept
            .into_iter()
            .map(|k| Placed {
                x: anchors[k].x,
                y: anchors[k].y,
                surface: matches!(anchors[k].kind, Kind::Surface(_)),
                text: texts[k].clone(),
            })
            .collect();
        if current.load(Ordering::Relaxed) != generation {
            return;
        }
        let seconds = start.elapsed().as_secs_f64();
        log::debug!(
            "slice labels: {} anchors, {:.1} ms",
            anchors.len(),
            1e3 * seconds
        );
        if tx
            .send(Done {
                generation,
                labels,
                seconds,
            })
            .is_ok()
        {
            on_done();
        }
    });
}

/// Draw `labels` of an image shown at `r` (the image's `nx × ny` pixels
/// stretched over it), clipped by `painter`.
pub fn draw(
    painter: &egui::Painter,
    r: egui::Rect,
    nx: usize,
    ny: usize,
    labels: &[Placed],
    surface_colour: egui::Color32,
) {
    let font = egui::FontId::monospace(FONT_SIZE);
    let halo = egui::Color32::from_black_alpha(200);
    for l in labels {
        let pos = r.min + egui::vec2(l.x / nx as f32 * r.width(), l.y / ny as f32 * r.height());
        if !painter.clip_rect().expand(40.0).contains(pos) {
            continue;
        }
        let colour = if l.surface {
            surface_colour
        } else {
            egui::Color32::WHITE
        };
        let galley = painter.layout_no_wrap(l.text.clone(), font.clone(), colour);
        let at = pos - 0.5 * galley.size();
        let back = painter.layout_no_wrap(l.text.clone(), font.clone(), halo);
        for d in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
            painter.galley(at + egui::vec2(d.0, d.1), back.clone(), halo);
        }
        painter.galley(at, galley, colour);
    }
}
