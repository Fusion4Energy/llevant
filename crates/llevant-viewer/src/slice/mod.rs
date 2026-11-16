//! The 2D slice view: a plane `x`, `y` or `z = at`, traced exactly by the
//! CPU engine one row at a time (`tracer`). Drag to pan, scroll to zoom
//! about the cursor.
//!
//! Two layers: the last complete image, drawn moved and scaled to the
//! current view at once (so panning and zooming follow the pointer before
//! anything is traced), and the trace in progress over it, coarse rows
//! first, rows not traced yet left transparent. Cell and surface numbers
//! (`labels`), when asked for, are found once each complete image is in and
//! drawn over it; a grid of the plane's coordinates (`grid`) over all, so
//! that where the slice is and how large it is can be read off it.

pub mod grid;
pub mod labels;
pub mod tracer;

use rayon::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use llevant::{Aabb, Model, Slice, Vec3};
use llevant_render::colours::{ColourBy, Rgb, OUTLINE};
use llevant_render::field::Overlay;
use llevant_scene::index::CellSet;
use llevant_scene::materials::Looks;

use llevant::vec3::{add, cross, dot, normalize, scale, sub};

use crate::state::{AxisName, OutlineBy, PlaneState, Radial, SliceState};
use tracer::Rows;

/// The plane's two image axes (as `llevant::Slice::axis`).
fn plane_axes(axis: AxisName) -> (usize, usize, usize) {
    match axis {
        AxisName::X => (0, 1, 2),
        AxisName::Y => (1, 0, 2),
        AxisName::Z => (2, 0, 1),
    }
}

/// A plane and its coordinates: points `origin + a·u + b·v`, `n` its unit
/// normal. For `x = at` (and y, z) the coordinates are the model's (y, z;
/// x, z; x, y), as in MCNP plots; for a radial plane, the distance out
/// along the radius and up the axis from the axis point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub origin: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub n: Vec3,
}

impl Frame {
    /// `axis = at`.
    pub fn axis(axis: AxisName, at: f64) -> Frame {
        let (n, a, b) = plane_axes(axis);
        let e = |k: usize| {
            let mut v = [0.0; 3];
            v[k] = 1.0;
            v
        };
        Frame {
            origin: scale(e(n), at),
            u: e(a),
            v: e(b),
            n: e(n),
        }
    }

    /// The plane through `r`'s axis at its angle, moved `at` along its
    /// normal.
    pub fn radial(r: &Radial, at: f64) -> Frame {
        let a = normalize(r.axis);
        let zero = sub(r.zero, scale(a, dot(r.zero, a)));
        let e0 = normalize(zero);
        let e90 = cross(a, e0);
        let (s, c) = r.phi.to_radians().sin_cos();
        let u = add(scale(e0, c), scale(e90, s));
        let n = cross(u, a);
        Frame {
            origin: add(r.point, scale(n, at)),
            u,
            v: a,
            n,
        }
    }

    /// The plane's coordinates of `p`, and how far off it `p` is (signed,
    /// along `n`).
    pub fn coords(&self, p: Vec3) -> ([f64; 2], f64) {
        let q = sub(p, self.origin);
        ([dot(q, self.u), dot(q, self.v)], dot(q, self.n))
    }

    pub fn point(&self, c: [f64; 2]) -> Vec3 {
        add(self.origin, add(scale(self.u, c[0]), scale(self.v, c[1])))
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Spec {
    frame: Frame,
    centre: [f64; 2],
    width: f64,
    /// Height of the view (cm): `width` times the screen's aspect, the same
    /// for traces at reduced resolution.
    height: f64,
    nx: usize,
    ny: usize,
    by: ColourBy,
    /// Changes with the looks of materials (realistic colours) and with
    /// the field.
    paint: u64,
    /// The universe level shown (MCNP's `LEVEL n`), or the deepest cells.
    level: Option<usize>,
    /// Screen pixels per traced pixel across and down (rows are fewer
    /// while the view moves), so that the patterns of gaps and problems
    /// keep their size on screen.
    scale: [usize; 2],
}

/// Longest a trace of the slice should take while the view moves.
const MOVING_SECONDS: f64 = 0.03;

/// While the view moves, every `k`-th row of `ny` is traced: the smallest
/// `k` of 1, 2 and 4 whose rows fit [`MOVING_SECONDS`] at `row_seconds`
/// each (the last complete trace's), 4 before any is measured. A slice
/// costs per row (one line traced across the whole width), not per
/// column, so the columns stay whole.
fn moving_rows(row_seconds: Option<f64>, ny: usize) -> usize {
    let Some(s) = row_seconds else { return 4 };
    [1, 2, 4]
        .into_iter()
        .find(|&k| s * ny.div_ceil(k) as f64 <= MOVING_SECONDS)
        .unwrap_or(4)
}

/// An image and the view it shows.
struct Layer {
    texture: egui::TextureHandle,
    spec: Spec,
}

/// The plane a slice view shows, and how it maps onto the view's
/// rectangle on screen: `width` cm across and `height` cm high, centred on
/// `centre` (in the frame's coordinates).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaneView {
    pub frame: Frame,
    pub centre: [f64; 2],
    pub width: f64,
    pub height: f64,
}

impl PlaneView {
    /// Where `p`, projected onto the plane, is on screen, and how far off
    /// the plane it is (cm).
    pub fn to_screen(&self, rect: egui::Rect, p: Vec3) -> (egui::Pos2, f64) {
        let per_point = self.width / rect.width() as f64;
        let (c, off) = self.frame.coords(p);
        let x = (c[0] - self.centre[0]) / per_point;
        let y = (self.centre[1] - c[1]) / per_point;
        (rect.center() + egui::vec2(x as f32, y as f32), off.abs())
    }

