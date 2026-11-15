//! Post-process passes of the 3D view: compute shaders that turn the
//! G-buffer (one hit per pixel) into the image shown, one after the other.
//!
//! A pass is WGSL defining `fn effect(p: vec2<i32>) -> vec4<f32>`, appended
//! to `shaders/prelude.wgsl`, which gives it the hits (`hit_t`,
//! `hit_cell`, `hit_normal`, ...), the previous pass's image (`source`),
//! the camera (`ray_dir`, `hit_point`) and twelve numbers of its own
//! (`F.params`, three `vec4`s: [`Params`]). The first pass ([`Colour`]) makes the base image; the
//! others ([`Outline`], [`AmbientOcclusion`], or any
//! [`PostPass`] of the caller's) change it. [`Ghosts`] (when the
//! visibility has ghost cells) and [`Highlight`] come last, after them:
//! shells of ghost cells, then the selection
//! (`is_selected`, a bit per cell) and the cell under the pointer.
//!
//! A pass may also read storage buffers of its own (a field's values, a
//! lookup table): it declares how many ([`PostPass::data_buffers`]) and
//! binds them in its WGSL at `@group(1) @binding(0 ..)`; the caller fills
//! them with [`PassChain::set_data`]. The prelude's `@group(0)` stays the
//! same for every pass.

use llevant_gpu::Gpu;

const PRELUDE: &str = include_str!("shaders/prelude.wgsl");

/// A pass's numbers: `F.params[0]`, `[1]` and `[2]` in its shader.
pub type Params = [f32; 12];

/// Eight numbers, the last four zero.
pub const fn eight(p: [f32; 8]) -> Params {
    [
        p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7], 0.0, 0.0, 0.0, 0.0,
    ]
}

/// One post-process effect.
pub trait PostPass: Send + Sync {
    /// Unique name (to switch it on and off).
    fn name(&self) -> &str;
    /// WGSL defining `fn effect(p: vec2<i32>) -> vec4<f32>`.
    fn wgsl(&self) -> String;
    /// The pass's numbers, `F.params[0]` to `F.params[2]` in the shader.
    fn params(&self) -> Params {
        [0.0; 12]
    }
    /// How many storage buffers of its own the pass reads, declared in its
    /// WGSL as `@group(1) @binding(k) var<storage, read> ...` for `k` from
    /// 0, and filled with [`PassChain::set_data`] (one zero word each until
    /// then).
    fn data_buffers(&self) -> u32 {
        0
    }
}

/// A pass from WGSL source and fixed numbers, for effects defined at run
/// time.
pub struct WgslPass {
    pub name: String,
    pub source: String,
    pub params: Params,
}

impl PostPass for WgslPass {
    fn name(&self) -> &str {
        &self.name
    }
    fn wgsl(&self) -> String {
        self.source.clone()
    }
    fn params(&self) -> Params {
        self.params
    }
}

/// Base colours: palette, shading (flat, or physically based for
/// realistic colours, as [`llevant_render::shading`]), gaps, problems, the
/// gaps seen through, background, selection.
#[derive(Default)]
pub struct Colour {
    pub pbr: bool,
}

impl Colour {
    pub fn params_for(pbr: bool) -> Params {
        eight([
            if pbr { 1.0 } else { 0.0 },
            llevant_render::colours::VEIL_ALPHA,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ])
    }
}

impl PostPass for Colour {
    fn name(&self) -> &str {
        "colour"
    }
    fn wgsl(&self) -> String {
        include_str!("shaders/colour.wgsl").into()
    }
    fn params(&self) -> Params {
        Colour::params_for(self.pbr)
    }
}

/// Lines along cell boundaries, depth jumps and folds.
pub struct Outline {
    pub strength: f32,
    pub depth_jump: f32,
    pub min_cos: f32,
    /// The lines' colour (0 to 1), `strength` of the way over the image.
    pub colour: [f32; 3],
}

