//! Tracing work on the GPU: the shader's parameters, jobs traced a slice
//! at a time, explicit rays and camera images.

use super::*;
use crate::device::{output_buffer, read_buffer, read_mapped};
use crate::hits::{decode, decode_all, decode_ghosts};
use crate::visibility::Vis;

/// Uniform block, as `Params` in the shader.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    offsets: [u32; 9],
    n_rays: u32,
    max_steps: u32,
    /// Camera mode: regions of `cut` (each its planes up to `cut_ends`).
    cut_regions: u32,
    eps_abs: f32,
    eps_rel: f32,
    width: u32,
    height: u32,
    sections: [u32; 4],
    /// Rays in flight at most (a multiple of 64; see [`Job`]).
    capacity: u32,
    /// Program words a ray walks in one slice before it pauses.
    slice_work: u32,
    /// Camera mode: blocks of 8 × 8 pixels in the job.
    n_blocks: u32,
    tail_work: u32,
    max_work: u32,
    /// Where the octree candidates' programs (`Export::grid_progs`) start,
    /// less the octree section's item offset: entry `grid_progs + first + i`
    /// for candidate `first + i`.
    grid_progs: u32,
    /// Where the ghost records go in the hits ([`NONE`]: not written).
    ghost_out: u32,
    /// Where the unit boxes, octree-node bits, ghost bits and flags start
    /// in the visibility buffer (`VisLayout`).
    vis: [u32; 4],
    /// The shader aligns the vectors that follow to 16 bytes.
    _pad: u32,
    eye: [f32; 4],
    fwd: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    /// Planes of the camera's cut, region after region: unit normal (into
    /// the space removed), w: the eye's signed distance from the plane.
    cut: [[f32; 4]; MAX_CUT_PLANES],
    /// Where each region's planes end in `cut`.
    cut_ends: [u32; MAX_CUT_REGIONS],
}

/// Control words of a [`Job`] before its blocks, as `C_*` in the shader.
const CTL_WORDS: usize = 12;

/// A workgroup's block starts in the next slice (as in the shader).
const FRESH: u32 = 0x8000_0000;

impl GpuModel {
    fn params(&self, n_rays: u32) -> Params {
        Params {
            offsets: self.offsets,
            n_rays,
            max_steps: self.max_steps,
            cut_regions: 0,
            eps_abs: self.eps_abs,
            eps_rel: self.eps_rel,
            width: 0,
            height: 0,
            sections: self.sections,
            capacity: 0,
            slice_work: u32::MAX,
            n_blocks: 0,
            tail_work: u32::MAX,
            max_work: self.max_work,
            grid_progs: self.grid_progs,
            ghost_out: NONE,
            vis: self.vis_offsets,
            _pad: 0,
            eye: [0.0; 4],
            fwd: [0.0; 4],
            right: [0.0; 4],
            up: [0.0; 4],
            cut: [[0.0; 4]; MAX_CUT_PLANES],
            cut_ends: [0; MAX_CUT_REGIONS],
        }
    }

