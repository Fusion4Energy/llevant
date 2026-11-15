//! `report <model...> [--json file] [--markdown file] [--no-gpu]`: the
//! numbers `docs/benchmarks.md` quotes, measured in one run per model and
//! written as JSON and as that page's tables, with the machine, the build
//! profile, the commit and the date. Build with `--profile dist` (what is
//! shipped) for published numbers.
//!
//! Per model: compiling (no cache), to a staged model (no octrees), and
//! again against the compiled model after one surface of the last cell
//! moved, and that surface dragged live (`llevant::draft::Draft`: starting the
//! drag, a step, and on the GPU writing a step into the model uploaded);
//! reading the compiled model back;
//! points and 100 cm lines drawn uniformly inside the model's extent, on
//! every thread and on one; a 1000-row slice across the middle; a CPU render
//! (800 × 600, solid cells); fingerprints; a gap and overlap check of the
//! contents' extent on lines about 1/400 of its longest side apart; and,
//! with a GPU, its upload and a 1920 × 1080 frame of the same view. Times are the best of a few runs where
//! a run is short.

use std::sync::Arc;
use std::time::Instant;

use llevant::draft::Draft;
use llevant::{
    Axis, Camera, Depth, Line, Model, Scope, Slice, Visibility, VolumeGrid, DEFAULT_PRECISION,
};
use serde_json::{json, Value};

use crate::rng::Rng;

const POINTS: usize = 2_000_000;
const LINES: usize = 200_000;
/// Share of the batch run on one thread.
const ONE_THREAD: usize = 10;

pub fn run(args: Vec<String>) {
    let mut models = Vec::new();
    let (mut json_out, mut md_out, mut gpu) = (None, None, true);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => json_out = Some(it.next().expect("--json <file>")),
            "--markdown" => md_out = Some(it.next().expect("--markdown <file>")),
            "--no-gpu" => gpu = false,
            _ => models.push(a),
        }
    }
    assert!(
        !models.is_empty(),
        "usage: report <model...> [--json file] [--markdown file] [--no-gpu]"
    );
    let gpu = if gpu {
        llevant_gpu::Gpu::new()
            .inspect_err(|e| eprintln!("no GPU: {e}"))
            .ok()
    } else {
        None
    };

    let mut results = Vec::new();
    for path in &models {
        eprintln!("{path}");
        results.push(measure(path, gpu.as_ref()));
        // Each model's GPU work runs on a device of its own (`measure`).
    }
    let report = json!({
        "machine": machine(gpu.as_ref()),
        "profile": env!("LLEVANT_BENCH_PROFILE"),
        "commit": git(&["rev-parse", "--short", "HEAD"]),
        "date": today(),
        "models": results,
    });
    let text = serde_json::to_string_pretty(&report).unwrap() + "\n";
    print!("{text}");
    if let Some(f) = json_out {
        std::fs::write(&f, &text).unwrap();
    }
    if let Some(f) = md_out {
        std::fs::write(&f, markdown(&report)).unwrap();
    }
}

