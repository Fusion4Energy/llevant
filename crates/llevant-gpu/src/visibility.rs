//! Which cells are visible, packed for the shader: a bit per cell, the
//! skip boxes and octree bits, ghosts and the level shown.

use super::*;
use std::sync::{Arc, PoisonError};

use crate::pack::{push_box, push_f32};

/// A visibility on the GPU. Each [`GpuModel::set_visibility`] uploads a new
/// one rather than writing over the buffer: a job binds the one current when
/// it is made and keeps reading it over all its slices, however many
/// visibilities are set meanwhile (each is a few MB at most, freed with the
/// last job that reads it).
pub(crate) struct Vis {
    pub(crate) buffer: wgpu::Buffer,
    /// Whether it has ghosts (images then write ghost records where their
    /// hit buffer has room).
    pub(crate) ghosts: bool,
}

/// A visibility uploaded for a [`GpuModel`] ([`GpuModel::prepare_visibility`]),
/// not yet the one it traces with: preparing it takes the work (the skip's
/// words, the upload), switching to it ([`GpuModel::use_visibility`]) none,
/// so a caller can prepare on a thread of its own.
#[derive(Clone)]
pub struct GpuVisibility(Arc<Vis>);

impl GpuVisibility {
    /// Whether it has ghosts.
    pub fn has_ghosts(&self) -> bool {
        self.0.ghosts
    }
}

/// Where things are in the visibility buffer, in words: a bit per cell,
/// then per unit the box of its cells worth entering (6 f32, rounded
/// outward), then a bit per octree node ([`llevant::export::ExportSkip`]),
/// then a bit per ghost cell and three words: the level shown ([`NONE`] for
/// none), whether any cell is a ghost and whether gaps are veils.
pub(crate) struct VisLayout {
    pub(crate) boxes: usize,
    pub(crate) nodes: usize,
    pub(crate) ghosts: usize,
    pub(crate) flags: usize,
    len: usize,
}

impl VisLayout {
    pub(crate) fn new(cells: usize, units: usize, grid_nodes: usize) -> VisLayout {
        let boxes = cells.div_ceil(32).max(1);
        let nodes = boxes + 6 * units;
        let ghosts = nodes + grid_nodes.div_ceil(32).max(1);
        let flags = ghosts + cells.div_ceil(32).max(1);
        VisLayout {
            boxes,
            nodes,
            ghosts,
            flags,
            len: flags + 3,
        }
    }

    /// Everything visible, nothing skipped, no level, no ghosts.
    pub(crate) fn everything(&self) -> Vec<u32> {
        let mut w = vec![u32::MAX; self.len];
        for u in 0..(self.nodes - self.boxes) / 6 {
            let at = self.boxes + 6 * u;
            let mut b = Vec::with_capacity(6);
            push_f32(&mut b, &[-1e30, -1e30, -1e30, 1e30, 1e30, 1e30]);
            w[at..at + 6].copy_from_slice(&b);
        }
        w[self.ghosts..self.flags].fill(0);
        w[self.flags + 1] = 0;
        w[self.flags + 2] = 0;
        w
    }
}

impl GpuModel {
    /// Which cells are visible (all are until this is called), in `m`, the
    /// model this was uploaded from. A hidden filled cell hides what it
    /// holds. Rays jump over what holds nothing visible ([`Model::skip`]),
    /// so they see gaps and problems only elsewhere; where they step, gaps
    /// and problems always stop them, but gaps are seen through with
    /// [`Visibility::veil`] (as the CPU does; overlaps are not looked for),
    /// said by a bit of each hit ([`Hit::veils`]). [`Error::Invalid`] unless `vis` has
    /// one flag per cell of a model of this size.
    pub fn set_visibility(&self, gpu: &Gpu, m: &Model, vis: &Visibility) -> Result<()> {
        self.set_visibility_with(gpu, m, vis, &m.skip(vis)?)
    }