    /// A job of `total` rays through `pipeline` with `params`, writing into
    /// `hits` and seeing `vis`; `blocks` (camera mode) the 8 × 8 blocks of
    /// its image in the order traced. `sliced`: rays pause after
    /// [`GpuModel::slice_work`] words, [`GpuModel::capacity`] at most in
    /// flight; else one slice traces them all to the end.
    #[allow(clippy::too_many_arguments)]
    fn job(
        &self,
        gpu: &Gpu,
        mut params: Params,
        rays: &wgpu::Buffer,
        hits: &wgpu::Buffer,
        vis: &wgpu::Buffer,
        pipeline: &wgpu::ComputePipeline,
        total: u32,
        blocks: &[[u32; 2]],
        sliced: bool,
    ) -> Job {
        let device = &gpu.device;
        let all = total.next_multiple_of(64).max(64);
        let capacity = if sliced {
            self.capacity.next_multiple_of(64).clamp(64, all)
        } else {
            all
        };
        params.capacity = capacity;
        params.slice_work = if sliced {
            self.slice_work.max(1)
        } else {
            u32::MAX
        };
        params.tail_work = self.tail_work;
        params.n_blocks = blocks.len() as u32;
        // Workgroups of 64 rays, each a block at a time: the first ones
        // start with the first blocks.
        let groups = capacity as usize / 64;
        let first = groups.min(total.div_ceil(64) as usize);
        let lists = CTL_WORDS + 2 * blocks.len();
        let mut ctl = vec![0u32; lists + 3 * groups];
        // The indirect dispatch's y and z; `plan` takes list 0 first.
        ctl[1] = 1;
        ctl[2] = 1;
        ctl[3] = first as u32;
        ctl[5] = 1;
        ctl[6] = first as u32;
        ctl[7] = total;
        // Rays left per block: all 64 of its rays end through
        // `finish_camera_ray`, those past the image's edge too.
        for (k, &[bx, by]) in blocks.iter().enumerate() {
            ctl[CTL_WORDS + k] = 64;
            ctl[CTL_WORDS + blocks.len() + k] = bx | (by << 16);
        }
        for g in 0..first {
            ctl[lists + g] = g as u32;
            ctl[lists + 2 * groups + g] = g as u32 | FRESH;
        }
        let ctl = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("job control"),
            contents: bytemuck::cast_slice(&ctl),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let args = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("job dispatch"),
            size: 12,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Unsliced jobs pause nothing.
        let states_words = if sliced {
            capacity as u64 * STATE_WORDS as u64
        } else {
            STATE_WORDS as u64
        };
        let states = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("paused rays"),
            size: 4 * states_words,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut buffers = vec![&uniform, vis, rays, hits, &ctl, &states];
        buffers.extend(&self.pool);
        buffers.extend(&self.spare);
        let entries: Vec<wgpu::BindGroupEntry> = buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.as_entire_binding(),
            })
            .collect();
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llevant"),
            layout: &self.layout,
            entries: &entries,
        });
        Job {
            bind,
            pipeline: pipeline.clone(),
            plan: self.plan.clone(),
            ctl,
            args,
            n_blocks: blocks.len(),
            rays: total,
        }
    }

    /// Run `job` to its end, a few slices per submission; the wall time
    /// from the first submission to the end. [`Error::Gpu`] when the
    /// device fails.
    fn run_job(&self, gpu: &Gpu, job: &Job) -> Result<f64> {
        let device = &gpu.device;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("job progress"),
            size: 4 * CTL_WORDS as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let start = Instant::now();
        let mut slices = 4;
        loop {
            let mut enc = device.create_command_encoder(&Default::default());
            job.encode_slices(&mut enc, slices);
            enc.copy_buffer_to_buffer(&job.ctl, 0, &readback, 0, 4 * CTL_WORDS as u64);
            gpu.queue.submit([enc.finish()]);
            let words = read_mapped(gpu, &readback)?;
            if Progress::from_words(&words).done {
                return Ok(start.elapsed().as_secs_f64());
            }
            slices = (2 * slices).min(64);
        }
    }

    /// Trace `n` explicit rays (`data`: 8 floats each) through `pipeline`
    /// with `params`, seeing `vis`, into a buffer of `n_out` hits (and the
    /// ghost records, when `params` says where); its words read back and
    /// the time traced.
    #[allow(clippy::too_many_arguments)]
    fn run_rays(
        &self,
        gpu: &Gpu,
        params: Params,
        data: &[f32],
        n: usize,
        n_out: usize,
        pipeline: &wgpu::ComputePipeline,
        vis: &wgpu::Buffer,
        sliced: bool,
    ) -> Result<(Vec<u32>, f64)> {
        let device = &gpu.device;
        let rays = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("rays"),
            contents: if data.is_empty() {
                &[0u8; 32]
            } else {
                bytemuck::cast_slice(data)
            },
            usage: wgpu::BufferUsages::STORAGE,
        });
        let per_ray = HIT_WORDS
            + if params.ghost_out != NONE {
                GHOST_WORDS
            } else {
                0
            };
        let hits = output_buffer(gpu, (per_ray * n_out.max(1)) as u64);
        let job = self.job(
            gpu,
            params,
            &rays,
            &hits,
            vis,
            pipeline,
            n as u32,
            &[],
            sliced,
        );
        let seconds = self.run_job(gpu, &job)?;
        Ok((read_buffer(gpu, &hits)?, seconds))
    }

    /// First hits of explicit rays (for tests and picking-sized batches).
    pub fn trace_rays(&self, gpu: &Gpu, rays: &[Line]) -> Vec<Hit> {
        self.trace_rays_with(gpu, rays, self.max_steps)
            .expect("GPU dispatch")
    }

    /// First hits of explicit rays in one dispatch, seeing the model's
    /// visibility ([`GpuModel::set_visibility`]) and giving up after
    /// `max_steps` locate steps ([`Status::Steps`]). [`Error::Gpu`] when the
    /// device fails.
    pub fn trace_rays_with(&self, gpu: &Gpu, rays: &[Line], max_steps: u32) -> Result<Vec<Hit>> {
        Ok(self.trace_rays_ghosts(gpu, rays, max_steps)?.0)
    }

    /// [`GpuModel::trace_rays_with`], with each ray's ghost boundaries
    /// ([`llevant::Ghost`]; empty when the visibility has no ghosts).
    pub fn trace_rays_ghosts(
        &self,
        gpu: &Gpu,
        rays: &[Line],
        max_steps: u32,
    ) -> Result<(Vec<Hit>, Vec<llevant::Ghost>)> {
        let mut data = Vec::with_capacity(8 * rays.len());
        for r in rays {
            data.extend(r.origin.iter().map(|&x| x as f32));
            data.push(r.t_min as f32);
            data.extend(r.dir.iter().map(|&x| x as f32));
            data.push(r.t_max as f32);
        }
        let mut params = self.params(rays.len() as u32);
        params.max_steps = max_steps;
        let vis = self.current_vis();
        let ghosts = vis.ghosts;
        if ghosts {
            params.ghost_out = (HIT_WORDS * rays.len()) as u32;
        }
        let (words, _) = self.run_rays(
            gpu,
            params,
            &data,
            rays.len(),
            rays.len(),
            &self.trace_rays,
            &vis.buffer,
            true,
        )?;
        let ghosts = if ghosts {
            decode_ghosts(&words[HIT_WORDS * rays.len()..], rays.len())
        } else {
            Vec::new()
        };
        Ok((decode_all(&words, rays.len()), ghosts))
    }
    /// Whether the model's visibility ([`GpuModel::set_visibility`]) has
    /// ghosts.
    pub fn has_ghosts(&self) -> bool {
        self.current_vis().ghosts
    }

    /// The steps one ray takes (for debugging): per step, where it is
    /// located, the crossing before, the level located from, the depth and
    /// status found, the surface crossed and the cells of the first four
    /// levels.
    pub fn debug_steps(&self, gpu: &Gpu, ray: &Line, max: usize) -> (Hit, Vec<[u32; 8]>) {
        let mut data: Vec<f32> = ray.origin.iter().map(|&x| x as f32).collect();
        data.push(ray.t_min as f32);
        data.extend(ray.dir.iter().map(|&x| x as f32));
        data.push(ray.t_max as f32);
        let params = self.params(1);
        let debug = self
            .debug
            .get_or_init(|| self.variant(gpu, "trace_rays", &[("DEBUG_STEPS", 1.0)]));
        let (words, _) = self
            .run_rays(
                gpu,
                params,
                &data,
                1,
                1 + max,
                debug,
                &self.current_vis().buffer,
                false,
            )
            .expect("GPU dispatch");
        let steps = words[HIT_WORDS..]
            .as_chunks::<HIT_WORDS>()
            .0
            .iter()
            .copied()
            .take_while(|w| w[..8] != [0; 8])
            .map(|w| w[..8].try_into().expect("8 words"))
            .collect();
        (decode(&words[..HIT_WORDS]), steps)
    }

    /// One frame through `camera`, with the rays [`Camera::ray`] gives (a
    /// hit at the clip plane takes the clip normal, as in
    /// [`llevant::Model::render`]). [`Error::Invalid`] for an image larger
    /// than 65535 pixels on a side.
    pub fn render(&self, gpu: &Gpu, camera: &Camera) -> Result<Frame> {
        self.render_with(gpu, camera, &self.render)
    }

    /// One frame with work counters in place of the normals: program words
    /// walked for exits, words evaluated while locating, candidate cells;
    /// and in place of the instance, the most words one locate step walked.
    #[doc(hidden)]
    pub fn render_stats(&self, gpu: &Gpu, camera: &Camera) -> Result<Frame> {
        let stats = self
            .stats
            .get_or_init(|| self.variant(gpu, "render", &[("STATS", 1.0)]));
        self.render_with(gpu, camera, stats)
    }

    /// A job tracing `blocks` of `camera`'s image (8 × 8 pixels each, in
    /// that order; [`tile_blocks`]) into `out`, sized as the image: the
    /// pixels of those blocks inside the image, each written when done.
    /// Rays give up ([`Status::Steps`]) after `max_steps` locate steps or
    /// `max_work` program words ([`GpuModel::max_work`] for the model's
    /// own bound; fewer bound the time of the slowest rays, for quick
    /// previews). Nothing runs until [`Job::encode_slices`].
    /// [`Error::Invalid`] if `out` is not the image's size or a block is
    /// outside the image.
    #[allow(clippy::too_many_arguments)]
    pub fn camera_job(
        &self,
        gpu: &Gpu,
        camera: &Camera,
        blocks: &[[u32; 2]],
        out: &HitBuffer,
        max_steps: u32,
        max_work: u32,
    ) -> Result<Job> {
        let vis = self.current_vis();
        self.camera_job_with(
            gpu,
            camera,
            blocks,
            out,
            max_steps,
            max_work,
            &self.render,
            &vis,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn camera_job_with(
        &self,
        gpu: &Gpu,
        camera: &Camera,
        blocks: &[[u32; 2]],
        out: &HitBuffer,
        max_steps: u32,
        max_work: u32,
        pipeline: &wgpu::ComputePipeline,
        vis: &Vis,
        sliced: bool,
    ) -> Result<Job> {
        let mut params = self.camera_params(camera)?;
        params.max_steps = max_steps;
        params.max_work = max_work;
        let (w, h) = (params.width, params.height);
        if (w, h) != (out.width, out.height) {
            return Err(Error::Invalid(format!(
                "hit buffer of {}×{} for an image of {w}×{h}",
                out.width, out.height
            )));
        }
        if let Some(b) = blocks.iter().find(|[x, y]| 8 * x >= w || 8 * y >= h) {
            return Err(Error::Invalid(format!(
                "block {b:?} outside an image of {w}×{h}"
            )));
        }
        let rays = u32::try_from(64 * blocks.len() as u64)
            .map_err(|_| Error::Invalid("too many blocks for one job".into()))?;
        if out.ghosts && vis.ghosts {
            params.ghost_out = out.ghost_offset();
        }
        Ok(self.job(
            gpu,
            params,
            &self.no_rays,
            &out.buffer,
            &vis.buffer,
            pipeline,
            rays,
            blocks,
            sliced,
        ))
    }

    /// Record the rendering of one `tile` of `camera`'s image into `enc`,
    /// writing its hits into `out` (sized as the image) at their pixels;
    /// nothing is submitted or waited for. Pixels of the 8 × 8 blocks the
    /// tile touches are written too; others of `out` are left as they are.
    /// One dispatch traces every ray to its end: [`GpuModel::camera_job`]
    /// bounds the time of each. [`Error::Invalid`] if `out` is not the
    /// image's size or the tile not inside it.
    pub fn encode_render(
        &self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        camera: &Camera,
        tile: Tile,
        out: &HitBuffer,
    ) -> Result<()> {
        self.encode_render_steps(gpu, enc, camera, tile, out, self.max_steps)
    }

    /// [`GpuModel::encode_render`] with rays giving up after `max_steps`
    /// locate steps ([`Status::Steps`]) instead of [`GpuModel::max_steps`]:
    /// fewer bound the time of the slowest rays, for quick previews.
    pub fn encode_render_steps(
        &self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        camera: &Camera,
        tile: Tile,
        out: &HitBuffer,
        max_steps: u32,
    ) -> Result<()> {
        let inside = |lo: u32, len: u32, n: usize| {
            lo.checked_add(len)
                .is_some_and(|hi| hi as usize <= n && n <= 65535)
        };
        if !inside(tile.x, tile.width, camera.width) || !inside(tile.y, tile.height, camera.height)
        {
            return Err(Error::Invalid(format!(
                "{tile:?} outside an image of {}×{}",
                camera.width, camera.height
            )));
        }
        let blocks = tile_blocks(tile);
        if blocks.is_empty() {
            return Ok(());
        }
        let vis = self.current_vis();
        let job = self.camera_job_with(
            gpu,
            camera,
            &blocks,
            out,
            max_steps,
            self.max_work,
            &self.render,
            &vis,
            false,
        )?;
        job.encode_slices(enc, 1);
        Ok(())
    }

    /// [`GpuModel::render`]'s tracing without reading its hits back: the
    /// seconds the frame took. For timing: frames timed one after another
    /// keep the GPU busy, where `render`'s readback leaves it idle between
    /// them, and a driver may then hold the GPU at a fraction of its clock
    /// (an RTX 3060 Laptop at ~780 of 2100 MHz).
    #[doc(hidden)]
    pub fn render_seconds(&self, gpu: &Gpu, camera: &Camera) -> Result<f64> {
        Ok(self.trace_frame(gpu, camera, &self.render)?.1)
    }

    /// Trace a whole frame through `camera` with `pipeline`: its hits on the
    /// GPU and the seconds it took.
    fn trace_frame(
        &self,
        gpu: &Gpu,
        camera: &Camera,
        pipeline: &wgpu::ComputePipeline,
    ) -> Result<(HitBuffer, f64)> {
        let params = self.camera_params(camera)?;
        let (width, height) = (params.width, params.height);
        // One visibility for the buffer's room and the job.
        let vis = self.current_vis();
        let out = HitBuffer::sized(gpu, width, height, vis.ghosts);
        let blocks = tile_blocks(Tile::full(width, height));
        let job = self.camera_job_with(
            gpu,
            camera,
            &blocks,
            &out,
            self.max_steps,
            self.max_work,
            pipeline,
            &vis,
            true,
        )?;
        let seconds = self.run_job(gpu, &job)?;
        Ok((out, seconds))
    }

    fn render_with(
        &self,
        gpu: &Gpu,
        camera: &Camera,
        pipeline: &wgpu::ComputePipeline,
    ) -> Result<Frame> {
        let (out, gpu_seconds) = self.trace_frame(gpu, camera, pipeline)?;
        let n = camera.width * camera.height;
        let words = read_buffer(gpu, &out.buffer)?;
        Ok(Frame {
            width: out.width,
            height: out.height,
            hits: decode_all(&words, n),
            ghosts: if out.ghosts {
                decode_ghosts(&words[HIT_WORDS * n..], n)
            } else {
                Vec::new()
            },
            gpu_seconds,
        })
    }
    /// Uniforms for rendering through `camera` (no tile).
    fn camera_params(&self, camera: &Camera) -> Result<Params> {
        let side = |n: usize| u32::try_from(n).ok().filter(|&n| n <= 65535);
        let (Some(width), Some(height)) = (side(camera.width), side(camera.height)) else {
            return Err(Error::Invalid(format!(
                "image too large for the GPU renderer: {}×{}",
                camera.width, camera.height
            )));
        };
        let [f, r, u] = camera.basis();
        let v4 = |v: [f64; 3], w: f64| [v[0] as f32, v[1] as f32, v[2] as f32, w as f32];
        let mut params = self.params(width * height);
        params.width = width;
        params.height = height;
        params.eye = v4(camera.eye, camera.reach);
        params.fwd = v4(f, (0.5 * camera.fov_deg.to_radians()).tan());
        // w: half the height of an orthographic view (0 through a pinhole).
        params.right = v4(r, camera.ortho.map_or(0.0, |h| 0.5 * h));
        params.up = v4(u, 0.0);
        // The eye's distance from each plane in f64: per pixel, only the
        // direction's component along the normal (and an orthographic
        // pixel's offset) is left to f32. A region with a zero normal cuts
        // nothing, as in `Cutaway::kept`.
        let mut planes = 0;
        let mut regions = 0;
        for region in &camera.cut.regions {
            let unit: Option<Vec<[f64; 3]>> = region
                .iter()
                .map(|c| {
                    let len = c.normal.iter().map(|x| x * x).sum::<f64>().sqrt();
                    (len > 0.0 && len.is_finite()).then(|| c.normal.map(|x| x / len))
                })
                .collect();
            let Some(normals) = unit.filter(|n| !n.is_empty()) else {
                continue;
            };
            if regions == MAX_CUT_REGIONS || planes + region.len() > MAX_CUT_PLANES {
                return Err(Error::Invalid(format!(
                    "the GPU renderer cuts away at most {MAX_CUT_REGIONS} regions of \
                     {MAX_CUT_PLANES} planes in all"
                )));
            }
            for (c, n) in region.iter().zip(normals) {
                let s0: f64 = (0..3).map(|i| (camera.eye[i] - c.point[i]) * n[i]).sum();
                params.cut[planes] = v4(n, s0);
                planes += 1;
            }
            params.cut_ends[regions] = planes as u32;
            regions += 1;
        }
        params.cut_regions = regions as u32;
        Ok(params)
    }
}

/// Rays of a camera's image ([`GpuModel::camera_job`]) or explicit rays,
/// traced a slice at a time: in each slice every ray in flight walks at
/// most [`GpuModel::slice_work`] program words, then pauses, so no
/// dispatch runs long whatever the rays, and other work on the GPU (a
/// window's frames, even on another device) gets its turn between them.
/// Rays that end leave room for the job's next ones, so the slowest rays of
/// one part of an image are traced alongside the next part. Camera jobs go
/// by blocks of 8 × 8 pixels in a given order and count the rays left in
/// each ([`Job::encode_progress`]).
pub struct Job {
    bind: wgpu::BindGroup,
    pipeline: wgpu::ComputePipeline,
    plan: wgpu::ComputePipeline,
    /// Control words, then rays left per block, then the blocks' order.
    ctl: wgpu::Buffer,
    /// The next slice's indirect dispatch, copied from `ctl`.
    args: wgpu::Buffer,
    n_blocks: usize,
    rays: u32,
}

impl Job {
    /// Record `n` slices into `enc`. Once every ray is done they dispatch
    /// nothing.
    pub fn encode_slices(&self, enc: &mut wgpu::CommandEncoder, n: usize) {
        for _ in 0..n {
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.plan);
                pass.set_bind_group(0, &self.bind, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            enc.copy_buffer_to_buffer(&self.ctl, 0, &self.args, 0, 12);
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind, &[]);
            pass.dispatch_workgroups_indirect(&self.args, 0);
        }
    }

    /// Record a copy of the job's progress into `to` at byte `offset`:
    /// [`Job::progress_bytes`] bytes, for [`Progress::from_words`].
    pub fn encode_progress(&self, enc: &mut wgpu::CommandEncoder, to: &wgpu::Buffer, offset: u64) {
        enc.copy_buffer_to_buffer(&self.ctl, 0, to, offset, self.progress_bytes());
    }

    /// Bytes of [`Job::encode_progress`].
    pub fn progress_bytes(&self) -> u64 {
        4 * (CTL_WORDS + self.n_blocks) as u64
    }

    /// Rays in the job (64 per block of a camera job).
    pub fn rays(&self) -> u32 {
        self.rays
    }
}

/// Where a [`Job`] is, from the words of [`Job::encode_progress`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    /// Every ray is done.
    pub done: bool,
    /// Rays done.
    pub rays_done: u32,
    /// Workgroups of 64 rays the next slice runs.
    pub groups: u32,
    /// Per block of a camera job, in its order, the rays left (of 64,
    /// counting those past the image's edge): its pixels are written once
    /// it is 0.
    pub blocks_left: Vec<u32>,
}

impl Progress {
    pub fn from_words(w: &[u32]) -> Progress {
        let groups = w[3 + 1 - w[5] as usize];
        Progress {
            done: groups == 0,
            rays_done: w[11],
            groups,
            blocks_left: w[CTL_WORDS..].to_vec(),
        }
    }
}

/// The 8 × 8 blocks of an image that `tile` touches, row by row.
pub fn tile_blocks(tile: Tile) -> Vec<[u32; 2]> {
    if tile.width == 0 || tile.height == 0 {
        return Vec::new();
    }
    let xs = tile.x / 8..(tile.x + tile.width).div_ceil(8);
    (tile.y / 8..(tile.y + tile.height).div_ceil(8))
        .flat_map(|y| xs.clone().map(move |x| [x, y]))
        .collect()
}
