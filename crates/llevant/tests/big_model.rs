//! Opt-in checks on a large real model:
//! `LLEVANT_BIG_MODEL=path/to/model.mcnp cargo test --release -- --ignored`

use llevant::Depth;
use llevant::{Line, Model, Options, Status};

fn load() -> Option<Model> {
    let path = std::env::var("LLEVANT_BIG_MODEL").ok()?;
    Some(Model::load(path, &Options::default(), None).unwrap())
}

#[test]
#[ignore]
fn big_model_compiles_and_matches_brute_force() {
    let Some(model) = load() else {
        eprintln!("LLEVANT_BIG_MODEL not set; skipping");
        return;
    };
    let stats = model.stats();
    assert_eq!(
        stats.unsupported_cells,
        0,
        "{:?}",
        &model.diagnostics()[..model.diagnostics().len().min(10)]
    );

    // Points inside randomly chosen envelopes (filled level-0 cells).
    let envs: Vec<_> = model.units()[0]
        .cells
        .iter()
        .map(|&c| &model.cells()[c as usize])
        .filter(|c| {
            c.fill.is_some()
                && c.bbox.is_finite()
                && (0..3).all(|i| c.bbox.hi[i] - c.bbox.lo[i] < 1e4)
                // Bounded by its own surfaces, not cut by universe 0's region.
                && (0..3).all(|i| {
                    let roi = model.units()[0].roi;
                    c.bbox.lo[i] > roi.lo[i] && c.bbox.hi[i] < roi.hi[i]
                })
        })
        .map(|c| c.bbox)
        .collect();
    assert!(!envs.is_empty());
    let mut s = 0x9e3779b97f4a7c15u64;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    let points: Vec<[f64; 3]> = (0..20_000)
        .map(|_| {
            let b = envs[(next() * envs.len() as f64) as usize % envs.len()];
            [0, 1, 2].map(|i| b.lo[i] + next() * (b.hi[i] - b.lo[i]))
        })
        .collect();
    let fast = model.classify(&points, Depth::All);
    let slow = llevant::reference::classify(&model, &points, Depth::All);
    for i in 0..points.len() {
        assert_eq!(
            fast.labels[fast.ids[i] as usize], slow.labels[slow.ids[i] as usize],
            "point {:?}",
            points[i]
        );
    }
    let inside = fast
        .ids
        .iter()
        .filter(|&&id| fast.labels[id as usize].status == Status::Inside)
        .count();
    assert!(inside > points.len() * 9 / 10);
}

#[test]
#[ignore]
fn big_model_tracing_matches_brute_force_and_points() {
    let Some(model) = load() else {
        eprintln!("LLEVANT_BIG_MODEL not set; skipping");
        return;
    };
    let envs: Vec<_> = model.units()[0]
        .cells
        .iter()
        .map(|&c| &model.cells()[c as usize])
        .filter(|c| {
            c.fill.is_some()
                && c.bbox.is_finite()
                && (0..3).all(|i| c.bbox.hi[i] - c.bbox.lo[i] < 1e4)
                // Bounded by its own surfaces, not cut by universe 0's region.
                && (0..3).all(|i| {
                    let roi = model.units()[0].roi;
                    c.bbox.lo[i] > roi.lo[i] && c.bbox.hi[i] < roi.hi[i]
                })
        })
        .map(|c| c.bbox)
        .collect();
    let mut s = 0x2545f4914f6cdd1du64;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    let lines: Vec<Line> = (0..5000)
        .map(|_| {
            let b = envs[(next() * envs.len() as f64) as usize % envs.len()];
            let origin = [0, 1, 2].map(|i| b.lo[i] + next() * (b.hi[i] - b.lo[i]));
            let z: f64 = 2.0 * next() - 1.0;
            let phi = 2.0 * std::f64::consts::PI * next();
            let r = (1.0 - z * z).sqrt();
            Line {
                origin,
                dir: [r * phi.cos(), r * phi.sin(), z],
                t_min: 0.0,
                t_max: 100.0,
            }
        })
        .collect();
    let fast = model.trace(&lines, Depth::All).unwrap();
    let few = &lines[..40];
    let slow = llevant::reference::trace(&model, few, Depth::All).unwrap();
    for (i, l) in few.iter().enumerate() {
        let a: Vec<_> = fast.segments(i).map(|k| fast.segment(k)).collect();
        let b: Vec<_> = slow.segments(i).map(|k| slow.segment(k)).collect();
        assert!(
            llevant::reference::traces_agree(&model, l, &a, &b),
            "line {i}"
        );
    }
    // Points along segments classify to the segment's label, except where
    // the line runs within the tolerance of the surface its segment ends on
    // (the point is then on-surface next to the segment's cell).
    let mut pts = Vec::new();
    let mut want = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        for k in fast.segments(i) {
            let (a, b) = (fast.t0[k], fast.t1[k]);
            for f in [0.382, 0.073, 0.917] {
                let t = a + f * (b - a);
                if (t - a).min(b - t) > 1e-6 {
                    pts.push([0, 1, 2].map(|j| l.origin[j] + t * l.dir[j]));
                    want.push(fast.ids[k]);
                }
            }
        }
    }
    let got = model.classify(&pts, Depth::All);
    let (mut bad, mut grazing) = (Vec::new(), 0);
    for i in 0..pts.len() {
        let (p, s) = (
            &got.labels[got.ids[i] as usize],
            &fast.labels[want[i] as usize],
        );
        if p == s {
            continue;
        }
        if llevant::reference::on_surface_next_to(p, s) {
            grazing += 1;
        } else {
            bad.push((pts[i], p.clone(), s.clone()));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {}: {:?}",
        bad.len(),
        pts.len(),
        &bad[..bad.len().min(3)]
    );
    assert!(grazing * 10_000 < pts.len(), "{grazing} of {}", pts.len());
}

