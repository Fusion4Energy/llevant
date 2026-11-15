//! An image rendered tile by tile into a `HitBuffer` (`encode_render`) is
//! the image `render` gives, hit for hit. Skipped when no GPU adapter is
//! available.

use llevant::{Aabb, Camera, Clip, Cutaway, Model, Options, Scope};
use llevant_gpu::{Gpu, GpuModel, HitBuffer, Tile, HIT_WORDS};

fn read(gpu: &Gpu, hits: &HitBuffer) -> Vec<u32> {
    let size = hits.buffer.size();
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_buffer_to_buffer(&hits.buffer, 0, &readback, 0, size);
    gpu.queue.submit([enc.finish()]);
    readback.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let words = bytemuck::cast_slice(&readback.get_mapped_range(..).unwrap()).to_vec();
    words
}

#[test]
fn tiles_equal_frames() {
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    let path = format!(
        "{}/../../tools/mcnp-oracle/models/universes.i",
        env!("CARGO_MANIFEST_DIR")
    );
    let model = Model::load(&path, &Options::default(), None).unwrap();
    let gm = GpuModel::new(&gpu, &model.export()).unwrap();
    let region: Aabb = model.extent(Scope::Contents, 64);
    let mut cam = Camera::fit(&region, [1.0, 0.7, -0.4], 101, 67);
    for clip in [None, Some(region.center())] {
        cam.cut = clip.map_or_else(Cutaway::none, |point| {
            Cutaway::plane(Clip {
                point,
                normal: [0.3, 1.0, 0.0],
            })
        });
        let frame = gm.render(&gpu, &cam).unwrap();
        let out = HitBuffer::new(&gpu, 101, 67);
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        // Tiles of odd sizes, cut at the image's edges.
        for y in (0..67).step_by(23) {
            for x in (0..101).step_by(37) {
                let tile = Tile {
                    x,
                    y,
                    width: 37.min(101 - x),
                    height: 23.min(67 - y),
                };
                gm.encode_render(&gpu, &mut enc, &cam, tile, &out).unwrap();
            }
        }
        gpu.queue.submit([enc.finish()]);
        let words = read(&gpu, &out);
        for (k, h) in frame.hits.iter().enumerate() {
            assert_eq!(
                &words[HIT_WORDS * k..][..HIT_WORDS],
                &h.to_words(),
                "pixel {k}"
            );
        }
    }
    // Wrong sizes are refused.
    let out = HitBuffer::new(&gpu, 100, 67);
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    assert!(gm
        .encode_render(&gpu, &mut enc, &cam, Tile::full(101, 67), &out)
        .is_err());
    let out = HitBuffer::new(&gpu, 101, 67);
    let tile = Tile {
        x: 90,
        y: 0,
        width: 12,
        height: 1,
    };
    assert!(gm.encode_render(&gpu, &mut enc, &cam, tile, &out).is_err());
}

/// A camera job's block counts reach 0 only once the block's pixels hold
/// their final hits, on an image whose edge blocks are partial: traced a
/// slice at a time, every block counted done has the frame's hits.
#[test]
fn blocks_done_hold_their_hits() {
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    let path = format!(
        "{}/../../tools/mcnp-oracle/models/universes.i",
        env!("CARGO_MANIFEST_DIR")
    );
    let model = Model::load(&path, &Options::default(), None).unwrap();
    let mut gm = GpuModel::new(&gpu, &model.export()).unwrap();
    let region: Aabb = model.extent(Scope::Contents, 64);
    let (w, h) = (101, 67);
    let cam = Camera::fit(&region, [1.0, 0.7, -0.4], w as usize, h as usize);
    let frame = gm.render(&gpu, &cam).unwrap();
    // Short slices, so that blocks end over many of them.
    gm.slice_work = 8;
    gm.tail_work = 8;
    let out = HitBuffer::new(&gpu, w, h);
    let blocks = llevant_gpu::tile_blocks(Tile::full(w, h));
    let job = gm
        .camera_job(&gpu, &cam, &blocks, &out, gm.max_steps, gm.max_work)
        .unwrap();
    let progress = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: job.progress_bytes(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut checked = vec![false; blocks.len()];
    for _ in 0..10_000 {
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        job.encode_slices(&mut enc, 1);
        job.encode_progress(&mut enc, &progress, 0);
        gpu.queue.submit([enc.finish()]);
        llevant_gpu::map_for_reading(&gpu.device, &progress).unwrap();
        let p_words: Vec<u32> =
            bytemuck::cast_slice(&progress.get_mapped_range(..).unwrap()).to_vec();
        progress.unmap();
        let p = llevant_gpu::Progress::from_words(&p_words);
        let words = read(&gpu, &out);
        for (k, &[bx, by]) in blocks.iter().enumerate() {
            assert!(p.blocks_left[k] <= 64, "block {k}: {}", p.blocks_left[k]);
            if p.blocks_left[k] != 0 || checked[k] {
                continue;
            }
            checked[k] = true;
            for y in 8 * by..(8 * by + 8).min(h) {
                for x in 8 * bx..(8 * bx + 8).min(w) {
                    let i = (y * w + x) as usize;
                    assert_eq!(
                        &words[HIT_WORDS * i..][..HIT_WORDS],
                        &frame.hits[i].to_words(),
                        "pixel ({x}, {y}) of block {k}, counted done"
                    );
                }
            }
        }
        if p.done {
            assert!(checked.iter().all(|&c| c));
            return;
        }
    }
    panic!("the job did not end");
}
