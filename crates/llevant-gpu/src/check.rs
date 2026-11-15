//! GPU first hits against the CPU's, for the tests and `tools/bench`: views
//! of a model (from outside, close up, from inside; cutaways), visibilities
//! to see it with (all, solid cells, a random half, levels, ghosts, one part
//! isolated), and comparisons of GPU hits with CPU [`Model::first_hits`] ray
//! by ray ([`compare`]) and of whole frames with CPU [`Model::render`]
//! ([`compare_frame`]). The GPU is approximate by design: [`Agreement`]
//! counts how often it agrees, and callers decide what is enough.

use llevant::{
    Aabb, Camera, Clip, Cutaway, Fill, Line, Model, Status as CpuStatus, Visibility, NO_HIT,
    NO_SURFACE,
};

use crate::{Gpu, GpuModel, Status, NONE};

/// One GPU device for every test of a process, opened on first use; `None`
/// (with a note on stderr) when there is no adapter, and the test skips.
/// Opening devices at once on several test threads can hang a Vulkan driver
/// (and costs a second or more each on lavapipe), so tests share this one.
pub fn shared_gpu() -> Option<std::sync::Arc<Gpu>> {
    static GPU: std::sync::OnceLock<Option<std::sync::Arc<Gpu>>> = std::sync::OnceLock::new();
    GPU.get_or_init(|| match Gpu::new() {
        Ok(g) => Some(std::sync::Arc::new(g)),
        Err(e) => {
            eprintln!("no GPU, GPU tests skipped: {e}");
            None
        }
    })
    .clone()
}

/// The whole model from outside, cut through its middle: the near half
/// away, and an oblique cut.
pub fn cutaways(model: &Model, n: usize) -> Vec<(&'static str, Camera)> {
    let (_, whole) = views(model, n).swap_remove(0);
    let c = whole.target;
    let cut = |normal| Camera {
        cut: Cutaway::plane(Clip { point: c, normal }),
        ..whole.clone()
    };
    let towards_eye = [0, 1, 2].map(|i| whole.eye[i] - c[i]);
    let r = 0.25 * llevant::vec3::norm(llevant::vec3::sub(whole.eye, c));
    let section = llevant::Aabb {
        lo: c.map(|x| x - r),
        hi: c.map(|x| x + r),
    };
    vec![
        ("cut, near half away", cut(towards_eye)),
        ("cut, oblique", cut([1.0, 0.3, 2.0])),
        (
            "cut, wedge of 270° about z",
            Camera {
                cut: Cutaway::wedge(c, [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], 30.0, 300.0),
                ..whole.clone()
            },
        ),
        (
            "section box",
            Camera {
                cut: Cutaway::keep_box(&section),
                ..whole.clone()
            },
        ),
    ]
}

/// Every cell, solid cells, a random half of the solid cells, a kept
/// envelope and filler cell, solid cells at levels 0 and 1, the kept
/// envelope a ghost, every filled level-0 cell a ghost, and solid cells with
/// gaps seen through (veils).
pub fn visibilities(model: &Model) -> Vec<(&'static str, Visibility)> {
    let mut half = Visibility::solid(model);
    let mut rng = 0x9e3779b97f4a7c15u64;
    for v in half.cells.iter_mut() {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        *v &= rng.is_multiple_of(2);
    }
    let (envelope, filler, _, _) = emptied(model);
    let at = |level| Visibility {
        level: Some(level),
        ..Visibility::solid(model)
    };
    // The kept envelope as a ghost, what fills it solid; every filled
    // level-0 cell a ghost.
    let level0 = &model.units()[0].cells;
    let filled: Vec<i64> = level0
        .iter()
        .map(|&c| &model.cells()[c as usize])
        .filter(|c| c.fill.is_some())
        .map(|c| c.id)
        .collect();
    let kept: Vec<i64> = level0
        .iter()
        .filter(|&&c| envelope.cells[c as usize] && model.cells()[c as usize].fill.is_some())
        .map(|&c| model.cells()[c as usize].id)
        .collect();
    let mut ghosted = envelope.clone();
    ghosted.ghost(model, &kept).unwrap();
    let mut shells = Visibility::solid(model);
    shells.ghost(model, &filled).unwrap();
    let veiled = Visibility {
        veil: true,
        ..Visibility::solid(model)
    };
    vec![
        ("all cells", Visibility::all(model)),
        ("solid", Visibility::solid(model)),
        ("random half of solid", half),
        ("one envelope kept", envelope),
        ("one filler cell kept", filler),
        ("level 0", at(0)),
        ("level 1", at(1)),
        ("one envelope a ghost", ghosted),
        ("filled level-0 cells ghosts", shells),
        ("solid, gaps seen through", veiled),
    ]
}