fn measure(path: &str, gpu: Option<&llevant_gpu::Gpu>) -> Value {
    let name = std::path::Path::new(path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let opts = crate::options();
    let t = Instant::now();
    let model = Model::load(path, &opts, None).unwrap();
    let compile = t.elapsed().as_secs_f64();
    let mut bytes = Vec::new();
    model.save(&mut bytes).unwrap();
    let t = Instant::now();
    let model = Arc::new(Model::read(&mut bytes.as_slice()).unwrap());
    let read = t.elapsed().as_secs_f64();
    let saved_mb = bytes.len() as f64 / 1e6;
    drop(bytes);
    eprintln!("  compile {compile:.2} s, read {read:.2} s");

    let input = std::fs::read(path).unwrap();
    let t = Instant::now();
    drop(Model::compile_staged(&input, &opts, None).unwrap());
    let staged = t.elapsed().as_secs_f64();
    let moved = moved_surface(&input);
    let recompile = moved.as_ref().map(|(_, new)| {
        let t = Instant::now();
        let (m, changed) = model.recompile(new.as_bytes(), &opts, None).unwrap();
        let first = t.elapsed().as_secs_f64();
        drop(m.into_complete(None).unwrap());
        let all = t.elapsed().as_secs_f64();
        eprintln!(
            "  staged {staged:.2} s; recompile {first:.2} s, complete {all:.2} s, {} universes rebuilt",
            changed.universes_rebuilt.len()
        );
        json!({
            "staged_s": first,
            "complete_s": all,
            "universes_rebuilt": changed.universes_rebuilt.len(),
        })
    });
    // The same surface dragged: steps of up to 1 cm either way.
    let mut draft = moved.and_then(|(id, _)| {
        let tree = migjorn::Model::parse(std::str::from_utf8(&input).ok()?);
        let t = Instant::now();
        let mut d = Draft::begin(model.clone(), &tree, id)
            .inspect_err(|e| eprintln!("  drag: {e}"))
            .ok()?;
        let begin = t.elapsed().as_secs_f64();
        let start = d.coeffs().to_vec();
        let k = start.len() - 1;
        let mut steps: Vec<f64> = (-20..=20)
            .filter_map(|i| {
                let mut c = start.clone();
                c[k] += i as f64 / 20.0;
                let t = Instant::now();
                d.set(&c).ok()?;
                Some(1e3 * t.elapsed().as_secs_f64())
            })
            .collect();
        steps.sort_by(f64::total_cmp);
        let p95 = steps[((steps.len() - 1) as f64 * 0.95).round() as usize];
        eprintln!(
            "  drag: begin {begin:.2} s, step p95 {p95:.1} ms ({} cells)",
            d.moved_cells().len()
        );
        Some((d, begin, p95))
    });
    drop(input);

    let region = model.extent(Scope::Contents, 64);
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let inside =
        |r: &mut Rng| [0, 1, 2].map(|i| region.lo[i] + r.next() * (region.hi[i] - region.lo[i]));
    let points: Vec<[f64; 3]> = (0..POINTS).map(|_| inside(&mut rng)).collect();
    let lines: Vec<Line> = (0..LINES)
        .map(|_| Line {
            origin: inside(&mut rng),
            dir: rng.direction(),
            t_min: 0.0,
            t_max: 100.0,
        })
        .collect();
    let single = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    // Best of 3, as the other timings: a single call right after loading
    // pays the first touches of the model's pages and swung by 2x run to run.
    let rate = |n: usize, f: &dyn Fn()| n as f64 / (1e-3 * best_ms(3, f));
    let points_s = rate(POINTS, &|| {
        model.classify(&points, Depth::All);
    });
    let p1 = &points[..POINTS / ONE_THREAD];
    let points_1 = rate(p1.len(), &|| {
        single.install(|| model.classify(p1, Depth::All));
    });
    let lines_s = rate(LINES, &|| {
        model.trace(&lines, Depth::All).unwrap();
    });
    let l1 = &lines[..LINES / ONE_THREAD];
    let lines_1 = rate(l1.len(), &|| {
        single.install(|| model.trace(l1, Depth::All).unwrap());
    });
    eprintln!("  points {points_s:.3e}/s, lines {lines_s:.3e}/s");

    let mid = 0.5 * (region.lo[2] + region.hi[2]);
    let slice = Slice::axis(Axis::Z, mid, &region, 1000);
    let slice_ms = best_ms(3, || {
        model.slice(&slice, Depth::All).unwrap();
    });
    let vis = Visibility::solid(&model);
    let camera = Camera::fit(&region, [0.6, 0.7, -0.4], 800, 600);
    let render_ms = best_ms(2, || {
        model.render(&camera, &vis).unwrap();
    });
    let t = Instant::now();
    model.fingerprints(DEFAULT_PRECISION).unwrap();
    let fingerprints = t.elapsed().as_secs_f64();
    eprintln!("  slice {slice_ms:.0} ms, render {render_ms:.0} ms");

    let longest = (0..3)
        .map(|i| region.hi[i] - region.lo[i])
        .fold(0.0, f64::max);
    let h = round_spacing(longest / 400.0);
    let t = Instant::now();
    let vols = model
        .volumes(&VolumeGrid::seeded(region, h, 1), None)
        .unwrap();
    let check = t.elapsed().as_secs_f64();
    let problems = vols.problems().len();
    eprintln!("  check at h = {h} cm: {check:.1} s, {problems} problem labels");

    let gpu_frame = gpu.map(|_| {
        // A device of its own, as a frontend has for one model: on one
        // device kept across models, uploads after a large model's took
        // ~9 s even for models of a few MB (0.05 s on a new device).
        let gpu = &llevant_gpu::Gpu::new().unwrap();
        let t = Instant::now();
        let gm = llevant_gpu::GpuModel::upload(gpu, &model).unwrap();
        let upload = t.elapsed().as_secs_f64();
        gm.set_visibility(gpu, &model, &vis).unwrap();
        let cam = Camera::fit(&region, [0.6, 0.7, -0.4], 1920, 1080);
        // Back to back after a warm-up, so the GPU keeps its clock (`gpu_time`).
        let mut timer = crate::gpu_time::FrameTimer::new();
        let best = timer.best(gpu, &gm, &cam, 4);
        eprintln!(
            "  GPU upload {upload:.1} s, frame {:.1} ms; {}",
            1e3 * best,
            timer.clock_report()
        );
        // A drag's step written into the upload (of a step 1 cm out).
        let patch = draft.as_mut().and_then(|(d, _, _)| {
            let mut c = d.coeffs().to_vec();
            *c.last_mut()? += 1.0;
            let p = d.set(&c).ok()?;
            let ms = best_ms(3, || {
                gm.patch(gpu, &p).unwrap();
                let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
            });
            eprintln!("  GPU drag step {ms:.1} ms");
            Some(ms)
        });
        json!({
            "upload_s": upload,
            "gpu_mb": gm.bytes as f64 / 1e6,
            "frame_1080p_ms": 1e3 * best,
            "drag_patch_ms": patch,
        })
    });

    json!({
        "model": name,
        "cells": model.cells().len(),
        "surfaces": model.stats().surfaces,
        "levels": model.levels(),
        "compile_s": compile,
        "compile_staged_s": staged,
        "recompile": recompile,
        "drag": draft.as_ref().map(|(d, begin, p95)| json!({
            "cells": d.moved_cells().len(),
            "begin_s": begin,
            "step_p95_ms": p95,
        })),
        "read_s": read,
        "saved_mb": saved_mb,
        "points_per_s": points_s,
        "points_per_s_1_thread": points_1,
        "lines_per_s": lines_s,
        "lines_per_s_1_thread": lines_1,
        "slice_1000_rows_ms": slice_ms,
        "render_800x600_ms": render_ms,
        "fingerprints_s": fingerprints,
        "check_h_cm": h,
        "check_s": check,
        "check_problem_labels": problems,
        "gpu": gpu_frame,
    })
}

const ROWS: [&str; 15] = [
    "Cells / compiled surfaces / levels",
    "Compile (no cache)",
    "Compile to a staged model (no octrees)",
    "Recompile, one surface moved: staged / complete (universes rebuilt)",
    "That surface dragged: start / step, 95th percentile (cells moved)",
    "Read the compiled model (cache)",
    "Points, all threads / one",
    "100 cm lines, all threads / one",
    "Slice, 1000 rows",
    "CPU render, 800 × 600",
    "Fingerprints",
    "Gap and overlap check",
    "GPU upload",
    "GPU frame, 1920 × 1080",
    "GPU: a drag's step written into the upload",
];

/// `input` with the last coefficient of the first surface of its last cell
/// moved by 1e-3 cm (an edit to one universe), if it has one, and that
/// surface.
fn moved_surface(input: &[u8]) -> Option<(i64, String)> {
    let mut m = migjorn::Model::parse(std::str::from_utf8(input).ok()?);
    let id = m
        .cells()
        .filter_map(|c| c.surface_refs().first().map(|s| s.id))
        .last()?;
    let c = m.surface(id)?.coeffs().ok()?;
    let k = c.len().checked_sub(1)?;
    m.surface_mut(id)?.set_coeff(k, c[k] + 1e-3).ok()?;
    Some((id, m.to_source()))
}

/// Row `i` of [`ROWS`] for model `x`.
fn row(i: usize, x: &Value) -> String {
    let f = |v: &Value| v.as_f64().unwrap_or(f64::NAN);
    let g = &x["gpu"];
    let r = &x["recompile"];
    match i {
        0 => format!("{} / {} / {}", x["cells"], x["surfaces"], x["levels"]),
        1 => format!("{:.2} s", f(&x["compile_s"])),
        2 => format!("{:.2} s", f(&x["compile_staged_s"])),
        3 if !r.is_null() => format!(
            "{:.2} s / {:.2} s ({})",
            f(&r["staged_s"]),
            f(&r["complete_s"]),
            r["universes_rebuilt"]
        ),
        4 if !x["drag"].is_null() => format!(
            "{:.0} ms / {:.1} ms ({})",
            1e3 * f(&x["drag"]["begin_s"]),
            f(&x["drag"]["step_p95_ms"]),
            x["drag"]["cells"]
        ),
        5 => format!("{:.2} s ({:.0} MB)", f(&x["read_s"]), f(&x["saved_mb"])),
        6 => format!(
            "{} / {} per s",
            sci(f(&x["points_per_s"])),
            sci(f(&x["points_per_s_1_thread"]))
        ),
        7 => format!(
            "{} / {} per s",
            sci(f(&x["lines_per_s"])),
            sci(f(&x["lines_per_s_1_thread"]))
        ),
        8 => format!("{:.0} ms", f(&x["slice_1000_rows_ms"])),
        9 => format!("{:.0} ms", f(&x["render_800x600_ms"])),
        10 => format!("{:.2} s", f(&x["fingerprints_s"])),
        11 => format!("{:.1} s (h = {} cm)", f(&x["check_s"]), x["check_h_cm"]),
        12 if !g.is_null() => format!("{:.1} s ({:.0} MB)", f(&g["upload_s"]), f(&g["gpu_mb"])),
        13 if !g.is_null() => format!("{:.0} ms", f(&g["frame_1080p_ms"])),
        14 if !g["drag_patch_ms"].is_null() => format!("{:.1} ms", f(&g["drag_patch_ms"])),
        _ => "—".into(),
    }
}

/// The page's tables.
fn markdown(r: &Value) -> String {
    let m = &r["machine"];
    let mut s = format!(
        "Measured {} at commit `{}`, `{}` profile, on {} ({} threads, {}){}.\n\n",
        r["date"].as_str().unwrap_or("?"),
        r["commit"].as_str().unwrap_or("?"),
        r["profile"].as_str().unwrap_or("?"),
        m["cpu"].as_str().unwrap_or("?"),
        m["threads"],
        m["os"].as_str().unwrap_or("?"),
        m["gpu"]
            .as_str()
            .map_or(String::new(), |g| format!(", GPU {g}")),
    );
    let models = r["models"].as_array().unwrap();
    s.push_str("| |");
    for x in models {
        s.push_str(&format!(" {} |", x["model"].as_str().unwrap()));
    }
    s.push_str("\n|---|");
    s.push_str(&"---|".repeat(models.len()));
    s.push('\n');
    for (i, name) in ROWS.iter().enumerate() {
        s.push_str(&format!("| {name} |"));
        for x in models {
            s.push_str(&format!(" {} |", row(i, x)));
        }
        s.push('\n');
    }
    s
}

fn sci(x: f64) -> String {
    let e = x.log10().floor();
    format!("{:.2}e{}", x / 10f64.powf(e), e as i32)
}

fn best_ms(runs: usize, mut f: impl FnMut()) -> f64 {
    (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            1e3 * t.elapsed().as_secs_f64()
        })
        .fold(f64::INFINITY, f64::min)
}

/// 1, 2 or 5 times a power of ten, at most `x`.
fn round_spacing(x: f64) -> f64 {
    let p = 10f64.powf(x.log10().floor());
    [5.0, 2.0, 1.0]
        .into_iter()
        .map(|k| k * p)
        .find(|&v| v <= x)
        .unwrap_or(p)
}

fn machine(gpu: Option<&llevant_gpu::Gpu>) -> Value {
    let cpu = std::env::var("PROCESSOR_IDENTIFIER").ok().or_else(|| {
        std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|v| v.trim().to_string())
        })
    });
    json!({
        "cpu": cpu,
        "threads": rayon::current_num_threads(),
        "os": std::env::consts::OS,
        "gpu": gpu.map(|g| g.info.name.clone()),
    })
}

fn git(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Today's date (UTC), as YYYY-MM-DD.
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    // Days to civil date (Howard Hinnant's algorithm).
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}
