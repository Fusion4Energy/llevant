//! Check llevant against MCNP itself.
//!
//! ```text
//! mcnp-oracle check <model> [--points N] [--sample envelopes|cells|world] [--seed S]
//!                           [--mcnp PATH] [--work DIR]
//!                           [--golden FILE [--golden-rows N]] [--dump FILE]
//!                           [--extra CARD]  (an extra data card, e.g. "cut:n 100")
//!                           [--only-lattice x]  (only points inside lattice elements)
//! mcnp-oracle submodel <in> <out> [--keep N] [--seed S] [--keep-universe U,U...]
//! ```
//!
//! `check` samples points, classifies them with llevant, and asks MCNP for
//! the cell of every point llevant finds uniquely inside a cell: each point is
//! a source position (`SDEF POS=D1` with an `L` list), identified in the PTRAC
//! source events by an energy that encodes its index. MCNP aborts a run when
//! a source point is in no cell; the batch is then bisected to find the
//! points concerned, which count as disagreements.
//!
//! `submodel` keeps every level-0 cell of a model but only `N` randomly
//! chosen fills (the other filled cells become empty), plus the universes,
//! surfaces and data those need: MCNP takes far too long to set up the
//! largest models for routine checks.

use llevant::Depth;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use llevant::Aabb;
use llevant::{Model, Options, Status};

const DEFAULT_MCNP: &str = "mcnp6";
/// Points per MCNP run: the energy `1 + (i + 1)·1e-4` must stay below 10 MeV
/// to survive PTRAC's 5-digit output.
const BATCH: usize = 80_000;

struct Args {
    rest: Vec<String>,
    opts: BTreeMap<String, String>,
}

fn args() -> Args {
    let mut rest = Vec::new();
    let mut opts = BTreeMap::new();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        if let Some(k) = a.strip_prefix("--") {
            opts.insert(k.to_string(), it.next().unwrap_or_default());
        } else {
            rest.push(a);
        }
    }
    Args { rest, opts }
}