/// As `bench_view`: the whole model from outside and a close-up of a random
/// envelope (filled level-0 cell); plus a view from the middle of the model.
pub fn views(model: &Model, n: usize) -> Vec<(&'static str, Camera)> {
    let boxes = level0_boxes(model);
    let hull = boxes.iter().fold(Aabb::EMPTY, |a, b| a.union(b));
    let c = hull.center();
    let size = (0..3).map(|i| hull.hi[i] - hull.lo[i]).fold(0.0, f64::max);
    let mut rng = 0x9e3779b97f4a7c15u64;
    rng ^= rng << 13;
    rng ^= rng >> 7;
    rng ^= rng << 17;
    let env = boxes[(rng >> 11) as usize % boxes.len()];
    let ec = env.center();
    let esize = (0..3).map(|i| env.hi[i] - env.lo[i]).fold(0.0, f64::max);
    let cam = |target: [f64; 3], dist: f64, fov: f64| Camera {
        eye: [
            target[0] - 0.6 * dist,
            target[1] - 0.7 * dist,
            target[2] + 0.4 * dist,
        ],
        target,
        up: [0.0, 0.0, 1.0],
        fov_deg: fov,
        width: n,
        height: n,
        reach: 2.0 * dist + size,
        cut: Cutaway::none(),
        ortho: None,
    };
    let mut inside = cam(c, 0.01 * size, 70.0);
    inside.reach = size;
    let mut out = vec![
        ("whole model", cam(c, 1.5 * size, 40.0)),
        ("close-up", cam(ec, 1.2 * esize, 50.0)),
        ("inside", inside),
    ];
    // The envelope the emptied visibilities keep, framed.
    let kept = emptied(model).2.intersect(&hull);
    if !kept.is_empty() {
        let ksize = (0..3).map(|i| kept.hi[i] - kept.lo[i]).fold(0.0, f64::max);
        out.push(("kept envelope", cam(kept.center(), 1.5 * ksize, 40.0)));
    }
    out
}

/// Boxes of the filled level-0 cells (envelopes), or of every bounded
/// level-0 cell when none is filled.
pub fn level0_boxes(model: &Model) -> Vec<Aabb> {
    let any_filled = model.units()[0]
        .cells
        .iter()
        .any(|&c| model.cells()[c as usize].fill.is_some());
    model.units()[0]
        .cells
        .iter()
        .map(|&c| &model.cells()[c as usize])
        .filter(|c| {
            (c.fill.is_some() || !any_filled)
                && !c.imp_zero
                && c.bbox.is_finite()
                && (0..3).all(|i| c.bbox.hi[i] - c.bbox.lo[i] < 1e4)
                // Bounded by its own surfaces, not cut by universe 0's region.
                && (0..3).all(|i| {
                    let roi = model.units()[0].roi;
                    c.bbox.lo[i] > roi.lo[i] && c.bbox.hi[i] < roi.hi[i]
                })
        })
        .map(|c| c.bbox)
        .collect()
}

/// The instance word the GPU should write for each CPU label.
fn cpu_instances(model: &Model, labels: &[llevant::Label]) -> Vec<u32> {
    let total = model.instances().total();
    labels
        .iter()
        .map(|l| crate::instance_word(total, model.instance(l)))
        .collect()
}

/// How GPU and CPU first hits agree on a batch of rays.
pub struct Agreement {
    /// Rays compared (those that do not meet an overlap first).
    pub rays: usize,
    /// Same hit or miss and the same deepest cell.
    pub same: usize,
    /// Of those, hits whose normal is not the CPU's (shading).
    pub normal_diff: usize,
    /// Of those, hits in another instance of the cell (a neighbouring
    /// lattice element, at its edge).
    pub instance_diff: usize,
    /// Rays that gave up after the step limit.
    pub out_of_steps: usize,
    /// Of the same, rays whose ghost record differs in count or cell (or
    /// `t`, by more than a step).
    pub ghost_diff: usize,
    /// Of the same, rays that crossed a ghost boundary (CPU).
    pub ghosted: usize,
    /// Of the same, rays that say otherwise whether they went through a
    /// veiled gap.
    pub veil_diff: usize,
    /// Of the same, rays that went through a veiled gap (CPU).
    pub veiled: usize,
    pub text: String,
}