impl Default for Outline {
    fn default() -> Outline {
        Outline {
            strength: 0.55,
            depth_jump: 0.02,
            min_cos: 0.8,
            colour: [0.0; 3],
        }
    }
}

impl PostPass for Outline {
    fn name(&self) -> &str {
        "outline"
    }
    fn wgsl(&self) -> String {
        include_str!("shaders/outline.wgsl").into()
    }
    fn params(&self) -> Params {
        let [r, g, b] = self.colour;
        eight([
            self.strength,
            self.depth_jump,
            self.min_cos,
            0.0,
            r,
            g,
            b,
            0.0,
        ])
    }
}

/// Screen-space ambient occlusion.
pub struct AmbientOcclusion {
    pub radius_px: f32,
    pub strength: f32,
    pub min_depth: f32,
    pub max_depth: f32,
}

impl Default for AmbientOcclusion {
    fn default() -> AmbientOcclusion {
        AmbientOcclusion {
            radius_px: 12.0,
            strength: 0.5,
            min_depth: 0.002,
            max_depth: 0.1,
        }
    }
}

impl PostPass for AmbientOcclusion {
    fn name(&self) -> &str {
        "ambient_occlusion"
    }
    fn wgsl(&self) -> String {
        include_str!("shaders/ssao.wgsl").into()
    }
    fn params(&self) -> Params {
        eight([
            self.radius_px,
            self.strength,
            self.min_depth,
            self.max_depth,
            0.0,
            0.0,
            0.0,
            0.0,
        ])
    }
}

/// The selection, tinted and outlined, and the cell under the pointer,
/// outlined. Always last.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Highlight {
    pub selection: [f32; 3],
    /// How much selected cells take the selection's colour (0..1).
    pub tint: f32,
    pub hover: [f32; 3],
    /// Outline the selection.
    pub outline: bool,
}

impl Default for Highlight {
    fn default() -> Highlight {
        Highlight {
            // theme::SELECTION
            selection: [1.0, 0.78, 0.17],
            tint: 0.35,
            hover: [1.0, 1.0, 1.0],
            outline: true,
        }
    }
}

impl PostPass for Highlight {
    fn name(&self) -> &str {
        "highlight"
    }
    fn wgsl(&self) -> String {
        include_str!("shaders/highlight.wgsl").into()
    }
    fn params(&self) -> Params {
        let [r, g, b] = self.selection;
        let [hr, hg, hb] = self.hover;
        let o = if self.outline { 1.0 } else { 0.0 };
        eight([r, g, b, self.tint, hr, hg, hb, o])
    }
}

/// Shells of ghost cells, from the ghost records after the hits in the
/// G-buffer: as [`llevant_render::ghost`] draws them.
pub struct Ghosts;

impl PostPass for Ghosts {
    fn name(&self) -> &str {
        "ghosts"
    }
    fn wgsl(&self) -> String {
        include_str!("shaders/ghost.wgsl").into()
    }
    fn params(&self) -> Params {
        use llevant_render::ghost::{ALPHA, MAX_ALPHA, RIM};
        eight([ALPHA, RIM, MAX_ALPHA, 0.0, 0.0, 0.0, 0.0, 0.0])
    }
}

/// The uniform block of every pass, as `Frame` in the prelude.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FrameUniform {
    pub size: [u32; 4],
    pub eye: [f32; 4],
    pub fwd: [f32; 4],
    pub right: [f32; 4],
    pub up: [f32; 4],
    pub background: [f32; 4],
    pub params: Params,
}

