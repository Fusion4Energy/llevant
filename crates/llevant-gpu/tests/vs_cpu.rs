//! GPU first hits against CPU `first_hit` on the MCNP zoo models, from
//! outside, close up and from inside, with every cell, solid cells and a
//! random half of them visible, at levels 0 and 1, with ghosts, and
//! cutaway frames; the instance of each cell hit and the ghost boundaries
//! crossed too. Again in slices of a few words, so that rays pause and
//! resume many times. Skipped when no GPU adapter is available
//! (lavapipe, Mesa's software Vulkan, is enough).

use llevant::{Line, Model, Options};
use llevant_gpu::{check, GpuModel, SLICE_WORK};

/// Rays walk at most this many words per slice in the `*_in_slices` tests.
const TINY_SLICE: u32 = 16;

fn agree(model_file: &str, slice_work: u32) {
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    let path = format!(
        "{}/../../tools/mcnp-oracle/models/{model_file}",
        env!("CARGO_MANIFEST_DIR")
    );
    let model = Model::load(&path, &Options::default(), None).unwrap();
    let mut gm = GpuModel::new(&gpu, &model.export()).unwrap();
    gm.slice_work = slice_work;
    let n = 80;
    for (name, vis) in check::visibilities(&model) {
        gm.set_visibility(&gpu, &model, &vis).unwrap();
        for (vname, cam) in check::views(&model, n) {
            let rays: Vec<Line> = cam.rays().into_iter().flatten().collect();
            let r = check::compare(&model, &gm, &gpu, &rays, &vis);
            println!("{model_file}, {name}, {vname}: {}", r.text);
            assert_eq!(r.out_of_steps, 0, "{model_file}, {name}, {vname}");
            // Edges, slivers thinner than a step and overlaps may differ.
            assert!(
                r.same as f64 >= 0.995 * r.rays as f64,
                "{model_file}, {name}, {vname}: {}",
                r.text
            );
            // Hits on the same cell shade alike (a few may enter through a
            // different one of coincident surfaces).
            assert!(
                r.normal_diff as f64 <= 0.005 * r.same as f64,
                "{model_file}, {name}, {vname}: {}",
                r.text
            );
            // and are in the same instance of it (but at lattice element
            // edges).
            assert!(
                r.instance_diff as f64 <= 0.005 * r.same as f64,
                "{model_file}, {name}, {vname}: {}",
                r.text
            );
            // and cross the same ghost boundaries (levels and ghosts
            // among the visibilities).
            assert!(
                r.ghost_diff as f64 <= 0.005 * r.same as f64,
                "{model_file}, {name}, {vname}: {}",
                r.text
            );
        }
        // Whole frames through the shader's own rays, cut by a plane.
        for (vname, cam) in check::cutaways(&model, n) {
            let r = check::compare_frame(&model, &gm, &gpu, &cam, &vis);
            println!("{model_file}, {name}, {vname}: {}", r.text);
            assert_eq!(r.out_of_steps, 0, "{model_file}, {name}, {vname}");
            assert!(
                r.same as f64 >= 0.995 * r.rays as f64,
                "{model_file}, {name}, {vname}: {}",
                r.text
            );
            assert!(
                r.normal_diff as f64 <= 0.005 * r.same as f64,
                "{model_file}, {name}, {vname}: {}",
                r.text
            );
            assert!(
                r.instance_diff as f64 <= 0.005 * r.same as f64,
                "{model_file}, {name}, {vname}: {}",
                r.text
            );
            // and cross the same ghost boundaries (levels and ghosts
            // among the visibilities).
            assert!(
                r.ghost_diff as f64 <= 0.005 * r.same as f64,
                "{model_file}, {name}, {vname}: {}",
                r.text
            );
        }
    }
}

#[test]
fn universes() {
    agree("universes.i", SLICE_WORK);
}

#[test]
fn surfaces() {
    agree("surfaces.i", SLICE_WORK);
}

#[test]
fn periodic() {
    agree("periodic.i", SLICE_WORK);
}

#[test]
fn universes_in_slices() {
    agree("universes.i", TINY_SLICE);
}

#[test]
fn surfaces_in_slices() {
    agree("surfaces.i", TINY_SLICE);
}

#[test]
fn periodic_in_slices() {
    agree("periodic.i", TINY_SLICE);
}