    /// The point of the plane at `pos`.
    pub fn from_screen(&self, rect: egui::Rect, pos: egui::Pos2) -> Vec3 {
        let per_point = self.width / rect.width() as f64;
        self.frame.point([
            self.centre[0] + (pos.x - rect.center().x) as f64 * per_point,
            self.centre[1] - (pos.y - rect.center().y) as f64 * per_point,
        ])
    }

    /// The corners of the view in the model, clockwise from the top left.
    pub fn corners(&self) -> [Vec3; 4] {
        let (w, h) = (0.5 * self.width, 0.5 * self.height);
        let c = self.centre;
        [
            self.frame.point([c[0] - w, c[1] + h]),
            self.frame.point([c[0] + w, c[1] + h]),
            self.frame.point([c[0] + w, c[1] - h]),
            self.frame.point([c[0] - w, c[1] - h]),
        ]
    }

    /// Centimetres per screen point.
    pub fn cm_per_point(&self, rect: egui::Rect) -> f64 {
        self.width / rect.width() as f64
    }
}

/// What happened in the slice view this frame.
pub struct SliceEvents {
    /// Where the view is on screen, and the pointer's interaction with it.
    pub response: egui::Response,
    /// The plane shown, after this frame's panning and zooming.
    pub plane: PlaneView,
    /// The point under the cursor.
    pub hovered: Option<Vec3>,
    /// The deepest cell under the cursor, as traced (no new query).
    pub hovered_cell: Option<u32>,
    /// The point clicked.
    pub clicked: Option<Vec3>,
}

pub struct SliceView {
    model: Arc<Model>,
    looks: Arc<Looks>,
    /// Numbers drawn over the cells.
    field: Option<Overlay>,
    /// Bumped by [`SliceView::set_looks`] and [`SliceView::set_field`].
    paint_generation: u64,
    region: Aabb,
    pub axis: AxisName,
    /// For `axis = at`, where along the axis; for a radial plane, how far
    /// it is moved off its axis, along its normal.
    pub at: f64,
    /// The plane through an axis at an angle, instead of `axis = at`.
    pub radial: Option<Radial>,
    pub centre: [f64; 2],
    pub width: f64,
    /// Height over width of the view on screen (last frame).
    aspect: f64,
    pub outline: bool,
    /// Which boundaries the outlines follow.
    pub outline_by: OutlineBy,
    /// The outlines' colour (none: [`OUTLINE`]).
    pub outline_colour: Option<Rgb>,
    /// A grid of the plane's coordinates over the slice ([`grid`]).
    pub grid: bool,
    /// What [`OutlineBy`] tells apart, per cell of the model (which it is
    /// for, and for what model): materials or universes, made once. (The
    /// model is held so that it cannot be another at the same address.)
    classes: Option<(OutlineBy, Arc<Model>, Arc<Vec<u64>>)>,
    /// The outlines the image shown was composed with: other ones compose
    /// it again (no new trace).
    composed_outline: (bool, OutlineBy, Option<Rgb>),
    pub by: ColourBy,
    /// The universe level shown (MCNP's plotter `LEVEL n`: the rest is
    /// blank), or `None` for the deepest cell of every path.
    pub level: Option<usize>,
    /// Shown where nothing is traced yet.
    pub background: Rgb,
    /// The view being traced, and when its trace started.
    spec: Option<Spec>,
    started: Instant,
    /// Seconds per row of the last complete trace, for how many rows to
    /// trace while the view moves.
    row_seconds: Option<f64>,
    /// Every how many rows are traced while the view moves, chosen when it
    /// starts moving and kept until it stops: chosen per trace, timings
    /// near the limit flip it, and the thin layers come and go.
    moving_k: Option<usize>,
    generation: u64,
    current: Arc<AtomicU64>,
    tx: Sender<Rows>,
    rx: Receiver<Rows>,
    colours: Vec<Rgb>,
    keys: Vec<u64>,
    /// Deepest cell of each pixel (`tracer::NONE` for none).
    cells: Vec<u32>,
    filled: Vec<bool>,
    /// Problems met while tracing (gaps, overlaps...), each once, with a
    /// point of each.
    pub seen: Vec<(llevant::Label, Vec3)>,
    /// Cells tinted and outlined, and the cell under the pointer, outlined.
    selection: Option<Arc<CellSet>>,
    hovered: u32,
    /// Colours of the selection and of the hovered cell's outline.
    pub highlight: [Rgb; 2],
    rows_done: usize,
    image_dirty: bool,
    /// Which numbers to write over the slice.
    pub labels_wanted: labels::Wanted,
    /// The labels of an image (and its view), and the image and choice
    /// last asked for.
    labels: Option<(Spec, Vec<labels::Placed>)>,
    labels_for: Option<(Spec, labels::Wanted)>,
    label_generation: u64,
    label_current: Arc<AtomicU64>,
    label_tx: Sender<labels::Done>,
    label_rx: Receiver<labels::Done>,
    /// The trace in progress (its view is `spec`).
    partial: Option<egui::TextureHandle>,
    /// The last complete image.
    shown: Option<Layer>,
    /// The trace given up for a newer view, until the newer one covers it.
    previous: Option<Layer>,
    /// The view on screen last frame, and when it last changed: while it
    /// moves, slices are traced at a quarter of the resolution.
    last_view: Option<Spec>,
    changed: Instant,
}

impl SliceView {
    pub fn new(
        model: Arc<Model>,
        looks: Arc<Looks>,
        region: Aabb,
        s: &SliceState,
        by: ColourBy,
    ) -> SliceView {
        let (tx, rx) = channel();
        let (label_tx, label_rx) = channel();
        let mut v = SliceView {
            model,
            looks,
            field: None,
            paint_generation: 0,
            region,
            axis: s.axis,
            at: 0.0,
            radial: s.radial,
            centre: [0.0; 2],
            width: 1.0,
            aspect: 0.75,
            outline: s.outline,
            outline_by: s.outline_by,
            outline_colour: s.outline_colour,
            grid: s.grid,
            classes: None,
            composed_outline: (s.outline, s.outline_by, s.outline_colour),
            by,
            level: None,
            background: llevant_render::colours::BACKGROUND,
            spec: None,
            started: Instant::now(),
            row_seconds: None,
            moving_k: None,
            generation: 0,
            current: Arc::new(AtomicU64::new(0)),
            tx,
            rx,
            colours: Vec::new(),
            keys: Vec::new(),
            cells: Vec::new(),
            filled: Vec::new(),
            seen: Vec::new(),
            selection: None,
            hovered: tracer::NONE,
            highlight: [[255, 199, 44], [255, 255, 255]],
            rows_done: 0,
            image_dirty: false,
            labels_wanted: labels::Wanted {
                cells: s.cell_labels,
                surfaces: s.surface_labels,
            },
            labels: None,
            labels_for: None,
            label_generation: 0,
            label_current: Arc::new(AtomicU64::new(0)),
            label_tx,
            label_rx,
            partial: None,
            shown: None,
            previous: None,
            last_view: None,
            changed: Instant::now(),
        };
        v.fit();
        if let Some(at) = s.at {
            v.at = at;
        }
        if let Some(c) = s.centre {
            v.centre = c;
        }
        if let Some(w) = s.width {
            v.width = w;
        }
        v
    }