impl FrameUniform {
    /// For an image of `camera` (its size is the image's), with `hovered`
    /// cell outlined ([`llevant_gpu::NONE`] for none).
    pub fn new(camera: &llevant::Camera, hovered: u32) -> FrameUniform {
        let [f, r, u] = camera.basis();
        let v4 = |v: [f64; 3], w: f64| [v[0] as f32, v[1] as f32, v[2] as f32, w as f32];
        let bg = llevant_render::colours::BACKGROUND.map(|x| x as f32 / 255.0);
        FrameUniform {
            size: [camera.width as u32, camera.height as u32, hovered, 0],
            eye: v4(camera.eye, camera.reach),
            fwd: v4(f, (0.5 * camera.fov_deg.to_radians()).tan()),
            right: v4(r, camera.ortho.map_or(0.0, |h| 0.5 * h)),
            up: v4(u, 0.0),
            background: [bg[0], bg[1], bg[2], 1.0],
            params: [0.0; 12],
        }
    }

    /// With `rgb` behind the model where rays hit nothing.
    pub fn with_background(mut self, rgb: [u8; 3]) -> FrameUniform {
        let c = rgb.map(|x| x as f32 / 255.0);
        self.background = [c[0], c[1], c[2], 1.0];
        self
    }
}

struct Stage {
    pass: Box<dyn PostPass>,
    enabled: bool,
    /// Numbers set with [`PassChain::set_params`], instead of the pass's.
    params: Option<Params>,
    pipeline: wgpu::ComputePipeline,
    /// The pass's own buffers ([`PostPass::data_buffers`]), bound as group 1.
    data: Option<StageData>,
}

struct StageData {
    layout: wgpu::BindGroupLayout,
    bind: wgpu::BindGroup,
}

impl StageData {
    /// Group 1 of a pass reading `n` buffers, each holding `contents[k]`
    /// (one zero word when empty or missing).
    fn new(gpu: &Gpu, name: &str, n: u32, contents: &[&[u8]]) -> StageData {
        use wgpu::util::DeviceExt;
        let layout = data_layout(gpu, n);
        let buffers: Vec<wgpu::Buffer> = (0..n as usize)
            .map(|k| {
                let bytes = contents.get(k).copied().filter(|b| !b.is_empty());
                gpu.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(name),
                        contents: bytes.unwrap_or(&[0; 4]),
                        usage: wgpu::BufferUsages::STORAGE,
                    })
            })
            .collect();
        let entries: Vec<wgpu::BindGroupEntry> = buffers
            .iter()
            .enumerate()
            .map(|(k, b)| wgpu::BindGroupEntry {
                binding: k as u32,
                resource: b.as_entire_binding(),
            })
            .collect();
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(name),
            layout: &layout,
            entries: &entries,
        });
        StageData { layout, bind }
    }
}

/// The layout of `n` read-only storage buffers.
fn data_layout(gpu: &Gpu, n: u32) -> wgpu::BindGroupLayout {
    let entries: Vec<wgpu::BindGroupLayoutEntry> = (0..n)
        .map(|binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        })
        .collect();
    gpu.device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post pass data"),
            entries: &entries,
        })
}

/// Two images the passes write in turn.
pub struct Targets {
    pub width: u32,
    pub height: u32,
    pub textures: [wgpu::Texture; 2],
    pub views: [wgpu::TextureView; 2],
}

impl Targets {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> Targets {
        let make = |label| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: width.max(1),
                    height: height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let textures = [make("view3d a"), make("view3d b")];
        let views = [0, 1].map(|k| textures[k].create_view(&Default::default()));
        Targets {
            width,
            height,
            textures,
            views,
        }
    }
}

/// The passes in order, the first always on, and the highlight after
/// them.
pub struct PassChain {
    stages: Vec<Stage>,
    /// Shells of ghost cells, when `ghosts`.
    ghost: Stage,
    ghosts: bool,
    highlight: Stage,
    layout: wgpu::BindGroupLayout,
}

