//! A draft's changes written into an uploaded model in place
//! ([`GpuModel::patch`]): no new upload, no new shader.

use super::*;
use crate::pack::{
    push_box, recentre, BVH_ITEM_WORDS, BVH_NODE_WORDS, CELL_WORDS, SLACK_WORDS, UNIT_WORDS,
};
use llevant::draft::{DraftPatch, PatchAlways};
use llevant::export::{SHAPE_CONE, SHAPE_QUADRIC};

impl GpuModel {
    /// Write `p`, a step of a [`llevant::draft::Draft`] of the model this was
    /// uploaded from, into the model on the GPU: shapes (recentred as at
    /// upload), cell, node, BVH and unit boxes, units' octree roots and
    /// lists of cells checked everywhere (in a slack area reserved at
    /// upload), octree leaf programs. Written through the queue: jobs made
    /// after see it; a job already under way would see it from its next
    /// slice, so patch between jobs. [`Error::Invalid`] if the units' lists
    /// do not fit the slack, nothing written then.
    pub fn patch(&self, gpu: &Gpu, p: &DraftPatch) -> Result<()> {
        let mut writes: Vec<(u32, Vec<u32>)> = Vec::new();
        let mut slack_at = self.slack;
        for u in &p.units {
            if let PatchAlways::Own(list) = &u.always {
                if (slack_at - self.slack) as usize + list.len() > SLACK_WORDS {
                    return Err(Error::Invalid(format!(
                        "a draft's lists of cells checked everywhere need over {SLACK_WORDS} words"
                    )));
                }
                slack_at += list.len() as u32;
            }
        }
        for s in &p.shapes {
            let mut data = s.data.clone();
            if (s.kind == SHAPE_QUADRIC || s.kind == SHAPE_CONE) && !s.usage.is_empty() {
                recentre(&mut data[..13], &s.usage);
            }
            let words: Vec<u32> = data.iter().map(|&x| (x as f32).to_bits()).collect();
            writes.push((self.sections[0] + s.start, words));
        }
        for &(c, b) in &p.cells {
            let mut w = Vec::with_capacity(6);
            push_box(&mut w, &b);
            writes.push((self.offsets[1] + CELL_WORDS as u32 * c, w));
        }
        for &(i, [t, f]) in &p.node_boxes {
            let mut w = Vec::with_capacity(12);
            push_box(&mut w, &t);
            push_box(&mut w, &f);
            writes.push((self.sections[1] + 12 * i, w));
        }
        for &(i, b) in &p.bvh_nodes {
            let mut w = Vec::with_capacity(6);
            push_box(&mut w, &b);
            writes.push((self.offsets[3] + BVH_NODE_WORDS as u32 * i, w));
        }
        for &(i, b) in &p.bvh_items {
            let mut w = Vec::with_capacity(6);
            push_box(&mut w, &b);
            writes.push((self.offsets[4] + BVH_ITEM_WORDS as u32 * i + 1, w));
        }
        let mut slack_at = self.slack;
        for u in &p.units {
            let at = self.offsets[2] + UNIT_WORDS as u32 * u.unit;
            let mut w = Vec::with_capacity(6);
            push_box(&mut w, &u.roi);
            writes.push((at, w));
            let (start, len) = match &u.always {
                PatchAlways::Export(span) => (self.always_list + span.start, span.len),
                PatchAlways::Own(list) => {
                    let start = slack_at;
                    writes.push((start, list.clone()));
                    slack_at += list.len() as u32;
                    (start, list.len() as u32)
                }
            };
            writes.push((at + 14, vec![start, len]));
            writes.push((at + 17, vec![u.grid_root]));
        }
        // Octree leaf programs: `grid_progs` is relative to the leaf's
        // first item as the shader sees it (past the nodes).
        let progs = self.grid_progs + 2 * self.n_grid_nodes as u32;
        for &(k, v) in &p.grid_progs {
            writes.push((progs + k, vec![v]));
        }
        self.write_all(gpu, writes);
        Ok(())
    }

    /// Write every `(pool index, words)` through one staging buffer: runs
    /// that touch merged, then one copy per run (per pool buffer it
    /// crosses), all in one submission. Many small queue writes cost
    /// ~2 µs each; a reference-model drag writes tens of thousands.
    fn write_all(&self, gpu: &Gpu, mut writes: Vec<(u32, Vec<u32>)>) {
        writes.sort_by_key(|w| w.0);
        let mut staged: Vec<u32> = Vec::new();
        // (pool index, start in `staged`, len)
        let mut runs: Vec<(usize, usize, usize)> = Vec::new();
        for (at, words) in writes {
            let at = at as usize;
            match runs.last_mut() {
                Some(r) if r.0 + r.2 == at => {
                    r.2 += words.len();
                }
                _ => runs.push((at, staged.len(), words.len())),
            }
            staged.extend_from_slice(&words);
        }
        if staged.is_empty() {
            return;
        }
        let src = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("draft patch"),
                contents: bytemuck::cast_slice(&staged),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("draft patch"),
            });
        for (at, from, len) in runs {
            let mut done = 0;
            while done < len {
                let i = at + done;
                let (chunk, off) = (i >> self.shift, i & ((1 << self.shift) - 1));
                let n = ((1 << self.shift) - off).min(len - done);
                enc.copy_buffer_to_buffer(
                    &src,
                    4 * (from + done) as u64,
                    &self.pool[chunk],
                    4 * off as u64,
                    4 * n as u64,
                );
                done += n;
            }
        }
        gpu.queue.submit([enc.finish()]);
    }
}
