//! The hits shown, moved to the camera wanted now (`shaders/reproject.wgsl`):
//! while the camera moves, the 3D view shades the hits it has as seen from
//! where the camera is, every frame, instead of showing the last image
//! where it was until a new one is traced. Each pixel's point goes to the
//! pixel it falls in now, the nearest winning; holes a pixel wide are
//! filled from the pixels around; what was hidden shows as nothing until
//! traced.

use llevant::Camera;
use llevant_gpu::Gpu;
use wgpu::util::DeviceExt;

use super::{storage, G_WORDS};

/// A camera as the shaders take it (as `Frame` in the prelude; also the
/// resolve pass's).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Cam {
    eye: [f32; 4],
    fwd: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
}

impl Cam {
    pub(super) fn new(c: &Camera) -> Cam {
        let [f, r, u] = c.basis();
        let v4 = |v: [f64; 3], w: f64| [v[0] as f32, v[1] as f32, v[2] as f32, w as f32];
        Cam {
            eye: v4(c.eye, c.reach),
            fwd: v4(f, (0.5 * c.fov_deg.to_radians()).tan()),
            right: v4(r, c.ortho.map_or(0.0, |h| 0.5 * h)),
            up: v4(u, 0.0),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    size: [u32; 4],
    a: Cam,
    b: Cam,
}

/// The pipelines and the buffers of one image size.
pub struct Reprojector {
    layout: wgpu::BindGroupLayout,
    clear: wgpu::ComputePipeline,
    scatter_depth: wgpu::ComputePipeline,
    scatter_owner: wgpu::ComputePipeline,
    gather: wgpu::ComputePipeline,
    /// Per pixel: the nearest depth fallen there, and from which pixel.
    depth: wgpu::Buffer,
    owner: wgpu::Buffer,
    /// The G-buffer moved, laid out as the one read.
    pub out: wgpu::Buffer,
    size: (u32, u32),
}

impl Reprojector {
    pub fn new(gpu: &Gpu) -> Reprojector {
        let device = &gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("reproject.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/reproject.wgsl").into()),
        });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("reproject"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Uniform),
                entry(1, wgpu::BufferBindingType::Storage { read_only: true }),
                entry(2, rw),
                entry(3, rw),
                entry(4, rw),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("reproject"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |name: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(name),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Reprojector {
            clear: pipeline("clear"),
            scatter_depth: pipeline("scatter_depth"),
            scatter_owner: pipeline("scatter_owner"),
            gather: pipeline("gather"),
            layout,
            depth: storage(gpu, "reproject depth", 1),
            owner: storage(gpu, "reproject owner", 1),
            out: storage(gpu, "reprojected gbuf", G_WORDS),
            size: (0, 0),
        }
    }

    /// Record moving `gbuf` (`width` × `height` hits of camera `from`, with
    /// ghost records if `ghosts`) to camera `to` (the same size) into
    /// [`Reprojector::out`].
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        gbuf: &wgpu::Buffer,
        width: u32,
        height: u32,
        ghosts: bool,
        from: &Camera,
        to: &Camera,
    ) {
        let n = (width * height) as usize;
        if self.size != (width, height) {
            self.depth = storage(gpu, "reproject depth", n);
            self.owner = storage(gpu, "reproject owner", n);
            self.out = storage(gpu, "reprojected gbuf", G_WORDS * n);
            self.size = (width, height);
        }
        let u = Uniform {
            size: [width, height, ghosts as u32, 0],
            a: Cam::new(from),
            b: Cam::new(to),
        };
        let uniform = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("reproject"),
                contents: bytemuck::bytes_of(&u),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let buffers = [&uniform, gbuf, &self.depth, &self.owner, &self.out];
        let entries: Vec<wgpu::BindGroupEntry> = buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.as_entire_binding(),
            })
            .collect();
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reproject"),
            layout: &self.layout,
            entries: &entries,
        });
        let mut pass = enc.begin_compute_pass(&Default::default());
        pass.set_bind_group(0, &bind, &[]);
        for p in [
            &self.clear,
            &self.scatter_depth,
            &self.scatter_owner,
            &self.gather,
        ] {
            pass.set_pipeline(p);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
    }
}