impl PassChain {
    /// [`Colour`], then the field ([`super::field::FieldPass`], off until
    /// a field is set), [`Outline`] (on), [`AmbientOcclusion`] (off;
    /// the viewer turns it on) and the marks of a geometry being edited
    /// ([`super::edits::EditMarks`], off until something is marked).
    pub fn new(gpu: &Gpu) -> PassChain {
        let device = &gpu.device;
        let buffer = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let ro = wgpu::BufferBindingType::Storage { read_only: true };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post pass"),
            entries: &[
                buffer(0, wgpu::BufferBindingType::Uniform),
                buffer(1, ro),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                buffer(4, ro),
                buffer(5, ro),
            ],
        });
        let highlight =
            stage(gpu, &layout, Box::new(Highlight::default()), true).expect("built-in effect");
        let ghost = stage(gpu, &layout, Box::new(Ghosts), true).expect("built-in effect");
        let mut chain = PassChain {
            stages: Vec::new(),
            ghost,
            ghosts: false,
            highlight,
            layout,
        };
        let builtin: [(Box<dyn PostPass>, bool); 5] = [
            (Box::new(Colour::default()), true),
            (Box::new(super::field::FieldPass), false),
            (Box::new(Outline::default()), true),
            (Box::new(AmbientOcclusion::default()), false),
            (Box::new(super::edits::EditMarks), false),
        ];
        for (pass, on) in builtin {
            chain.push(gpu, pass, on).expect("built-in effect");
        }
        chain
    }

    /// Add a pass at the end. Its WGSL is compiled here: an error in it
    /// comes back as the shader compiler's message, and the pass is not
    /// added.
    pub fn push(
        &mut self,
        gpu: &Gpu,
        pass: Box<dyn PostPass>,
        enabled: bool,
    ) -> Result<(), String> {
        let at = self.stages.len();
        self.insert(gpu, at, pass, enabled)
    }

    /// Add a pass before the one at `index` (after the first, which stays
    /// first), compiled as [`PassChain::push`] does.
    pub fn insert(
        &mut self,
        gpu: &Gpu,
        index: usize,
        pass: Box<dyn PostPass>,
        enabled: bool,
    ) -> Result<(), String> {
        let s = stage(gpu, &self.layout, pass, enabled)?;
        let index = if self.stages.is_empty() {
            0
        } else {
            index.clamp(1, self.stages.len())
        };
        self.stages.insert(index, s);
        Ok(())
    }

    /// Fill the buffers a pass reads ([`PostPass::data_buffers`]), one
    /// slice of bytes each (an empty one stands for one zero word). An
    /// error if there is no such pass or it reads another number of
    /// buffers.
    pub fn set_data(&mut self, gpu: &Gpu, name: &str, contents: &[&[u8]]) -> Result<(), String> {
        let s = self
            .stages
            .iter_mut()
            .find(|s| s.pass.name() == name)
            .ok_or_else(|| format!("no effect called {name}"))?;
        let n = s.pass.data_buffers();
        if contents.len() != n as usize {
            return Err(format!(
                "effect {name} reads {n} buffers, given {}",
                contents.len()
            ));
        }
        s.data = Some(StageData::new(gpu, name, n, contents));
        Ok(())
    }

    /// Switch a pass on or off (the first stays on). Whether it changed.
    pub fn set_enabled(&mut self, name: &str, on: bool) -> bool {
        let mut changed = false;
        for s in self.stages.iter_mut().skip(1) {
            if s.pass.name() == name && s.enabled != on {
                s.enabled = on;
                changed = true;
            }
        }
        changed
    }

    /// Give a pass other numbers (`F.params`); whether there is such a
    /// pass.
    pub fn set_params(&mut self, name: &str, params: Params) -> bool {
        let mut found = false;
        for s in self.stages.iter_mut().filter(|s| s.pass.name() == name) {
            s.params = Some(params);
            found = true;
        }
        found
    }

    /// Whether to draw ghost cells (when the visibility has some).
    pub fn set_ghosts(&mut self, on: bool) {
        self.ghosts = on;
    }

    /// The colours and outline of the selection and of the cell under the
    /// pointer.
    pub fn set_highlight(&mut self, h: Highlight) {
        self.highlight.params = Some(h.params());
    }

    pub fn names(&self) -> Vec<(&str, bool)> {
        self.stages
            .iter()
            .map(|s| (s.pass.name(), s.enabled))
            .collect()
    }

    /// Record the enabled passes into `enc`, then the ghosts (when on) and
    /// the highlight; returns which of `targets` holds the result.
    pub fn encode(
        &self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        frame: &FrameUniform,
        buffers: &PassBuffers,
        targets: &Targets,
    ) -> usize {
        let mut out = 0;
        for (k, s) in self.stages.iter().filter(|s| s.enabled).enumerate() {
            let (read, write) = if k % 2 == 0 { (1, 0) } else { (0, 1) };
            self.run(gpu, enc, s, frame, buffers, targets, [read, write]);
            out = write;
        }
        if self.ghosts {
            self.run(
                gpu,
                enc,
                &self.ghost,
                frame,
                buffers,
                targets,
                [out, 1 - out],
            );
            out = 1 - out;
        }
        self.run(
            gpu,
            enc,
            &self.highlight,
            frame,
            buffers,
            targets,
            [out, 1 - out],
        );
        1 - out
    }

    /// Record one pass from `targets[rw[0]]` to `targets[rw[1]]`.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        s: &Stage,
        frame: &FrameUniform,
        buffers: &PassBuffers,
        targets: &Targets,
        rw: [usize; 2],
    ) {
        use wgpu::util::DeviceExt;
        let [read, write] = rw;
        {
            let mut f = *frame;
            f.params = s.params.unwrap_or_else(|| s.pass.params());
            let uniform = gpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("post pass"),
                    contents: bytemuck::bytes_of(&f),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(s.pass.name()),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: buffers.gbuf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&targets.views[read]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&targets.views[write]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: buffers.palette.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: buffers.marks.as_entire_binding(),
                    },
                ],
            });
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(s.pass.name()),
                timestamp_writes: None,
            });
            pass.set_pipeline(&s.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            if let Some(d) = &s.data {
                pass.set_bind_group(1, &d.bind, &[]);
            }
            pass.dispatch_workgroups(targets.width.div_ceil(8), targets.height.div_ceil(8), 1);
        }
    }
}