/// Whether CPU ray `i` stops at an overlap or sees through one (veils),
/// which the renderer, not looking for overlaps, cannot match.
fn overlapped(cpu: &llevant::Hits, i: usize) -> bool {
    (cpu.ids[i] != NO_HIT && cpu.labels[cpu.ids[i] as usize].status == CpuStatus::Overlap)
        || cpu.veils.get(i).is_some_and(|v| v.overlap)
}

/// Whether a GPU ghost record is the CPU's: the same count and first cell,
/// at about the same place.
fn ghosts_agree(cpu: &llevant::Ghost, gpu: &llevant::Ghost) -> bool {
    cpu.count == gpu.count
        && cpu.cell == gpu.cell
        && (cpu.count == 0 || (cpu.t - gpu.t).abs() <= 1e-3 * (1.0 + cpu.t.abs()))
}

/// GPU [`GpuModel::render`] against CPU `Model::render`, pixel by pixel.
pub fn compare_frame(
    model: &Model,
    gm: &GpuModel,
    gpu: &Gpu,
    cam: &Camera,
    vis: &Visibility,
) -> Agreement {
    let cpu = model.render(cam, vis).unwrap();
    let f = gm.render(gpu, cam).unwrap();
    let (mut same, mut steps_out, mut overlaps, mut normal_diff, mut cut) = (0, 0, 0, 0, 0);
    let (mut instance_diff, mut ghost_diff, mut ghosted) = (0, 0, 0);
    let (mut veil_diff, mut veiled) = (0, 0);
    let instances = cpu_instances(model, &cpu.labels);
    let mut examples = Vec::new();
    for (i, gh) in f.hits.iter().enumerate() {
        if gh.status == Status::Steps {
            steps_out += 1;
        }
        let cpu_cell = (cpu.ids[i] != NO_HIT).then(|| {
            let l = &cpu.labels[cpu.ids[i] as usize];
            match l.status {
                CpuStatus::Inside => l.path.last().map_or(NONE, |s| s.cell),
                _ => NONE,
            }
        });
        if overlapped(&cpu, i) {
            overlaps += 1;
            continue;
        }
        let gpu_cell = gh.is_hit().then_some(gh.cell);
        if cpu_cell != gpu_cell {
            if examples.len() < 5 {
                examples.push(format!(
                    "pixel {i}: cpu {cpu_cell:?} t {:.4} / gpu {:?} {gpu_cell:?} t {:.4}",
                    cpu.t[i], gh.status, gh.t
                ));
            }
            continue;
        }
        same += 1;
        if !cpu.veils.is_empty() {
            veiled += cpu.veils[i].gap as usize;
            if cpu.veils[i].gap != gh.veils.gap {
                veil_diff += 1;
                if examples.len() < 5 {
                    examples.push(format!(
                        "pixel {i}: cpu veils {:?} / gpu {:?}",
                        cpu.veils[i], gh.veils
                    ));
                }
            }
        }
        if !cpu.ghosts.is_empty() {
            ghosted += (cpu.ghosts[i].count > 0) as usize;
            if !ghosts_agree(&cpu.ghosts[i], &f.ghosts[i]) {
                ghost_diff += 1;
                if examples.len() < 5 {
                    examples.push(format!(
                        "pixel {i}: cpu ghosts {:?} / gpu {:?}",
                        cpu.ghosts[i], f.ghosts[i]
                    ));
                }
            }
        }
        if cpu_cell.is_none() {
            continue;
        }
        if instances[cpu.ids[i] as usize] != gh.instance {
            instance_diff += 1;
            if examples.len() < 5 {
                examples.push(format!(
                    "pixel {i}: cpu instance {} / gpu {}",
                    instances[cpu.ids[i] as usize], gh.instance
                ));
            }
        }
        cut += (cpu.surface[i] == NO_SURFACE && cpu.normal[i] != [0.0; 3]) as usize;
        let n = cpu.normal[i];
        let cos: f64 = (0..3).map(|k| n[k] * gh.normal[k] as f64).sum();
        if (n != [0.0; 3] || gh.normal != [0.0; 3]) && cos < 0.999 {
            normal_diff += 1;
            if examples.len() < 5 {
                examples.push(format!(
                    "pixel {i}: cpu normal {n:.3?} surface {} / gpu normal {:.3?} surface {}",
                    cpu.surface[i], gh.normal, gh.surface
                ));
            }
        }
    }
    let rays = f.hits.len() - overlaps;
    let mut s = format!(
        "{:.3}% same ({normal_diff} of them in normal, {instance_diff} in instance, {ghost_diff} of {ghosted} in ghosts, {veil_diff} of {veiled} in veils), {cut} on the cut, {steps_out} out of steps, {overlaps} overlaps not compared; gpu {:.3} s",
        100.0 * same as f64 / rays as f64,
        f.gpu_seconds
    );
    for e in examples {
        s.push_str(
            "
    ",
        );
        s.push_str(&e);
    }
    Agreement {
        rays,
        same,
        normal_diff,
        instance_diff,
        out_of_steps: steps_out,
        ghost_diff,
        ghosted,
        veil_diff,
        veiled,
        text: s,
    }
}