/// With gaps and problems hidden, first hits searched only where
/// `Model::skip` leaves something to see are those of the whole search: for
/// random lines across the model, with all filled level-0 cells but one in
/// eight hidden, and with only material cells of every fourth universe.
#[test]
#[ignore]
fn big_model_skipped_first_hits_match() {
    let Some(model) = load() else {
        eprintln!("LLEVANT_BIG_MODEL not set; skipping");
        return;
    };
    let b = model.extent(llevant::Scope::Contents, 64);
    let mut s = 0x2545f4914f6cdd1du64;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    let point = |next: &mut dyn FnMut() -> f64| -> [f64; 3] {
        [0, 1, 2].map(|i| b.lo[i] + next() * (b.hi[i] - b.lo[i]))
    };
    let lines: Vec<Line> = (0..20_000)
        .map(|_| {
            let (a, z) = (point(&mut next), point(&mut next));
            let d = [0, 1, 2].map(|i| z[i] - a[i]);
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            Line {
                origin: a,
                dir: d.map(|x| x / len),
                t_min: 0.0,
                t_max: len,
            }
        })
        .collect();
    let level0 = &model.units()[0].cells;
    let mut few = llevant::Visibility::solid(&model);
    for (k, &c) in level0.iter().enumerate() {
        if k % 8 != 0 {
            few.cells[c as usize] = false;
        }
    }
    let mut some = llevant::Visibility::solid(&model);
    for (c, cell) in model.cells().iter().enumerate() {
        if cell.fill.is_none() && cell.unit % 4 != 1 {
            some.cells[c] = false;
        }
    }
    for (name, mut vis) in [
        ("one envelope in eight", few),
        ("every fourth universe", some),
    ] {
        vis.gaps = false;
        vis.problems = false;
        let skip = model.skip(&vis).unwrap();
        let all = model.first_hits(&lines, &vis, None).unwrap();
        let fast = model.first_hits(&lines, &vis, Some(&skip)).unwrap();
        let mut hits = 0;
        for i in 0..lines.len() {
            assert_eq!(
                all.ids[i] == llevant::NO_HIT,
                fast.ids[i] == llevant::NO_HIT,
                "{name}, line {i}"
            );
            if all.ids[i] == llevant::NO_HIT {
                continue;
            }
            hits += 1;
            assert!((all.t[i] - fast.t[i]).abs() <= 1e-6, "{name}, line {i}");
            assert_eq!(
                all.labels[all.ids[i] as usize], fast.labels[fast.ids[i] as usize],
                "{name}, line {i}"
            );
        }
        eprintln!("{name}: {hits} of {} lines hit, all agree", lines.len());
        assert!(hits > 0, "{name}");
    }
}