    /// Trace `model` from now on (a draft of the model shown, with the same
    /// cells): traced again.
    pub fn set_model(&mut self, model: Arc<Model>) {
        self.model = model;
        self.paint_generation += 1;
        self.row_seconds = None;
    }

    /// The model traced.
    pub fn model(&self) -> &Arc<Model> {
        &self.model
    }

    /// New looks of the materials (other specs): traced again.
    pub fn set_looks(&mut self, looks: Arc<Looks>) {
        self.looks = looks;
        self.paint_generation += 1;
    }

    /// Draw `field` over the cells, or none: traced again.
    pub fn set_field(&mut self, field: Option<Overlay>) {
        self.field = field;
        self.paint_generation += 1;
    }

    /// Tint and outline these cells (no new trace).
    pub fn set_selection(&mut self, cells: Arc<CellSet>) {
        self.selection = (!cells.is_empty()).then_some(cells);
        self.image_dirty = true;
    }

    /// Outline the cell under the pointer (no new trace).
    pub fn hover(&mut self, cell: Option<u32>) {
        let c = cell.unwrap_or(tracer::NONE);
        if c != self.hovered {
            self.hovered = c;
            self.image_dirty = true;
        }
    }

    /// The slice as traced, once complete: width, height and RGB rows.
    pub fn image_rgb(&self) -> Option<(usize, usize, Vec<u8>)> {
        let spec = self.spec.as_ref()?;
        if self.rows_done < spec.ny {
            return None;
        }
        let img = self.image(spec);
        let rgb = img
            .pixels
            .iter()
            .flat_map(|c| [c.r(), c.g(), c.b()])
            .collect();
        Some((spec.nx, spec.ny, rgb))
    }