pub fn compare(
    model: &Model,
    gm: &GpuModel,
    gpu: &Gpu,
    rays: &[Line],
    vis: &Visibility,
) -> Agreement {
    let cpu = model.first_hits(rays, vis, None).unwrap();
    let t0 = std::time::Instant::now();
    let (g, gg) = gm
        .trace_rays_ghosts(gpu, rays, gm.max_steps)
        .expect("GPU dispatch");
    let gsecs = t0.elapsed().as_secs_f64();
    let (mut ghost_diff, mut ghosted) = (0, 0);
    let (mut veil_diff, mut veiled) = (0, 0);
    let (mut same, mut cell_diff, mut hit_diff, mut steps_out, mut hits) = (0, 0, 0, 0, 0);
    let mut max_dt = 0.0f64;
    let mut overlaps = 0;
    let mut normal_diff = 0;
    let mut instance_diff = 0;
    let instances = cpu_instances(model, &cpu.labels);
    let mut examples = Vec::new();
    for i in 0..rays.len() {
        let gh = &g[i];
        if gh.status == Status::Steps {
            steps_out += 1;
        }
        // The renderer does not look for overlaps.
        if overlapped(&cpu, i) {
            overlaps += 1;
            continue;
        }
        let cpu_cell = if cpu.ids[i] == NO_HIT {
            None
        } else {
            let l = &cpu.labels[cpu.ids[i] as usize];
            Some(match l.status {
                CpuStatus::Inside => l.path.last().map_or(NONE, |s| s.cell),
                _ => NONE,
            })
        };
        let gpu_cell = gh.is_hit().then_some(gh.cell);
        hits += cpu_cell.is_some() as usize;
        if cpu_cell.is_some() != gpu_cell.is_some() {
            hit_diff += 1;
            if examples.len() < 5 {
                examples.push(format!(
                    "ray {i}: cpu {:?} t {:.4} / gpu {:?} {:?} t {:.4}",
                    cpu_cell, cpu.t[i], gh.status, gpu_cell, gh.t
                ));
            }
        } else if cpu_cell != gpu_cell {
            cell_diff += 1;
            if examples.len() < 5 {
                examples.push(format!(
                    "ray {i}: cpu cell {:?} t {:.4} / gpu {:?} cell {:?} t {:.4}",
                    cpu_cell.map(|c| model.cells().get(c as usize).map(|c| c.id)),
                    cpu.t[i],
                    gh.status,
                    gpu_cell.map(|c| model.cells().get(c as usize).map(|c| c.id)),
                    gh.t
                ));
            }
        } else {
            same += 1;
            if !cpu.veils.is_empty() {
                veiled += cpu.veils[i].gap as usize;
                if cpu.veils[i].gap != gh.veils.gap {
                    veil_diff += 1;
                    if examples.len() < 5 {
                        examples.push(format!(
                            "ray {i}: cpu veils {:?} / gpu {:?}",
                            cpu.veils[i], gh.veils
                        ));
                    }
                }
            }
            if !cpu.ghosts.is_empty() {
                ghosted += (cpu.ghosts[i].count > 0) as usize;
                if !ghosts_agree(&cpu.ghosts[i], &gg[i]) {
                    ghost_diff += 1;
                    if examples.len() < 5 {
                        examples.push(format!(
                            "ray {i}: cpu ghosts {:?} / gpu {:?}",
                            cpu.ghosts[i], gg[i]
                        ));
                    }
                }
            }
            if cpu_cell.is_some() {
                if instances[cpu.ids[i] as usize] != gh.instance {
                    instance_diff += 1;
                    if examples.len() < 5 {
                        examples.push(format!(
                            "ray {i}: cpu instance {} / gpu {}",
                            instances[cpu.ids[i] as usize], gh.instance
                        ));
                    }
                }
                max_dt = max_dt.max((cpu.t[i] - gh.t as f64).abs());
                // Both face the ray; a zero one has no surface.
                let n = cpu.normal[i];
                let cos: f64 = (0..3).map(|k| n[k] * gh.normal[k] as f64).sum();
                if (n != [0.0; 3] || gh.normal != [0.0; 3]) && cos < 0.999 {
                    normal_diff += 1;
                    if examples.len() < 5 {
                        examples.push(format!(
                            "ray {i}: cpu normal {n:.3?} surface {} / gpu normal {:.3?} surface {}",
                            cpu.surface[i], gh.normal, gh.surface
                        ));
                    }
                }
            }
        }
    }
    let n = (rays.len() - overlaps) as f64;
    let mut s = format!(
        "{:.0}% hit; {:.3}% same ({} of them in normal, {instance_diff} in instance, {ghost_diff} of {ghosted} in ghosts, {veil_diff} of {veiled} in veils), {} differ in hit/miss, {} in cell, {} out of steps, {} overlaps not compared; max |dt| {:.2e}; gpu {:.3} s",
        100.0 * hits as f64 / n,
        100.0 * same as f64 / n,
        normal_diff,
        hit_diff,
        cell_diff,
        steps_out,
        overlaps,
        max_dt,
        gsecs
    );
    for e in examples {
        s.push_str("\n    ");
        s.push_str(&e);
    }
    Agreement {
        rays: rays.len() - overlaps,
        same,
        normal_diff,
        instance_diff,
        out_of_steps: steps_out,
        ghost_diff,
        ghosted,
        veil_diff,
        veiled,
        text: s,
    }
}

