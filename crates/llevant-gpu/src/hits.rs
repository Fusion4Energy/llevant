//! What the shader writes per ray (hits, ghost records) and how it is
//! decoded: word layouts, [`Hit`], [`Frame`], [`HitBuffer`], normals.

use super::*;

/// Words per hit in a [`HitBuffer`]: `t` (f32), cell, surface,
/// `status | depth << 8 | veiled overlap << 14 | veiled gap << 15 | steps << 16`
/// (depth in 6 bits), normal (3 × f32), level-0 cell, cell instance. Status
/// codes are those of [`Status::code`].
pub const HIT_WORDS: usize = 9;

/// Words per ray of a ghost record, after the hits in a [`HitBuffer`] made
/// [`HitBuffer::with_ghosts`]: the boundaries crossed, the first one's cell
/// ([`NONE`] without one), `t` (f32) and normal (octahedral, two snorm16;
/// [`decode_normal`]), as [`llevant::Ghost`].
pub const GHOST_WORDS: usize = 4;

/// The instance word of a hit ([`Hit::instance`]) for instance `id` of a
/// model with `total` instance ids: [`NONE`] without one, or when the ids
/// do not fit in 32 bits (as the GPU renderer writes it).
pub fn instance_word(total: u64, id: Option<u64>) -> u32 {
    match id {
        Some(i) if total < u32::MAX as u64 => i as u32,
        _ => NONE,
    }
}

/// What a ray found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Entered a visible cell.
    Inside,
    /// A point no cell holds.
    Gap,
    /// A point only an unsupported cell may hold.
    Problem,
    /// Nothing visible before `t_max`.
    Miss,
    /// Gave up after [`GpuModel::max_steps`] steps or [`GpuModel::max_work`]
    /// program words.
    Steps,
    /// Nesting deeper than [`MAX_LEVELS`].
    TooDeep,
}

/// First hit of one ray.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub t: f32,
    pub status: Status,
    /// Deepest cell of the path (index into the model's cells), or
    /// [`NONE`] (misses, gaps).
    pub cell: u32,
    /// Level-0 cell of the path, or [`NONE`].
    pub top_cell: u32,
    /// Compiled shape the ray entered through, [`NONE`] when it started in
    /// the visible segment.
    pub surface: u32,
    /// Unit normal of that surface in the global frame, facing the ray.
    pub normal: [f32; 3],
    /// The instance of the cell hit ([`llevant::Instances`]), or [`NONE`]:
    /// misses, gaps, and models with 2³² instances or more.
    pub instance: u32,
    /// Levels in the path (at most 63 written).
    pub depth: u8,
    /// The veils the ray went through before its hit ([`llevant::Veils`];
    /// the renderer sees gaps only).
    pub veils: llevant::Veils,
    /// Locate steps the ray took.
    pub steps: u16,
}

/// A rendered frame: one hit per pixel, rows from the top.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub hits: Vec<Hit>,
    /// Per pixel, the ghost boundaries crossed before the hit; empty when
    /// the visibility has no ghosts.
    pub ghosts: Vec<llevant::Ghost>,
    /// Wall time of the dispatch, from submit to completion (no readback).
    pub gpu_seconds: f64,
}

impl Frame {
    /// The probes of the pixels, for [`llevant_render::shading::render_image`].
    pub fn probes(&self) -> Vec<Probe> {
        self.hits.iter().map(|h| h.probe()).collect()
    }
}