    /// Frame the whole model on the current plane.
    pub fn fit(&mut self) {
        let r = self.region;
        let size = (0..3).map(|k| r.hi[k] - r.lo[k]).fold(0.0, f64::max);
        // Just off the middle: symmetric models often have a surface
        // there, which a slice exactly on it shows as on-surface (grey).
        self.at = match self.radial {
            None => {
                let (n, _, _) = plane_axes(self.axis);
                0.5 * (r.lo[n] + r.hi[n]) + 1e-3 * (r.hi[n] - r.lo[n])
            }
            Some(_) => 1e-3 * size,
        };
        // The region's corners in the plane's coordinates.
        let f = self.frame();
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for k in 0..8 {
            let p = [0, 1, 2].map(|d| if k >> d & 1 == 1 { r.hi[d] } else { r.lo[d] });
            let (c, _) = f.coords(p);
            for i in 0..2 {
                lo[i] = lo[i].min(c[i]);
                hi[i] = hi[i].max(c[i]);
            }
        }
        self.centre = [0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1])];
        self.width = 1.05 * (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-3);
    }

    /// The plane shown, with its coordinates.
    pub fn frame(&self) -> Frame {
        match &self.radial {
            Some(r) => Frame::radial(r, self.at),
            None => Frame::axis(self.axis, self.at),
        }
    }

    /// Names of the plane's two coordinates: the model's for `x = at` (and
    /// y, z), out along the radius and up its axis for a radial plane.
    pub fn axis_names(&self) -> [&'static str; 2] {
        match (self.radial.is_some(), self.axis) {
            (true, _) => ["r", "axis"],
            (false, AxisName::X) => ["y", "z"],
            (false, AxisName::Y) => ["x", "z"],
            (false, AxisName::Z) => ["x", "y"],
        }
    }

    /// Show the plane of `s` (its position and extent where given).
    pub fn set_state(&mut self, s: &SliceState) {
        self.axis = s.axis;
        self.radial = s.radial;
        self.outline = s.outline;
        self.outline_by = s.outline_by;
        self.outline_colour = s.outline_colour;
        self.grid = s.grid;
        self.labels_wanted = labels::Wanted {
            cells: s.cell_labels,
            surfaces: s.surface_labels,
        };
        self.fit();
        if let Some(at) = s.at {
            self.at = at;
        }
        if let Some(c) = s.centre {
            self.centre = c;
        }
        if let Some(w) = s.width {
            self.width = w;
        }
    }

    pub fn state(&self) -> SliceState {
        SliceState {
            axis: self.axis,
            at: Some(self.at),
            radial: self.radial,
            centre: Some(self.centre),
            width: Some(self.width),
            outline: self.outline,
            outline_by: self.outline_by,
            outline_colour: self.outline_colour,
            cell_labels: self.labels_wanted.cells,
            surface_labels: self.labels_wanted.surfaces,
            grid: self.grid,
        }
    }

    /// The labels drawn over the last complete image (for tests).
    pub fn labels_shown(&self) -> &[labels::Placed] {
        self.labels.as_ref().map_or(&[], |(_, l)| l.as_slice())
    }

    /// Whether the labels asked for are found for the image shown.
    pub fn labels_settled(&self) -> bool {
        !self.labels_wanted.any()
            || self
                .labels
                .as_ref()
                .zip(self.labels_for.as_ref())
                .is_some_and(|((a, _), (b, _))| a == b && self.spec.as_ref() == Some(a))
    }

    /// Labels for the complete image `traced` (the view on screen, still)
    /// if they are wanted and not found for it yet; and those found.
    fn update_labels(&mut self, traced: &Spec, ready: bool, ctx: &egui::Context) {
        let wanted = self.labels_wanted;
        if !wanted.any() {
            self.labels = None;
            self.labels_for = None;
            return;
        }
        let ask = (traced.clone(), wanted);
        if ready && self.labels_for.as_ref() != Some(&ask) {
            self.label_generation += 1;
            self.label_current
                .store(self.label_generation, Ordering::Relaxed);
            let ctx2 = ctx.clone();
            labels::spawn(
                self.model.clone(),
                self.slice(traced),
                self.keys.clone(),
                self.cells.clone(),
                traced.level,
                wanted,
                ctx.pixels_per_point(),
                self.label_generation,
                self.label_current.clone(),
                self.label_tx.clone(),
                Arc::new(move || ctx2.request_repaint()),
            );
            self.labels_for = Some(ask);
        }
        for d in self.label_rx.try_iter() {
            if d.generation == self.label_generation {
                if let Some((spec, _)) = &self.labels_for {
                    self.labels = Some((spec.clone(), d.labels));
                }
            }
        }
    }

    /// Which plane is shown (not where the view is on it).
    pub fn plane_state(&self) -> PlaneState {
        PlaneState {
            axis: self.axis,
            radial: self.radial,
            at: self.at,
        }
    }

    /// Show the plane `p`. The view is framed again if the plane is of
    /// another kind (another axis, radial or not); else it stays where it is.
    pub fn set_plane_state(&mut self, p: PlaneState) {
        let kind = |axis: AxisName, radial: Option<Radial>| match radial {
            Some(r) => (None, Some(r.axis.map(f64::to_bits))),
            None => (Some(axis), None),
        };
        let changed = kind(self.axis, self.radial) != kind(p.axis, p.radial);
        self.axis = p.axis;
        self.radial = p.radial;
        self.at = p.at;
        if changed {
            self.fit();
            self.at = p.at;
        }
    }

    /// Range of `at`: along the axis, or off a radial plane's axis.
    pub fn at_range(&self) -> (f64, f64) {
        let r = &self.region;
        match self.radial {
            None => {
                let (n, _, _) = plane_axes(self.axis);
                (r.lo[n], r.hi[n])
            }
            Some(_) => {
                let half = 0.5 * (0..3).map(|k| r.hi[k] - r.lo[k]).fold(0.0, f64::max);
                (-half, half)
            }
        }
    }

    /// Fraction of the rows traced.
    pub fn progress(&self) -> f64 {
        if self.filled.is_empty() {
            0.0
        } else {
            self.rows_done as f64 / self.filled.len() as f64
        }
    }

    /// Whether every row is traced, or the slice was never laid out (its
    /// tab not shown), so that nothing is coming.
    pub fn settled(&self) -> bool {
        // A quarter-resolution image traced while the view moved is not the
        // final one: the full one follows once the view has been still.
        let full = self.spec.as_ref().is_none_or(|s| s.scale == [1, 1]);
        full && (self.filled.is_empty() || self.rows_done >= self.filled.len())
    }

    /// The plane shown.
    pub fn plane(&self) -> PlaneView {
        PlaneView {
            frame: self.frame(),
            centre: self.centre,
            width: self.width,
            height: self.width * self.aspect,
        }
    }

    /// Move to the plane through `p`: along the axis, or turned about a
    /// radial plane's axis; centred on `p`.
    pub fn go_to(&mut self, p: Vec3) {
        if let Some(r) = &mut self.radial {
            // The angle of `p` about the axis.
            let a = normalize(r.axis);
            let e0 = normalize(sub(r.zero, scale(a, dot(r.zero, a))));
            let e90 = cross(a, e0);
            let q = sub(p, r.point);
            let (x, y) = (dot(q, e0), dot(q, e90));
            if x.hypot(y) > 0.0 {
                r.phi = y.atan2(x).to_degrees();
            }
            self.at = 0.0;
        } else {
            let (n, _, _) = plane_axes(self.axis);
            self.at = p[n];
        }
        self.centre = self.frame().coords(p).0;
    }

    fn slice(&self, spec: &Spec) -> Slice {
        let height = spec.height;
        let f = &spec.frame;
        let origin = f.point([
            spec.centre[0] - 0.5 * spec.width,
            spec.centre[1] - 0.5 * height,
        ]);
        Slice {
            origin,
            u: f.u,
            v: f.v,
            width: spec.width,
            height,
            nx: spec.nx,
            ny: spec.ny,
        }
    }

    fn start(&mut self, spec: Spec, ctx: &egui::Context) {
        self.generation += 1;
        self.current.store(self.generation, Ordering::Relaxed);
        let n = spec.nx * spec.ny;
        self.colours = vec![[255; 3]; n];
        self.keys = vec![0; n];
        self.cells = vec![tracer::NONE; n];
        self.filled = vec![false; spec.ny];
        // What the trace given up has done stays on screen.
        if let (Some(texture), Some(old)) = (self.partial.take(), self.spec.take()) {
            if self.rows_done > 0 {
                self.previous = Some(Layer { texture, spec: old });
            }
        }
        self.rows_done = 0;
        self.image_dirty = false;
        self.started = Instant::now();
        let ctx = ctx.clone();
        tracer::spawn(
            self.model.clone(),
            self.looks.clone(),
            self.field.clone(),
            self.slice(&spec),
            spec.by,
            spec.level,
            spec.scale,
            self.generation,
            self.current.clone(),
            self.tx.clone(),
            Arc::new(move || ctx.request_repaint()),
        );
        self.spec = Some(spec);
    }

    fn receive(&mut self) {
        let Some(spec) = &self.spec else { return };
        let nx = spec.nx;
        for r in self.rx.try_iter() {
            if r.generation != self.generation {
                continue;
            }
            for (k, &row) in r.rows.iter().enumerate() {
                self.colours[row * nx..][..nx].copy_from_slice(&r.colours[k * nx..][..nx]);
                self.keys[row * nx..][..nx].copy_from_slice(&r.keys[k * nx..][..nx]);
                self.cells[row * nx..][..nx].copy_from_slice(&r.cells[k * nx..][..nx]);
                if !self.filled[row] {
                    self.filled[row] = true;
                    self.rows_done += 1;
                    if self.rows_done == spec.ny {
                        let s = self.started.elapsed().as_secs_f64();
                        self.row_seconds = Some(s / spec.ny as f64);
                        log::debug!("slice {}×{} traced in {:.1} ms", spec.nx, spec.ny, 1e3 * s);
                    }
                }
            }
            for (l, p) in r.problems {
                if self.seen.len() < 500 && !self.seen.iter().any(|(s, _)| *s == l) {
                    self.seen.push((l, p));
                }
            }
            self.image_dirty = true;
        }
    }

    /// Whether the trace in progress may give way to a new view: once it is
    /// done, or after a while, so that a view that keeps moving still shows
    /// whole (quarter-resolution) images.
    fn may_restart(&self) -> bool {
        self.spec.as_ref().is_none_or(|s| self.rows_done == s.ny)
            || self.started.elapsed().as_secs_f64() > 0.1
    }

    /// The trace so far. A row not traced yet takes the nearest traced row
    /// at most 3 above it (filling the coarse pass's gaps); further from
    /// any, it is transparent and the last complete image shows through.
    fn image(&self, spec: &Spec) -> egui::ColorImage {
        let (nx, ny) = (spec.nx, spec.ny);
        let mut source = vec![None; ny];
        let mut last: Option<usize> = None;
        for j in 0..ny {
            if self.filled[j] {
                last = Some(j);
            }
            source[j] = last.filter(|&l| j - l <= 3);
        }
        // Fields apart: the closures run on the rayon pool.
        let (keys, cells, colours) = (&self.keys, &self.cells, &self.colours);
        // What an outline is between: labels (cells), or the materials or
        // universes of the deepest cells (but at a level shown, where the
        // cells drawn are not the deepest: cells then). Where there is no
        // cell, the label with a bit of its own, apart from any class.
        let classes = self
            .classes
            .as_ref()
            .filter(|c| c.0 == self.outline_by && spec.level.is_none())
            .map(|c| &*c.2);
        let outline = self.outline;
        let line = self.outline_colour.unwrap_or(OUTLINE);
        let class = |k: usize| match classes {
            Some(t) if cells[k] != tracer::NONE => t[cells[k] as usize],
            Some(_) => keys[k] | 1 << 63,
            None => keys[k],
        };
        let (selection, hovered) = (&self.selection, self.hovered);
        let sel = |c: u32| c != tracer::NONE && selection.as_ref().is_some_and(|s| s.contains(c));
        let hov = |c: u32| c != tracer::NONE && c == hovered;
        let cell_at = |i: usize, j: usize| source[j].map(|sj| cells[sj * nx + i]);
        let [sel_rgb, hov_rgb] = self.highlight;
        // Rows apart on the UI's own pool (the global one is busy tracing):
        // a full image is a million pixels and more, composed again
        // whenever rows land or the hovered cell changes, and the frame
        // waits for it.
        let mut pixels = vec![egui::Color32::TRANSPARENT; nx * ny];
        let compose = |(j, row): (usize, &mut [egui::Color32])| {
            let Some(sj) = source[j] else {
                return;
            };
            let below = source.get(j + 1).copied().flatten();
            for (i, px) in row.iter_mut().enumerate() {
                let k = class(sj * nx + i);
                let edge = outline
                    && ((i + 1 < nx && class(sj * nx + i + 1) != k)
                        || below.is_some_and(|b| b != sj && class(b * nx + i) != k));
                let cell = cells[sj * nx + i];
                // The selection's and the hovered cell's boundaries, on
                // their inner side.
                let differs = |f: &(dyn Fn(u32) -> bool + Sync)| {
                    let me = f(cell);
                    me && [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(di, dj)| {
                        let (x, y) = (i as isize + di, j as isize + dj);
                        x >= 0
                            && y >= 0
                            && (x as usize) < nx
                            && (y as usize) < ny
                            && cell_at(x as usize, y as usize).is_some_and(|c| !f(c))
                    })
                };
                let [r, g, b] = if differs(&hov) {
                    hov_rgb
                } else if differs(&sel) {
                    sel_rgb
                } else if edge {
                    line
                } else if sel(cell) {
                    let c = colours[sj * nx + i];
                    [0, 1, 2].map(|q| (0.65 * c[q] as f32 + 0.35 * sel_rgb[q] as f32) as u8)
                } else {
                    colours[sj * nx + i]
                };
                *px = egui::Color32::from_rgb(r, g, b);
            }
        };
        crate::tasks::ui_pool().install(|| pixels.par_chunks_mut(nx).enumerate().for_each(compose));
        egui::ColorImage::new([nx, ny], pixels)
    }

    /// Make what outlines tell apart per cell, for the model and choice of
    /// now, if not made yet.
    fn refresh_classes(&mut self) {
        let made = self
            .classes
            .as_ref()
            .is_some_and(|c| c.0 == self.outline_by && Arc::ptr_eq(&c.1, &self.model));
        if self.outline_by == OutlineBy::Cell || made {
            return;
        }
        let m = &self.model;
        let t = (0..m.cells().len() as u32)
            .map(|i| {
                let c = m.cell(i);
                match self.outline_by {
                    OutlineBy::Material => c.material as u64,
                    _ => m.units()[c.unit as usize].universe as u64,
                }
            })
            .collect();
        self.classes = Some((self.outline_by, self.model.clone(), Arc::new(t)));
    }

    /// Where an image of view `s` lies on screen when `rect` shows view
    /// `cur`; `None` for another plane.
    fn layer_rect(cur: &Spec, s: &Spec, rect: egui::Rect) -> Option<egui::Rect> {
        if s.frame != cur.frame {
            return None;
        }
        let (ch, sh) = (cur.height, s.height);
        let x = |a: f64| {
            rect.min.x + ((a - (cur.centre[0] - 0.5 * cur.width)) / cur.width) as f32 * rect.width()
        };
        let y =
            |b: f64| rect.min.y + (((cur.centre[1] + 0.5 * ch) - b) / ch) as f32 * rect.height();
        Some(egui::Rect::from_min_max(
            egui::pos2(x(s.centre[0] - 0.5 * s.width), y(s.centre[1] + 0.5 * sh)),
            egui::pos2(x(s.centre[0] + 0.5 * s.width), y(s.centre[1] - 0.5 * sh)),
        ))
    }

    /// Show the slice in the rest of `ui`; any drag pans it.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> SliceEvents {
        self.ui_with(ui, true)
    }

    /// [`SliceView::ui`], the left button's drag panning only if
    /// `primary_pans` (right and middle drags always do).
    pub fn ui_with(&mut self, ui: &mut egui::Ui, primary_pans: bool) -> SliceEvents {
        let size = ui.available_size().max(egui::vec2(16.0, 16.0));
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "slice view"));
        let ppp = ui.ctx().pixels_per_point();
        let nx = ((rect.width() * ppp) as usize).clamp(8, 4096);
        let ny = ((rect.height() * ppp) as usize).clamp(8, 4096);

        // Pan and zoom.
        let per_point = self.width / rect.width() as f64;
        let pans = (primary_pans && response.dragged_by(egui::PointerButton::Primary))
            || response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle);
        if pans {
            let d = response.drag_delta();
            self.centre[0] -= d.x as f64 * per_point;
            self.centre[1] += d.y as f64 * per_point;
        }
        if response.hovered() && !ui.ctx().egui_wants_keyboard_input() {
            // PageUp / PageDown: the next plane, a hundredth of the view
            // along the axis or a degree about a radial one (Shift: 10×).
            let (up, down, shift) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::PageUp),
                    i.key_pressed(egui::Key::PageDown),
                    i.modifiers.shift,
                )
            });
            let steps = (up as i32 - down as i32) as f64 * if shift { 10.0 } else { 1.0 };
            if steps != 0.0 {
                match &mut self.radial {
                    Some(r) => r.phi = (r.phi + steps + 180.0).rem_euclid(360.0) - 180.0,
                    None => self.at += steps * 0.01 * self.width,
                }
            }
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y) as f64;
            if scroll != 0.0 {
                let f = (-scroll * 0.002).exp();
                if let Some(pos) = response.hover_pos() {
                    // Keep the point under the cursor in place.
                    let ox = (pos.x - rect.center().x) as f64 * per_point;
                    let oy = (rect.center().y - pos.y) as f64 * per_point;
                    self.centre[0] += ox * (1.0 - f);
                    self.centre[1] += oy * (1.0 - f);
                }
                self.width = (self.width * f).max(1e-6);
            }
        }

        self.refresh_classes();
        // The view on screen, and the one to trace: while the view moves,
        // every column and as many rows as fit `MOVING_SECONDS` (so that
        // whole images come often), all of them once it has been still
        // for 0.2 s.
        self.aspect = ny as f64 / nx as f64;
        let spec = Spec {
            frame: self.frame(),
            centre: self.centre,
            width: self.width,
            height: self.width * ny as f64 / nx as f64,
            nx,
            ny,
            by: self.by,
            paint: self.paint_generation,
            level: self.level,
            scale: [1, 1],
        };
        if self.last_view.as_ref() != Some(&spec) {
            if self.last_view.is_some() {
                self.changed = Instant::now();
            }
            self.last_view = Some(spec.clone());
        }
        let moving = self.changed.elapsed().as_secs_f64() < 0.2;
        let mut wanted = spec.clone();
        let k = if moving {
            *self
                .moving_k
                .get_or_insert_with(|| moving_rows(self.row_seconds, ny))
        } else {
            self.moving_k = None;
            1
        };
        if k > 1 {
            wanted.ny = (ny / k).max(8);
            wanted.scale = [1, k];
        }
        if self.spec.as_ref() != Some(&wanted) && self.may_restart() {
            self.start(wanted.clone(), ui.ctx());
        }
        self.receive();
        let traced = self.spec.clone().expect("started");
        let complete = self.rows_done == traced.ny;
        let outline = (self.outline, self.outline_by, self.outline_colour);
        if self.composed_outline != outline {
            self.composed_outline = outline;
            self.image_dirty = true;
        }
        if self.image_dirty {
            let img = self.image(&traced);
            if complete {
                // The new complete image replaces the last one.
                let texture = ui
                    .ctx()
                    .load_texture("slice", img, egui::TextureOptions::NEAREST);
                self.shown = Some(Layer {
                    texture,
                    spec: traced.clone(),
                });
                self.partial = None;
                self.previous = None;
            } else {
                match &mut self.partial {
                    Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                    None => {
                        self.partial = Some(ui.ctx().load_texture(
                            "slice (tracing)",
                            img,
                            egui::TextureOptions::NEAREST,
                        ))
                    }
                }
            }
            self.image_dirty = false;
        }
        self.update_labels(&traced, complete && traced == spec, ui.ctx());

        // The layers, moved to the current view.
        let painter = ui.painter_at(rect);
        let bg = self.background;
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(bg[0], bg[1], bg[2]));
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        for l in [&self.shown, &self.previous].into_iter().flatten() {
            if let Some(r) = Self::layer_rect(&spec, &l.spec, rect) {
                painter.image(l.texture.id(), r, uv, egui::Color32::WHITE);
            }
        }
        if let (Some(t), false) = (&self.partial, complete) {
            if let Some(r) = Self::layer_rect(&spec, &traced, rect) {
                painter.image(t.id(), r, uv, egui::Color32::WHITE);
            }
        }
        // The grid under the numbers.
        if self.grid {
            grid::draw(&painter, rect, &self.plane(), self.axis_names());
        }
        if let Some((ls, placed)) = &self.labels {
            if let Some(r) = Self::layer_rect(&spec, ls, rect) {
                let [r0, g0, b0] = self.highlight[0];
                labels::draw(
                    &painter,
                    r,
                    ls.nx,
                    ls.ny,
                    placed,
                    egui::Color32::from_rgb(r0, g0, b0),
                );
            }
        }

        let plane = self.plane();
        // Repaint while tracing, or while a new view waits for its turn.
        if !complete || traced != spec || moving {
            ui.ctx().request_repaint();
        }
        let hovered_cell = response.hover_pos().and_then(|p| {
            let i = ((p.x - rect.min.x) / rect.width() * traced.nx as f32) as usize;
            let j = ((p.y - rect.min.y) / rect.height() * traced.ny as f32) as usize;
            // Only over an image of this very view (perhaps coarser).
            let same = traced.frame == spec.frame
                && traced.centre == spec.centre
                && traced.width == spec.width;
            let c = *self
                .cells
                .get(j.min(traced.ny - 1) * traced.nx + i.min(traced.nx - 1))?;
            (same && c != tracer::NONE).then_some(c)
        });
        SliceEvents {
            hovered: response.hover_pos().map(|p| plane.from_screen(rect, p)),
            hovered_cell,
            clicked: response
                .clicked()
                .then(|| response.interact_pointer_pos())
                .flatten()
                .map(|p| plane.from_screen(rect, p)),
            response,
            plane,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plane_view_maps_both_ways() {
        let plane = PlaneView {
            frame: Frame::axis(AxisName::Y, 5.0),
            centre: [10.0, -20.0],
            width: 200.0,
            height: 150.0,
        };
        let rect = egui::Rect::from_min_size(egui::pos2(50.0, 30.0), egui::vec2(400.0, 300.0));
        // The centre of the view, and x (right) and z (up) on a y plane.
        assert_eq!(
            plane.to_screen(rect, [10.0, 7.0, -20.0]),
            (rect.center(), 2.0)
        );
        let (p, _) = plane.to_screen(rect, [60.0, 5.0, 30.0]);
        assert_eq!(p, rect.center() + egui::vec2(100.0, -100.0));
        assert_eq!(plane.from_screen(rect, p), [60.0, 5.0, 30.0]);
        assert_eq!(plane.cm_per_point(rect), 0.5);
    }

    fn spec(centre: [f64; 2], width: f64) -> Spec {
        Spec {
            frame: Frame::axis(AxisName::Z, 0.0),
            centre,
            width,
            height: width / 2.0,
            nx: 200,
            ny: 100,
            by: ColourBy::Cell,
            paint: 0,
            level: None,
            scale: [1, 1],
        }
    }

    #[test]
    fn rows_while_moving_fit_the_time() {
        assert_eq!(moving_rows(None, 1000), 4);
        // 1000 rows of a whole reference model take ~70 ms on 16 threads.
        assert_eq!(moving_rows(Some(7e-5), 1000), 4);
        assert_eq!(moving_rows(Some(5e-5), 1000), 2);
        // A close-up: all of them.
        assert_eq!(moving_rows(Some(2e-5), 1000), 1);
        // Too slow even at a quarter: a quarter still.
        assert_eq!(moving_rows(Some(1e-3), 1000), 4);
    }

    #[test]
    fn layers_follow_pan_and_zoom() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(200.0, 100.0));
        let cur = spec([0.0, 0.0], 100.0);
        // The same view fills the rect.
        let r = SliceView::layer_rect(&cur, &cur, rect).unwrap();
        assert!((r.min - rect.min).length() < 1e-3 && (r.max - rect.max).length() < 1e-3);
        // An image of the view 25 cm to the right sits a quarter width left.
        let r = SliceView::layer_rect(&spec([0.0, 0.0], 100.0), &spec([-25.0, 0.0], 100.0), rect)
            .unwrap();
        assert!((r.min.x - (10.0 - 50.0)).abs() < 1e-3);
        // Zoomed in 2× about the centre: the old image is twice as large.
        let r = SliceView::layer_rect(&spec([0.0, 0.0], 50.0), &cur, rect).unwrap();
        assert!((r.width() - 400.0).abs() < 1e-3 && (r.center() - rect.center()).length() < 1e-3);
        // Another axis is not drawn.
        let mut other = cur.clone();
        other.frame = Frame::axis(AxisName::X, 0.0);
        assert!(SliceView::layer_rect(&cur, &other, rect).is_none());
    }

    #[test]
    fn radial_frames_hold_the_axis() {
        let r = Radial {
            point: [1.0, 2.0, 0.0],
            axis: [0.0, 0.0, 1.0],
            zero: [1.0, 0.0, 0.0],
            phi: 90.0,
        };
        let f = Frame::radial(&r, 0.0);
        // Out along +y, up along z, through the axis point.
        assert!((f.u[1] - 1.0).abs() < 1e-12 && f.v == [0.0, 0.0, 1.0]);
        let (c, off) = f.coords([1.0, 5.0, 7.0]);
        assert!((c[0] - 3.0).abs() < 1e-12 && (c[1] - 7.0).abs() < 1e-12 && off.abs() < 1e-12);
        let p = f.point([3.0, 7.0]);
        assert!((p[1] - 5.0).abs() < 1e-12);
    }
}