fn main() {
    let a = args();
    match a.rest.first().map(String::as_str) {
        Some("check") if a.rest.len() == 2 => check(&a),
        Some("submodel") if a.rest.len() == 3 => submodel(&a),
        _ => {
            eprintln!("usage: mcnp-oracle check <model> [--points N] [--sample envelopes|cells|world] [--seed S] [--mcnp PATH] [--work DIR]");
            eprintln!("       mcnp-oracle submodel <in> <out> [--keep N] [--seed S]");
            std::process::exit(2);
        }
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn read_text(path: &str) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    String::from_utf8_lossy(&bytes).into_owned()
}

// --- submodel -----------------------------------------------------------------

fn submodel(a: &Args) {
    let (src, dst) = (&a.rest[1], &a.rest[2]);
    let keep: usize = a.opts.get("keep").map_or(20, |s| s.parse().unwrap());
    let mut rng = Rng(a.opts.get("seed").map_or(1, |s| s.parse().unwrap()) | 1);
    let t = Instant::now();
    let mut m = migjorn::parse(&read_text(src));

    // Level-0 filled cells, in file order; keep a random subset, plus every
    // one filled with a universe listed in `--keep-universe`.
    let wanted: HashSet<i64> = a
        .opts
        .get("keep-universe")
        .map(|s| s.split(',').map(|u| u.trim().parse().unwrap()).collect())
        .unwrap_or_default();
    let filled: Vec<(u32, i64, i64)> = m
        .cells()
        .filter(|&c| universe(c) == 0)
        .filter_map(|c| Some((c.slot(), c.id().unwrap_or(0), single_fill(c)?)))
        .collect();
    let mut order: Vec<usize> = (0..filled.len()).collect();
    for i in (1..order.len()).rev() {
        order.swap(i, (rng.next() * (i + 1) as f64) as usize % (i + 1));
    }
    let mut kept: HashSet<usize> = order.into_iter().take(keep).collect();
    kept.extend((0..filled.len()).filter(|&i| wanted.contains(&filled[i].2)));
    for (i, &(slot, _, _)) in filled.iter().enumerate() {
        if !kept.contains(&i) {
            m.cell_mut_at(slot).unwrap().remove_param("fill").unwrap();
        }
    }

    // Universes reachable from the kept fills.
    let mut by_universe: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    let mut fill_of: BTreeMap<i64, i64> = BTreeMap::new();
    for c in m.cells() {
        let id = c.id().unwrap_or(0);
        by_universe.entry(universe(c).abs()).or_default().push(id);
        if let Some(u) = single_fill(c) {
            fill_of.insert(id, u);
        }
    }
    let mut reach: BTreeSet<i64> = BTreeSet::from([0]);
    let mut stack = vec![0i64];
    while let Some(u) = stack.pop() {
        for id in by_universe.get(&u).into_iter().flatten() {
            if let Some(&t) = fill_of.get(id) {
                if reach.insert(t) {
                    stack.push(t);
                }
            }
        }
    }
    let mut removed_cells = 0;
    for (u, ids) in &by_universe {
        if !reach.contains(u) {
            for id in ids {
                m.cell_mut(*id).map(|c| c.remove());
                removed_cells += 1;
            }
        }
    }

    // Surfaces still referenced (directly or through `#n`).
    let mut used: HashSet<i64> = HashSet::new();
    for c in m.cells() {
        used.extend(c.surface_ids());
        for r in c.cell_refs() {
            if let Some(rc) = m.cell(r) {
                used.extend(rc.surface_ids());
            }
        }
    }
    let unused: Vec<i64> = m
        .surfaces()
        .filter_map(|s| s.id())
        .filter(|id| !used.contains(id))
        .collect();
    for id in &unused {
        m.surface_mut(*id).map(|s| s.remove());
    }
    let out = mcnp_text(&m.to_source());
    std::fs::write(dst, out).unwrap();
    println!(
        "kept {} of {} fills, {} universes; removed {removed_cells} cells and {} surfaces in {:.1} s",
        kept.len(),
        filled.len(),
        reach.len(),
        unused.len(),
        t.elapsed().as_secs_f64()
    );
    for p in m.validate().iter().take(10) {
        println!("problem: {p}");
    }
}

/// A cell's `U=`; unreadable counts as level 0.
fn universe(c: migjorn::Cell) -> i64 {
    c.universe().ok().flatten().unwrap_or(0)
}

/// The universe of a single-universe `FILL`; `None` for none, an unreadable
/// one and the lattice-array form.
fn single_fill(c: migjorn::Cell) -> Option<i64> {
    match c.fill() {
        Ok(Some(migjorn::Fill::Single { universe, .. })) => Some(universe),
        _ => None,
    }
}

/// Make migjorn's output acceptable to MCNP 6.2: drop `$` comments from lines
/// longer than 128 columns, drop lines left empty by that (an empty line ends
/// a block), and wrap what is still too long onto continuation lines.
fn mcnp_text(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for raw in src.lines() {
        let line = raw.trim_end_matches('\r');
        if line.len() <= 128 {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let first = line.split_whitespace().next().unwrap_or("");
        if first.eq_ignore_ascii_case("c") && line.len() - line.trim_start().len() < 5 {
            out.push_str(&line[..line.floor_char_boundary(128)]);
            out.push('\n');
            continue;
        }
        let body = line.split('$').next().unwrap().trim_end();
        if body.trim().is_empty() {
            continue;
        }
        let mut rest = body;
        let mut cont = false;
        while rest.len() > if cont { 120 } else { 128 } {
            let lim = if cont { 120 } else { 128 };
            let cut = rest[..rest.floor_char_boundary(lim)]
                .rfind(' ')
                .unwrap_or(lim);
            let (head, tail) = rest.split_at(cut);
            if cont {
                out.push_str("     ");
                out.push_str(head.trim());
            } else {
                out.push_str(head);
            }
            out.push('\n');
            rest = tail;
            cont = true;
        }
        if cont {
            out.push_str("     ");
            out.push_str(rest.trim());
        } else {
            out.push_str(rest);
        }
        out.push('\n');
    }
    out
}

// --- check --------------------------------------------------------------------

fn check(a: &Args) {
    let path = &a.rest[1];
    let n: usize = a.opts.get("points").map_or(20_000, |s| s.parse().unwrap());
    let seed: u64 = a.opts.get("seed").map_or(7, |s| s.parse().unwrap());
    let sample = a.opts.get("sample").map_or("envelopes", String::as_str);
    let mcnp = a.opts.get("mcnp").map_or(DEFAULT_MCNP, String::as_str);
    let extra = a.opts.get("extra").map_or("", String::as_str);
    let work = PathBuf::from(
        a.opts
            .get("work")
            .cloned()
            .unwrap_or_else(|| "oracle-work".into()),
    );
    std::fs::create_dir_all(&work).unwrap();

    let text = read_text(path);
    let model = Model::from_text(&text, &Options::default());
    println!("llevant: {:?}", model.stats());
    for d in model.diagnostics().iter().take(10) {
        println!("diagnostic: {d}");
    }
    let mut points = sample_points(&model, n, sample, seed);
    if a.opts.contains_key("only-lattice") {
        // Keep only points that go through a lattice element, sampling more
        // until there are `n` of them (or giving up after 2000 rounds).
        let mut kept = Vec::new();
        for round in 0..2000u64 {
            let batch = sample_points(&model, 1_000_000, sample, seed.wrapping_add(round * 7919));
            let r = model.classify(&batch, Depth::All);
            kept.extend(batch.iter().zip(&r.ids).filter_map(|(p, &id)| {
                r.labels[id as usize]
                    .path
                    .iter()
                    .any(|s| s.lattice.is_some())
                    .then_some(*p)
            }));
            if kept.len() >= n {
                break;
            }
        }
        kept.truncate(n);
        points = kept;
    }
    let res = model.classify(&points, Depth::All);
    let mut statuses: BTreeMap<String, usize> = BTreeMap::new();
    let mut inside = Vec::new();
    for (i, &id) in res.ids.iter().enumerate() {
        let l = &res.labels[id as usize];
        *statuses.entry(format!("{:?}", l.status)).or_default() += 1;
        if l.status == Status::Inside {
            let c = &model.cells()[l.cell().unwrap() as usize];
            inside.push((i, c.id, c.material));
        }
    }
    println!("llevant statuses: {statuses:?}");
    let through_lattice = res
        .ids
        .iter()
        .filter(|&&id| {
            res.labels[id as usize]
                .path
                .iter()
                .any(|s| s.lattice.is_some())
        })
        .count();
    println!("points inside a lattice element: {through_lattice}");

    // MCNP input: the model's cells and surfaces, its TR and M cards, and our
    // source and PTRAC cards.
    let mut m = migjorn::parse(&text);
    let keep: HashSet<u32> = m
        .materials()
        .map(|v| v.slot())
        .chain(m.transforms().map(|v| v.slot()))
        .collect();
    let drop: Vec<u32> = m
        .data_cards()
        .map(|d| d.slot())
        .filter(|s| !keep.contains(s))
        .collect();
    for s in drop {
        m.remove_card(s);
    }
    let base = mcnp_text(&m.to_source());

    let mut runs = 0;
    let mut mcnp_cell: BTreeMap<usize, Option<(i64, i64)>> = BTreeMap::new();
    // Points in IMP=0 cells: MCNP will not start a particle there.
    let mut zero_imp: BTreeSet<usize> = BTreeSet::new();
    let t = Instant::now();
    let mut queue: Vec<Vec<(usize, i64, i64)>> = inside.chunks(BATCH).map(|c| c.to_vec()).collect();
    // Points MCNP did not report (random sampling, or a run cut short by an
    // MCNP crash) are resubmitted for a couple of rounds.
    for round in 0..3 {
        if round > 0 {
            let missing: Vec<(usize, i64, i64)> = inside
                .iter()
                .copied()
                .filter(|b| !mcnp_cell.contains_key(&b.0) && !zero_imp.contains(&b.0))
                .collect();
            if missing.is_empty() {
                break;
            }
            queue = missing.chunks(BATCH).map(|c| c.to_vec()).collect();
        }
        while let Some(batch) = queue.pop() {
            runs += 1;
            let pts: Vec<[f64; 3]> = batch.iter().map(|b| points[b.0]).collect();
            match run_mcnp(mcnp, &work, &base, &pts, extra) {
                Ok(found) => {
                    for (k, b) in batch.iter().enumerate() {
                        if let Some(c) = found.get(&k) {
                            mcnp_cell.insert(b.0, Some(*c));
                        }
                    }
                }
                Err(e @ (RunError::NoCell | RunError::ZeroImportance)) => {
                    if batch.len() == 1 {
                        if matches!(e, RunError::ZeroImportance) {
                            zero_imp.insert(batch[0].0);
                        } else {
                            mcnp_cell.insert(batch[0].0, None);
                        }
                    } else {
                        let (l, r) = batch.split_at(batch.len() / 2);
                        queue.push(l.to_vec());
                        queue.push(r.to_vec());
                    }
                }
                Err(RunError::Other(msg)) => {
                    eprintln!("MCNP failed: {msg}");
                    std::process::exit(1);
                }
            }
        }
    }
    println!("{runs} MCNP runs in {:.1} s", t.elapsed().as_secs_f64());

    let mut agree = 0;
    let mut bad = Vec::new();
    let mut unsampled = 0;
    for &(i, cell, mat) in &inside {
        if zero_imp.contains(&i) {
            continue;
        }
        match mcnp_cell.get(&i) {
            None => unsampled += 1,
            // The VOID card makes MCNP report material 0, so only cells are compared.
            Some(Some((c, _))) if *c == cell => agree += 1,
            Some(other) => bad.push((i, cell, mat, *other)),
        }
    }
    if let Some(golden) = a.opts.get("golden") {
        // Points MCNP confirmed, for llevant's own tests: x y z cell.
        let limit: usize = a
            .opts
            .get("golden-rows")
            .map_or(3000, |s| s.parse().unwrap());
        let mut out = String::from("# x y z cell (cell from MCNP PTRAC source events)\n");
        let rows = inside
            .iter()
            .filter(|(i, cell, _)| matches!(mcnp_cell.get(i), Some(Some((c, _))) if c == cell))
            .take(limit);
        for (i, cell, _) in rows {
            let p = points[*i];
            out.push_str(&format!(
                "{:.17e} {:.17e} {:.17e} {cell}\n",
                p[0], p[1], p[2]
            ));
        }
        std::fs::write(golden, out).unwrap();
    }
    println!(
        "compared {} points: {agree} agree, {} disagree, {unsampled} not sampled by MCNP, \
         {} in IMP=0 cells (MCNP starts no particle there)",
        inside.len() - unsampled - zero_imp.len(),
        bad.len(),
        zero_imp.len()
    );
    let mut by_cell: BTreeMap<(i64, Option<i64>), usize> = BTreeMap::new();
    for (_, cell, _, other) in &bad {
        *by_cell.entry((*cell, other.map(|o| o.0))).or_default() += 1;
    }
    if !by_cell.is_empty() {
        println!("disagreements by (llevant cell, MCNP cell): {by_cell:?}");
    }
    if let Some(dump) = a.opts.get("dump") {
        let mut out = String::from("# x y z llevant_cell mcnp_cell (0: no cell)\n");
        for (i, cell, _, other) in &bad {
            let p = points[*i];
            out.push_str(&format!(
                "{:.17e} {:.17e} {:.17e} {cell} {}\n",
                p[0],
                p[1],
                p[2],
                other.map_or(0, |o| o.0)
            ));
        }
        std::fs::write(dump, out).unwrap();
    }
    for (i, cell, mat, other) in bad.iter().take(20) {
        let p = points[*i];
        let label = &res.labels[res.ids[*i] as usize];
        let path: Vec<i64> = label
            .path
            .iter()
            .map(|s| model.cells()[s.cell as usize].id)
            .collect();
        match other {
            Some((c, m)) => println!(
                "  {p:?}: llevant cell {cell} (mat {mat}, path {path:?}), MCNP cell {c} (mat {m})"
            ),
            None => println!("  {p:?}: llevant cell {cell} (path {path:?}), MCNP: in no cell"),
        }
    }
    if !bad.is_empty() {
        std::process::exit(1);
    }
}

fn sample_points(model: &Model, n: usize, how: &str, seed: u64) -> Vec<[f64; 3]> {
    let finite_small = |b: &Aabb| b.is_finite() && (0..3).all(|i| b.hi[i] - b.lo[i] < 1e5);
    let boxes: Vec<Aabb> = match how {
        "envelopes" => model.units()[0]
            .cells
            .iter()
            .map(|&c| &model.cells()[c as usize])
            .filter(|c| c.fill.is_some() && finite_small(&c.bbox))
            .map(|c| c.bbox)
            .collect(),
        "cells" => model.units()[0]
            .cells
            .iter()
            .map(|&c| &model.cells()[c as usize])
            .filter(|c| !c.imp_zero && finite_small(&c.bbox))
            .map(|c| c.bbox)
            .collect(),
        "world" => {
            let mut w = Aabb::EMPTY;
            for &c in &model.units()[0].cells {
                let c = &model.cells()[c as usize];
                if !c.imp_zero && finite_small(&c.bbox) {
                    w = w.union(&c.bbox);
                }
            }
            vec![w]
        }
        other => panic!("unknown sampling `{other}`"),
    };
    assert!(!boxes.is_empty(), "nothing to sample in");
    let mut rng = Rng(seed.wrapping_mul(0x9e3779b97f4a7c15) | 1);
    (0..n)
        .map(|_| {
            let b = boxes[(rng.next() * boxes.len() as f64) as usize % boxes.len()];
            [0, 1, 2].map(|i| b.lo[i] + rng.next() * (b.hi[i] - b.lo[i]))
        })
        .collect()
}

enum RunError {
    NoCell,
    /// A point in an IMP=0 cell: MCNP stops the run part way ("bad trouble").
    ZeroImportance,
    Other(String),
}

/// Run MCNP with one source point per entry of `pts`; returns the (cell,
/// material) of each point index that MCNP sampled.
fn run_mcnp(
    mcnp: &str,
    work: &Path,
    base: &str,
    pts: &[[f64; 3]],
    extra: &str,
) -> Result<BTreeMap<usize, (i64, i64)>, RunError> {
    for f in ["inp", "run.o", "run.p", "run.r", "run.s"] {
        let _ = std::fs::remove_file(work.join(f));
    }
    let mut cards = String::new();
    let wrap = |cards: &mut String, head: &str, items: &mut dyn Iterator<Item = String>| {
        let mut line = String::from(head);
        for it in items {
            if line.len() + it.len() + 1 > 78 {
                cards.push_str(&line);
                cards.push('\n');
                line = String::from("     ");
            }
            line.push(' ');
            line.push_str(&it);
        }
        cards.push_str(&line);
        cards.push('\n');
    };
    wrap(
        &mut cards,
        "si1 L",
        &mut pts
            .iter()
            .map(|p| format!("{:.15e} {:.15e} {:.15e}", p[0], p[1], p[2])),
    );
    cards.push_str(&format!("sp1 1 {}r\n", pts.len() - 1));
    wrap(
        &mut cards,
        "ds2 L",
        &mut (0..pts.len()).map(|i| format!("{:.4}", energy(i))),
    );
    if !extra.is_empty() {
        cards.push_str(extra);
        cards.push('\n');
    }
    let nps = 4 * pts.len() + 10;
    let input = format!(
        "{}mode n\nvoid\nvol no\nsdef pos=d1 erg=fpos=d2\n{cards}nps {nps}\nlost 1000000 10\nptrac file=asc event=src write=all max={}\n",
        base.trim_end().to_string() + "\n",
        nps + 1
    );
    std::fs::write(work.join("inp"), input).unwrap();
    let out = Command::new(mcnp)
        .args(["i=inp", "n=run."])
        .current_dir(work)
        .output()
        .map_err(|e| RunError::Other(format!("cannot run {mcnp}: {e}")))?;
    let log =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    if log.contains("source is not in any cell") {
        return Err(RunError::NoCell);
    }
    let output = std::fs::read_to_string(work.join("run.o")).unwrap_or_default();
    // A point in an IMP=0 cell stops the run part way ("bad trouble"):
    // bisected like a point in no cell, then left out of the comparison.
    if output.contains("source particle started in a cell of zero importance") {
        return Err(RunError::ZeroImportance);
    }
    let fatal: Vec<&str> = output
        .lines()
        .filter(|l| l.contains("fatal error"))
        .take(10)
        .collect();
    if !fatal.is_empty() {
        return Err(RunError::Other(fatal.join("\n")));
    }
    let ptrac = std::fs::read_to_string(work.join("run.p")).map_err(|_| {
        RunError::Other(format!(
            "no PTRAC file; MCNP said:\n{}",
            log.lines().rev().take(15).collect::<Vec<_>>().join("\n")
        ))
    })?;
    let mut found = BTreeMap::new();
    let lines: Vec<&str> = ptrac.lines().collect();
    for w in lines.windows(2) {
        let ints: Vec<i64> = w[0]
            .split_whitespace()
            .filter_map(|t| t.parse().ok())
            .collect();
        if ints.len() != 6 || w[0].split_whitespace().count() != 6 {
            continue;
        }
        let reals: Vec<f64> = w[1]
            .split_whitespace()
            .filter_map(|t| t.parse().ok())
            .collect();
        if reals.len() != 9 {
            continue;
        }
        let i = ((reals[6] - 1.0) / 1e-4).round() as i64 - 1;
        if i >= 0 && (i as usize) < pts.len() {
            found.insert(i as usize, (ints[3], ints[4]));
        }
    }
    Ok(found)
}

fn energy(i: usize) -> f64 {
    1.0 + (i as f64 + 1.0) * 1e-4
}