/// Move one surface of `cell` by 0.01 in its last coefficient, through
/// migjorn: the new input.
fn moved(input: &str, model: &Model, cell: u32) -> (i64, String) {
    let (shape, _) = model.cell_surfaces(cell)[0];
    let id = model.surface_id(shape).unwrap().id;
    let mut src = migjorn::Model::parse(input);
    let c = src.surface(id).unwrap().coeffs().unwrap();
    let k = c.len() - 1;
    src.surface_mut(id)
        .unwrap()
        .set_coeff(k, c[k] + 0.01)
        .unwrap();
    (id, src.to_source())
}

#[test]
#[ignore]
fn big_model_recompiles_to_the_compiled_model() {
    let Ok(path) = std::env::var("LLEVANT_BIG_MODEL") else {
        eprintln!("LLEVANT_BIG_MODEL not set; skipping");
        return;
    };
    let input = std::fs::read_to_string(path).unwrap();
    let opts = Options::default();
    let model = Model::compile(input.as_bytes(), &opts, None).unwrap();
    let saved = |m: &Model| {
        let mut b = Vec::new();
        m.save(&mut b).unwrap();
        b
    };
    // A surface of an envelope (a filled level-0 cell), then one of a cell
    // of the largest filler.
    let envelope = model.units()[0]
        .cells
        .iter()
        .copied()
        .find(|&c| model.cells()[c as usize].fill.is_some())
        .unwrap();
    let filler = (1..model.units().len())
        .max_by_key(|&u| model.units()[u].cells.len())
        .unwrap();
    let filler_cell = model.units()[filler].cells[0];
    for (what, cell) in [("envelope", envelope), ("filler", filler_cell)] {
        let (surface, new) = moved(&input, &model, cell);
        let t = std::time::Instant::now();
        let (staged, changed) = model.recompile(new.as_bytes(), &opts, None).unwrap();
        let first = t.elapsed().as_secs_f64();
        let done = staged.complete(None).unwrap();
        let all = t.elapsed().as_secs_f64();
        eprintln!(
            "{what}: surface {surface} moved: staged in {first:.2} s, complete in {all:.2} s; \
             {} universes rebuilt, {} cells changed; {:?}",
            changed.universes_rebuilt.len(),
            changed.cells_changed.len(),
            staged.timings()
        );
        let fresh = Model::compile(new.as_bytes(), &opts, None).unwrap();
        assert!(
            saved(&done) == saved(&fresh),
            "{what}: differs from a compile"
        );
        // From the parsed input, as the viewer's card edits: no parse.
        let tree = migjorn::Model::parse(&new);
        let t = std::time::Instant::now();
        let (staged, _) = model.recompile_parsed(&tree, &opts, None).unwrap();
        let first = t.elapsed().as_secs_f64();
        let done = staged.complete(None).unwrap();
        let all = t.elapsed().as_secs_f64();
        eprintln!("{what}: from the parsed input: staged in {first:.2} s, complete in {all:.2} s");
        assert!(
            saved(&done) == saved(&fresh),
            "{what}: recompiled from the parsed input, differs from a compile"
        );
    }
}

/// Remove one level-0 cell near the start of the input: the indices of the
/// cells and surfaces after it move.
#[test]
#[ignore]
fn big_model_recompiles_a_structural_edit() {
    let Ok(path) = std::env::var("LLEVANT_BIG_MODEL") else {
        eprintln!("LLEVANT_BIG_MODEL not set; skipping");
        return;
    };
    let input = std::fs::read_to_string(path).unwrap();
    let opts = Options::default();
    let model = Model::compile(input.as_bytes(), &opts, None).unwrap();
    let saved = |m: &Model| {
        let mut b = Vec::new();
        m.save(&mut b).unwrap();
        b
    };
    // The first unfilled level-0 cell: removing it leaves a gap, nothing
    // else.
    let cell = model.units()[0]
        .cells
        .iter()
        .map(|&c| &model.cells()[c as usize])
        .find(|c| c.fill.is_none() && !c.imp_zero)
        .unwrap()
        .id;
    let mut src = migjorn::Model::parse(&input);
    assert!(src.cell_mut(cell).unwrap().remove());
    let new = src.to_source();
    let t = std::time::Instant::now();
    let (staged, changed) = model.recompile(new.as_bytes(), &opts, None).unwrap();
    let first = t.elapsed().as_secs_f64();
    let done = staged.complete(None).unwrap();
    let all = t.elapsed().as_secs_f64();
    eprintln!(
        "cell {cell} removed: staged in {first:.2} s, complete in {all:.2} s; {} of {} universes \
         rebuilt, {} cells changed",
        changed.universes_rebuilt.len(),
        model.units().len(),
        changed.cells_changed.len(),
    );
    let fresh = Model::compile(new.as_bytes(), &opts, None).unwrap();
    assert!(saved(&done) == saved(&fresh), "differs from a compile");
}