/// Gaps seen through (`Visibility::veil`): holes at level 0, in a filled
/// universe and lattice elements out of the fill's range, with octrees
/// and with BVHs only, so that the renderer's way out of a gap
/// (`gap_exit`) is tried in each kind of unit. The void cells around the
/// holes are ghosts, not hidden: the renderer's skip jumps hidden space,
/// gaps in it unseen. Rays must go through gaps, and say so (and cross
/// the ghost boundaries) as the CPU does.
#[test]
fn gaps_seen_through() {
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    let path = format!("{}/tests/gaps.i", env!("CARGO_MANIFEST_DIR"));
    for grid in [llevant::GridMode::Always, llevant::GridMode::Off] {
        let model = Model::load(&path, &Options::default().with_grid(grid), None).unwrap();
        let mut gm = GpuModel::new(&gpu, &model.export()).unwrap();
        let mut outer = llevant::Visibility::all(&model);
        outer.hide(&model, &[9]).unwrap();
        outer.ghost(&model, &[8]).unwrap();
        outer.veil = true;
        let mut voids = outer.clone();
        voids.ghost(&model, &[12, 32]).unwrap();
        for slice_work in [SLICE_WORK, TINY_SLICE] {
            gm.slice_work = slice_work;
            for (name, vis) in [("outer ghost", &outer), ("void ghosts", &voids)] {
                gm.set_visibility(&gpu, &model, vis).unwrap();
                let mut veiled = 0;
                for (vname, cam) in check::views(&model, 120) {
                    let rays: Vec<Line> = cam.rays().into_iter().flatten().collect();
                    let r = check::compare(&model, &gm, &gpu, &rays, vis);
                    println!("{grid:?}, {name}, {vname}: {}", r.text);
                    assert_eq!(r.out_of_steps, 0, "{grid:?}, {name}, {vname}");
                    assert!(
                        r.same as f64 >= 0.995 * r.rays as f64,
                        "{grid:?}, {name}, {vname}: {}",
                        r.text
                    );
                    assert!(
                        r.ghost_diff as f64 <= 0.005 * r.same as f64,
                        "{grid:?}, {name}, {vname}: {}",
                        r.text
                    );
                    assert!(
                        r.veil_diff as f64 <= 0.005 * r.same as f64,
                        "{grid:?}, {name}, {vname}: {}",
                        r.text
                    );
                    veiled += r.veiled;
                }
                assert!(veiled > 100, "{grid:?}, {name}: {veiled} rays through gaps");
            }
        }
    }
}

/// A void cell cut around a row of parts (`#1 #2 …`, as CAD conversions
/// write the space around them), with rays along the row that pass through
/// the parts' boxes without entering them, and rays into the parts. The
/// void's exit is where its program can turn false (`EXIT_BOUND`), so a ray
/// that passes a part by does not stop at its planes; hits are as the CPU's,
/// with or without, octrees on or off (off: exits take the cell's whole
/// program, as on a large model after `MAX_LEAVES` leaves).
#[test]
fn void_exits_skip_crossings_that_keep_the_cell() {
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    // Parts: square prisms turned 45° about z (|x - c| + |y| <= 1), from
    // z = -1 up to a top tilted along y (z = 1 + y/2), so that their boxes
    // reach z = 1.5 above where they are.
    let parts = 20;
    let mut deck = String::from("row of parts in a void cut around them\n");
    for k in 1..=parts {
        let s = 100 + 10 * k;
        deck.push_str(&format!(
            "{k} 1 -7.8 -{} {} -{} {} 91 -92 imp:n=1\n",
            s + 1,
            s + 2,
            s + 3,
            s + 4
        ));
    }
    deck.push_str("100 0 -99");
    for k in 1..=parts {
        deck.push_str(&format!("\n     #{k}"));
    }
    deck.push_str(" imp:n=1\n101 0 99 imp:n=0\n\n");
    for k in 1..=parts {
        let (s, c) = (100 + 10 * k, 4.0 * (k - 1) as f64);
        deck.push_str(&format!("{} p 1 1 0 {}\n", s + 1, c + 1.0));
        deck.push_str(&format!("{} p 1 1 0 {}\n", s + 2, c - 1.0));
        deck.push_str(&format!("{} p 1 -1 0 {}\n", s + 3, c + 1.0));
        deck.push_str(&format!("{} p 1 -1 0 {}\n", s + 4, c - 1.0));
    }
    deck.push_str("91 pz -1\n92 p 0 -0.5 1 1\n99 rpp -60 100 -20 20 -20 20\n\nm1 26000 1\n");
    // Along +x from x = -50, just above the parts' tops for small y (past
    // them, through their boxes) and below them for larger y (into them).
    let mut rays = Vec::new();
    for i in 0..21 {
        for j in 0..9 {
            let (y, z) = (-0.9 + 0.09 * i as f64, 1.05 + 0.05 * j as f64);
            rays.push(Line::new([-50.0, y, z], [1.0, 0.0, 0.0], 0.0, 1e3));
        }
    }
    for grid in [llevant::GridMode::Off, llevant::GridMode::Always] {
        let model = Model::from_text(&deck, &Options::default().with_grid(grid));
        let vis = llevant::Visibility::solid(&model);
        let mut gm = GpuModel::new(&gpu, &model.export()).unwrap();
        gm.set_visibility(&gpu, &model, &vis).unwrap();
        let mut steps = Vec::new();
        for bound in [0.0, 1.0] {
            gm.set_constants(&gpu, &[("EXIT_BOUND", bound)]);
            let r = check::compare(&model, &gm, &gpu, &rays, &vis);
            println!("{grid:?}, exit bound {bound}: {}", r.text);
            assert_eq!(r.out_of_steps, 0, "{grid:?}, exit bound {bound}");
            assert_eq!(r.same, r.rays, "{grid:?}, exit bound {bound}: {}", r.text);
            let hits = gm.trace_rays(&gpu, &rays);
            assert!(hits.iter().any(|h| h.is_hit()) && hits.iter().any(|h| !h.is_hit()));
            steps.push(hits.iter().map(|h| h.steps as u64).sum::<u64>());
        }
        println!(
            "{grid:?}, steps of all rays: first crossing {}, exit bound {}",
            steps[0], steps[1]
        );
        // Passing a part by stops at its planes, but for the bound (11 337
        // and 7 337 steps against 378 when written).
        assert!(steps[1] * 5 < steps[0], "{grid:?}, steps {steps:?}");
    }
}