/// The buffers every pass reads.
pub struct PassBuffers<'a> {
    /// Hits, [`llevant_gpu::HIT_WORDS`] per pixel.
    pub gbuf: &'a wgpu::Buffer,
    /// Two words per cell (colour, material).
    pub palette: &'a wgpu::Buffer,
    /// The selection, a bit per cell.
    pub marks: &'a wgpu::Buffer,
}

/// A pass compiled over the prelude's group `layout` (and its own buffers,
/// zero until set), or the shader compiler's message.
fn stage(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    pass: Box<dyn PostPass>,
    enabled: bool,
) -> Result<Stage, String> {
    let n = pass.data_buffers();
    let data = (n > 0).then(|| StageData::new(gpu, pass.name(), n, &[]));
    let mut groups = vec![Some(layout)];
    if let Some(d) = &data {
        groups.push(Some(&d.layout));
    }
    let pipeline_layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(pass.name()),
            bind_group_layouts: &groups,
            immediate_size: 0,
        });
    let pipeline = compile(gpu, &pipeline_layout, pass.as_ref())?;
    Ok(Stage {
        pass,
        enabled,
        params: None,
        pipeline,
        data,
    })
}

/// A pass's pipeline, or the shader compiler's message.
fn compile(
    gpu: &Gpu,
    layout: &wgpu::PipelineLayout,
    pass: &dyn PostPass,
) -> Result<wgpu::ComputePipeline, String> {
    let source = format!("{PRELUDE}\n{}", pass.wgsl());
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(pass.name()),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    let pipeline = gpu
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(pass.name()),
            layout: Some(layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
    if let Some(e) = pollster::block_on(scope.pop()) {
        return Err(format!("effect {}: {e}", pass.name()));
    }
    Ok(pipeline)
}