/// Views of a model mostly hidden, as when one part is isolated: one
/// level-0 filled cell kept with what fills it (the rest of level 0
/// hidden), and one unfilled material cell of its universe kept alone (with
/// the filled cell above it, through which it is seen). Chosen so that they
/// show something and runs of one model agree: of the filled cells, by
/// decreasing number of material cells in their universe (ties to the lower
/// number), the first that fills a fifth of a small CPU view of its box,
/// and the material cell seen most in that view. Without one, the solid
/// view twice. Also the kept envelope's box (within universe 0's region),
/// and a line saying what was kept.
pub fn emptied(model: &Model) -> (Visibility, Visibility, Aabb, String) {
    let cells = model.cells();
    let level0 = &model.units()[0].cells;
    let solid = Visibility::solid(model);
    let materials = |u: u32| {
        model.units()[u as usize]
            .cells
            .iter()
            .filter(|&&c| cells[c as usize].material != 0 && cells[c as usize].fill.is_none())
            .count()
    };
    let mut filled: Vec<(usize, u32, u32)> = level0
        .iter()
        .filter_map(|&e| match &cells[e as usize].fill {
            Some(Fill::Universe(t)) => Some((materials(t.unit()), e, t.unit())),
            _ => None,
        })
        .filter(|&(n, _, _)| n > 0)
        .collect();
    filled.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(cells[a.1 as usize].id.cmp(&cells[b.1 as usize].id))
    });
    let n = 24;
    for &(_, e, u) in filled.iter().take(64) {
        let b = cells[e as usize].bbox.intersect(&model.units()[0].roi);
        if b.is_empty() || !b.is_finite() {
            continue;
        }
        let mut envelope = solid.clone();
        for &c in level0 {
            envelope.cells[c as usize] = c == e;
        }
        let cam = Camera::fit(&b, [1.0, 0.7, -0.5], n, n);
        let rays: Vec<Line> = cam.rays().into_iter().flatten().collect();
        let Ok(h) = model.first_hits(&rays, &envelope, None) else {
            continue;
        };
        let mut seen: Vec<(u32, usize)> = Vec::new();
        for &id in &h.ids {
            if id == NO_HIT {
                continue;
            }
            if let Some(c) = h.labels[id as usize].cell() {
                if cells[c as usize].unit == u && cells[c as usize].material != 0 {
                    match seen.iter_mut().find(|x| x.0 == c) {
                        Some(x) => x.1 += 1,
                        None => seen.push((c, 1)),
                    }
                }
            }
        }
        let hits: usize = seen.iter().map(|x| x.1).sum();
        if 5 * hits < n * n {
            continue;
        }
        let f = seen
            .iter()
            .max_by(|a, b| {
                a.1.cmp(&b.1)
                    .then(cells[b.0 as usize].id.cmp(&cells[a.0 as usize].id))
            })
            .expect("a cell seen")
            .0;
        let mut filler = envelope.clone();
        for &c in &model.units()[u as usize].cells {
            filler.cells[c as usize] = c == f;
        }
        let what = format!(
            "envelope {} kept; filler cell {} of universe {} kept",
            cells[e as usize].id,
            cells[f as usize].id,
            model.units()[u as usize].universe
        );
        return (envelope, filler, b, what);
    }
    (
        solid.clone(),
        solid,
        Aabb::EMPTY,
        "no envelope to keep".into(),
    )
}