    /// [`GpuModel::set_visibility`] with `skip`, [`Model::skip`] of `vis`
    /// (for a caller that needs it too).
    pub fn set_visibility_with(
        &self,
        gpu: &Gpu,
        m: &Model,
        vis: &Visibility,
        skip: &Skip,
    ) -> Result<()> {
        let v = self.prepare_visibility(gpu, m, vis, skip)?;
        self.use_visibility(&v);
        Ok(())
    }

    /// Upload `vis` (whose skip is `skip`) without tracing with it yet:
    /// errors as [`GpuModel::set_visibility`]. Safe on any thread.
    pub fn prepare_visibility(
        &self,
        gpu: &Gpu,
        m: &Model,
        vis: &Visibility,
        skip: &Skip,
    ) -> Result<GpuVisibility> {
        let words = self.visibility_words(m, vis, skip)?;
        let buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("visibility"),
                contents: bytemuck::cast_slice(&words),
                usage: wgpu::BufferUsages::STORAGE,
            });
        Ok(GpuVisibility(Arc::new(Vis {
            buffer,
            ghosts: vis.has_ghosts(),
        })))
    }

    /// Trace with `v` (prepared for this model) from now on; jobs made
    /// before keep theirs.
    pub fn use_visibility(&self, v: &GpuVisibility) {
        *self.vis.lock().unwrap_or_else(PoisonError::into_inner) = v.0.clone();
    }

    /// The current visibility, for a job to bind.
    pub(crate) fn current_vis(&self) -> Arc<Vis> {
        self.vis
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// One bit per cell, then what [`Model::skip`] finds, the ghosts and
    /// the level ([`VisLayout`]).
    fn visibility_words(&self, m: &Model, vis: &Visibility, skip: &Skip) -> Result<Vec<u32>> {
        if !vis.ghosts.is_empty() && vis.ghosts.len() != vis.cells.len() {
            return Err(Error::Invalid("ghost flags not one per cell".into()));
        }
        if vis.cells.len() != self.n_cells || m.cells().len() != self.n_cells {
            return Err(Error::Invalid(format!(
                "visibility has {} cell flags and its model {} cells, for a model of {} cells",
                vis.cells.len(),
                m.cells().len(),
                self.n_cells
            )));
        }
        let skip = m.export_skip(skip);
        let layout = VisLayout::new(self.n_cells, self.n_units, self.n_grid_nodes);
        if skip.unit_boxes.len() != self.n_units || 32 * skip.node_bits.len() < self.n_grid_nodes {
            return Err(Error::Invalid(
                "visibility of another model than the one uploaded".into(),
            ));
        }
        let mut words = vec![0u32; layout.len];
        for (i, _) in vis.cells.iter().enumerate().filter(|(_, &v)| v) {
            words[i / 32] |= 1 << (i % 32);
        }
        for (u, b) in skip.unit_boxes.iter().enumerate() {
            let at = layout.boxes + 6 * u;
            let mut w = Vec::with_capacity(6);
            if b.is_empty() {
                // lo > hi: nothing inside.
                push_f32(&mut w, &[1e30, 1e30, 1e30, -1e30, -1e30, -1e30]);
            } else {
                push_box(&mut w, b);
            }
            words[at..at + 6].copy_from_slice(&w);
        }
        words[layout.nodes..layout.nodes + skip.node_bits.len()].copy_from_slice(&skip.node_bits);
        // A hidden ghost hides as any cell.
        for (i, _) in vis
            .ghosts
            .iter()
            .zip(&vis.cells)
            .enumerate()
            .filter(|(_, (&g, &v))| g && v)
        {
            words[layout.ghosts + i / 32] |= 1 << (i % 32);
        }
        // Levels below MAX_LEVELS: deeper ones are never reached.
        words[layout.flags] = vis
            .level
            .and_then(|l| u32::try_from(l).ok())
            .filter(|&l| l < MAX_LEVELS)
            .unwrap_or(NONE);
        words[layout.flags + 1] = vis.has_ghosts() as u32;
        words[layout.flags + 2] = vis.veil as u32;
        Ok(words)
    }
}