/// After moving a surface (of an envelope, then of a filler), the check of
/// what changed: the instance boxes of the cells changed, old and new, and
/// the lines through them (as the viewer runs it after a reload, 10⁵ lines).
#[test]
#[ignore]
fn big_model_checks_what_an_edit_changed() {
    let Ok(path) = std::env::var("LLEVANT_BIG_MODEL") else {
        eprintln!("LLEVANT_BIG_MODEL not set; skipping");
        return;
    };
    let input = std::fs::read_to_string(path).unwrap();
    let opts = Options::default();
    let model = Model::compile(input.as_bytes(), &opts, None).unwrap();
    let envelope = model.units()[0]
        .cells
        .iter()
        .copied()
        .find(|&c| model.cells()[c as usize].fill.is_some())
        .unwrap();
    let filler = (1..model.units().len())
        .max_by_key(|&u| model.units()[u].cells.len())
        .unwrap();
    // One that can hold something (some have empty boxes: none can).
    let filler_cell = model.units()[filler]
        .cells
        .iter()
        .copied()
        .find(|&c| !model.cells()[c as usize].bbox.is_empty())
        .unwrap();
    for (what, cell) in [("envelope", envelope), ("filler", filler_cell)] {
        let (surface, new) = moved(&input, &model, cell);
        let (staged, changed) = model.recompile(new.as_bytes(), &opts, None).unwrap();
        let done = staged.complete(None).unwrap();
        let t = std::time::Instant::now();
        let old_cells: Vec<u32> = changed
            .cells_changed
            .iter()
            .chain(&changed.cells_removed)
            .filter_map(|&id| model.cell_index(id))
            .collect();
        let new_cells: Vec<u32> = changed
            .cells_changed
            .iter()
            .filter_map(|&id| done.cell_index(id))
            .collect();
        let problem = done.bounds(llevant::Scope::Problem);
        let mut boxes = model.instance_boxes(&old_cells, problem, 0.0);
        boxes.extend(done.instance_boxes(&new_cells, problem, 0.0));
        let found = t.elapsed().as_secs_f64();
        assert!(!boxes.is_empty(), "{what}: no boxes");
        let region = boxes
            .iter()
            .fold(llevant::Aabb::EMPTY, |a, b| a.union(b))
            .intersect(&problem);
        let h = llevant::spacing_within(&region, &boxes, 1e5).unwrap();
        let t = std::time::Instant::now();
        let v = done
            .volumes_within(&llevant::VolumeGrid::new(region, h), &boxes, None)
            .unwrap();
        eprintln!(
            "{what}: surface {surface} moved, {} cells changed: {} boxes in {found:.2} s, \
             checked at h = {h:.3} cm in {:.2} s, {} problem labels",
            changed.cells_changed.len(),
            boxes.len(),
            t.elapsed().as_secs_f64(),
            v.problems().len()
        );
    }
}

/// What the viewer's `shown` costs: every cell that is not filled against
/// a plane through the middle of the problem, and a quarter wedge.
#[test]
#[ignore]
fn big_model_cells_kept_by_cuts() {
    use llevant::{Clip, Cutaway, Scope};
    let Some(model) = load() else {
        eprintln!("LLEVANT_BIG_MODEL not set; skipping");
        return;
    };
    let region = model.bounds(Scope::Problem);
    let mid = region.center();
    let cells: Vec<u32> = (0..model.cells().len() as u32)
        .filter(|&c| model.cell(c).fill.is_none())
        .collect();
    let cuts = [
        Cutaway::plane(Clip {
            point: mid,
            normal: [0.0, 1.0, 0.0],
        }),
        Cutaway::wedge(mid, [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], 0.0, 90.0),
    ];
    for cut in &cuts {
        let t = std::time::Instant::now();
        let kept = model.cells_kept(&cells, region, cut);
        let n = kept.iter().filter(|&&k| k).count();
        eprintln!(
            "{} of {} cells kept in {:.2} s",
            n,
            cells.len(),
            t.elapsed().as_secs_f64()
        );
        assert!(n > 0 && n < cells.len());
    }
}