/// A rectangle of a camera's image: corner `(x, y)` (column, row from the
/// top) and size in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Tile {
    /// All of a `width × height` image.
    pub fn full(width: u32, height: u32) -> Tile {
        Tile {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    pub fn pixels(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

/// Hits of an image kept on the GPU, [`HIT_WORDS`] words per pixel, rows
/// from the top: what [`GpuModel::encode_render`] writes, for shaders to
/// read without a readback. Made [`HitBuffer::with_ghosts`], the ghost
/// records follow ([`GHOST_WORDS`] per pixel, from
/// [`HitBuffer::ghost_offset`] on), written when the visibility has ghosts.
pub struct HitBuffer {
    pub buffer: wgpu::Buffer,
    pub width: u32,
    pub height: u32,
    /// Whether the buffer has room for ghost records.
    pub ghosts: bool,
}

impl HitBuffer {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> HitBuffer {
        HitBuffer::sized(gpu, width, height, false)
    }

    /// A buffer with room for the ghost records too.
    pub fn with_ghosts(gpu: &Gpu, width: u32, height: u32) -> HitBuffer {
        HitBuffer::sized(gpu, width, height, true)
    }

    pub(crate) fn sized(gpu: &Gpu, width: u32, height: u32, ghosts: bool) -> HitBuffer {
        let n = (width as u64 * height as u64).max(1);
        let words = HIT_WORDS + if ghosts { GHOST_WORDS } else { 0 };
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hits"),
            size: words as u64 * 4 * n,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        HitBuffer {
            buffer,
            width,
            height,
            ghosts,
        }
    }

    /// The word where the ghost records start.
    pub fn ghost_offset(&self) -> u32 {
        HIT_WORDS as u32 * self.width * self.height
    }
}

/// The ghost records of `n` rays.
pub(crate) fn decode_ghosts(words: &[u32], n: usize) -> Vec<llevant::Ghost> {
    words[..GHOST_WORDS * n]
        .as_chunks::<GHOST_WORDS>()
        .0
        .iter()
        .map(|w| ghost_from_words(w))
        .collect()
}

/// The ghost record of [`GHOST_WORDS`] words of a [`HitBuffer`].
pub fn ghost_from_words(w: &[u32]) -> llevant::Ghost {
    let count = w[0];
    llevant::Ghost {
        count,
        cell: if count > 0 { w[1] } else { llevant::NO_CELL },
        t: if count > 0 {
            f32::from_bits(w[2]) as f64
        } else {
            f64::INFINITY
        },
        normal: if count > 0 {
            decode_normal(w[3]).map(|x| x as f64)
        } else {
            [0.0; 3]
        },
    }
}

/// The [`GHOST_WORDS`] words of a ghost record, as the shader writes them.
pub fn ghost_to_words(g: &llevant::Ghost) -> [u32; GHOST_WORDS] {
    [
        g.count,
        g.cell,
        (g.t as f32).to_bits(),
        encode_normal(g.normal.map(|x| x as f32)),
    ]
}

/// A unit vector packed as the shader packs ghost normals (octahedral, two
/// snorm16).
pub fn encode_normal(n: [f32; 3]) -> u32 {
    let l1 = (n[0].abs() + n[1].abs() + n[2].abs()).max(1e-20);
    let n = n.map(|x| x / l1);
    let sign = |x: f32| if x >= 0.0 { 1.0 } else { -1.0 };
    let e = if n[2] < 0.0 {
        [
            (1.0 - n[1].abs()) * sign(n[0]),
            (1.0 - n[0].abs()) * sign(n[1]),
        ]
    } else {
        [n[0], n[1]]
    };
    let q = |x: f32| (x.clamp(-1.0, 1.0) * 32767.0).round() as i16 as u16 as u32;
    q(e[0]) | (q(e[1]) << 16)
}

/// A unit vector packed as the shader packs ghost normals: octahedral,
/// two snorm16 (zero for a zero vector).
pub fn decode_normal(w: u32) -> [f32; 3] {
    let e = [w & 0xffff, w >> 16].map(|x| (x as u16 as i16 as f32 / 32767.0).clamp(-1.0, 1.0));
    let z = 1.0 - e[0].abs() - e[1].abs();
    let (x, y) = if z < 0.0 {
        (
            (1.0 - e[1].abs()) * e[0].signum(),
            (1.0 - e[0].abs()) * e[1].signum(),
        )
    } else {
        (e[0], e[1])
    };
    let len = (x * x + y * y + z * z).sqrt();
    if len == 0.0 {
        return [0.0; 3];
    }
    [x / len, y / len, z / len]
}

pub(crate) fn decode_all(words: &[u32], n: usize) -> Vec<Hit> {
    words[..HIT_WORDS * n]
        .as_chunks::<HIT_WORDS>()
        .0
        .iter()
        .map(|w| decode(w))
        .collect()
}

impl Status {
    /// The status code in a [`HitBuffer`] (low byte of word 3).
    pub fn code(self) -> u32 {
        match self {
            Status::Inside => 0,
            Status::Gap => 1,
            Status::Problem => 2,
            Status::Miss => 3,
            Status::Steps => 4,
            Status::TooDeep => 5,
        }
    }

    fn from_code(code: u32) -> Status {
        match code {
            0 => Status::Inside,
            1 => Status::Gap,
            2 => Status::Problem,
            3 => Status::Miss,
            4 => Status::Steps,
            _ => Status::TooDeep,
        }
    }
}

pub(crate) fn decode(w: &[u32]) -> Hit {
    let status = Status::from_code(w[3] & 0xff);
    Hit {
        t: f32::from_bits(w[0]),
        status,
        cell: w[1],
        top_cell: w[7],
        instance: w[8],
        surface: w[2],
        normal: [4, 5, 6].map(|i| f32::from_bits(w[i])),
        depth: ((w[3] >> 8) & 0x3f) as u8,
        veils: llevant::Veils {
            gap: (w[3] >> 15) & 1 == 1,
            overlap: (w[3] >> 14) & 1 == 1,
        },
        steps: (w[3] >> 16) as u16,
    }
}

impl Hit {
    /// The hit of [`HIT_WORDS`] words of a [`HitBuffer`].
    pub fn from_words(w: &[u32]) -> Hit {
        decode(w)
    }

    /// What the ray found, as [`llevant_render::shading`] takes it. A ray that
    /// gave up is a problem in an image.
    pub fn probe(&self) -> Probe {
        let what = match self.status {
            Status::Miss => {
                return Probe {
                    veils: self.veils,
                    ..Probe::MISS
                }
            }
            Status::Inside if self.cell != NONE => Found::Cell(self.cell),
            Status::Gap => Found::Gap,
            _ => Found::Problem,
        };
        Probe {
            t: self.t as f64,
            what,
            instance: (self.instance != NONE).then_some(self.instance as u64),
            normal: self.normal.map(f64::from),
            veils: self.veils,
        }
    }

    /// The [`HIT_WORDS`] words of this hit in a [`HitBuffer`].
    pub fn to_words(&self) -> [u32; HIT_WORDS] {
        [
            self.t.to_bits(),
            self.cell,
            self.surface,
            self.status.code()
                | (self.depth.min(63) as u32) << 8
                | (self.veils.overlap as u32) << 14
                | (self.veils.gap as u32) << 15
                | (self.steps as u32) << 16,
            self.normal[0].to_bits(),
            self.normal[1].to_bits(),
            self.normal[2].to_bits(),
            self.top_cell,
            self.instance,
        ]
    }

    /// Whether the ray stopped at something (a cell, a gap or a problem).
    pub fn is_hit(&self) -> bool {
        matches!(
            self.status,
            Status::Inside | Status::Gap | Status::Problem | Status::TooDeep
        )
    }
}
