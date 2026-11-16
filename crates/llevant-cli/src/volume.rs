//! `volume` and `check`: volumes of cells, materials or universes with their
//! standard error, and the gaps and overlaps of a model.

use std::path::PathBuf;
use std::time::Instant;

use llevant::{Model, Scope, Volume, VolumeGrid};
use serde_json::{json, Value};

use crate::args::{Format, GridArgs, GroupBy, Open};
use crate::common::{csv_field, err, load, region};
use crate::json;

#[derive(clap::Args)]
pub(crate) struct VolumeArgs {
    /// The MCNP input file.
    model: PathBuf,
    /// Line spacing in cm: as many lines as three axis-parallel
    /// families this far apart.
    #[arg(long)]
    h: f64,
    #[command(flatten)]
    grid: GridArgs,
    /// Refine the groups with a number (cells, materials, universes;
    /// every inside label with --by label) until sigma / volume is at
    /// most this, halving h over the boxes their cells can be in. Each
    /// row gets a status: converged, not converged, not found (no line
    /// met it down to --h-min) or empty (its boxes are empty: volume 0).
    #[arg(long)]
    rel_sigma: Option<f64>,
    /// Smallest h to refine to with --rel-sigma, cm (default h / 64).
    #[arg(long)]
    h_min: Option<f64>,
    /// With --rel-sigma: a group whose sigma is at most this many cm3 is
    /// done too, however large its relative sigma (small cells would
    /// otherwise need lines a fraction of their size apart). Default 0:
    /// none.
    #[arg(long, default_value_t = 0.0)]
    abs_sigma: f64,
    /// With --rel-sigma: probes a group may have at one spacing. Groups a
    /// little above the target get more probes there when that is
    /// cheaper than a finer spacing: more converge, for more time.
    /// Default: --probes (none added) for the whole model, 48 with --cells.
    #[arg(long)]
    max_probes: Option<usize>,
    /// Only these cells (MCNP numbers, comma separated), every instance
    /// of each together, lines traced around them alone; a filled
    /// cell's volume is that of its region. --h is then the largest
    /// spacing: each cell starts at the one that puts 8 lines across
    /// its box, then halves it towards --rel-sigma.
    #[arg(long, value_delimiter = ',')]
    cells: Vec<i64>,
    #[arg(long, value_enum, default_value = "cell")]
    by: GroupBy,
    /// Output: a text table, CSV, or JSON (its fields: docs/guide/cli.md,
    /// "Scripting and agents";
    /// https://fusion4energy.github.io/llevant/guide/cli/#scripting-and-agents).
    #[arg(long, value_enum, default_value = "text")]
    pub(crate) format: Format,
}

impl VolumeArgs {
    pub(crate) fn run(self, open: &Open) -> Result<u8, String> {
        let VolumeArgs {
            model,
            h,
            grid,
            rel_sigma,
            h_min,
            abs_sigma,
            max_probes,
            cells,
            by,
            format,
        } = self;
        let m = load(&model, open)?;
        let h_min = h_min.unwrap_or(h / 64.0);
        if !cells.is_empty() {
            if by != GroupBy::Cell {
                return Err("--cells gives volumes by cell: --by must be cell".into());
            }
            let max_probes = max_probes.unwrap_or(llevant::MAX_PROBES);
            let c = cell_volumes(
                &m, &cells, h, h_min, rel_sigma, abs_sigma, max_probes, &grid,
            )?;
            return Ok(match rel_sigma {
                None => {
                    print_volumes(&m, by, &c.groups, None, format);
                    0
                }
                Some(rel_sigma) => {
                    print_volumes(&m, by, &c.groups, Some(&c), format);
                    report_converged(&c, rel_sigma, h_min)
                }
            });
        }
        let Some(rel_sigma) = rel_sigma else {
            let v = volumes(&m, h, &grid)?;
            print_volumes(&m, by, &v.grouped(&m, by.into()), None, format);
            return Ok(0);
        };
        let target = llevant::VolumeTarget {
            rel_sigma,
            h_min,
            abs_sigma,
            max_probes: max_probes.unwrap_or(grid.probes),
            by: by.into(),
        };
        let c = converged_volumes(&m, h, &grid, &target)?;
        print_volumes(&m, by, &c.groups, Some(&c), format);
        Ok(report_converged(&c, rel_sigma, h_min))
    }
}

#[derive(clap::Args)]
pub(crate) struct CheckArgs {
    /// The MCNP input file.
    model: PathBuf,
    /// Line spacing in cm.
    #[arg(long)]
    h: f64,
    #[command(flatten)]
    grid: GridArgs,
    /// Output: a text table, CSV, or JSON (its fields: docs/guide/cli.md,
    /// "Scripting and agents";
    /// https://fusion4energy.github.io/llevant/guide/cli/#scripting-and-agents).
    #[arg(long, value_enum, default_value = "text")]
    pub(crate) format: Format,
}

impl CheckArgs {
    pub(crate) fn run(self, open: &Open) -> Result<u8, String> {
        let CheckArgs {
            model,
            h,
            grid,
            format,
        } = self;
        let m = load(&model, open)?;
        let v = volumes(&m, h, &grid)?;
        let found = check(&m, &v, format);
        Ok(if found { 1 } else { 0 })
    }
}

fn volumes(m: &Model, h: f64, g: &GridArgs) -> Result<llevant::Volumes, String> {
    if !(h > 0.0 && h.is_finite()) {
        return Err("--h must be positive".into());
    }
    let grid = grid(m, h, g)?;
    eprintln!("tracing up to {} lines (h = {h} cm)", grid.line_count());
    let t = Instant::now();
    let v = m.volumes(&grid, None).map_err(err)?;
    eprintln!("traced in {:.1} s", t.elapsed().as_secs_f64());
    Ok(v)
}

fn grid(m: &Model, h: f64, g: &GridArgs) -> Result<VolumeGrid, String> {
    if !(h > 0.0 && h.is_finite()) {
        return Err("--h must be positive".into());
    }
    if g.probes < 2 {
        return Err("--probes must be at least 2".into());
    }
    Ok(VolumeGrid {
        probes: g.probes,
        ..VolumeGrid::seeded(region(m, &g.region, Scope::Problem)?, h, g.seed)
    })
}

fn converged_volumes(
    m: &Model,
    h: f64,
    g: &GridArgs,
    target: &llevant::VolumeTarget,
) -> Result<llevant::Converged, String> {
    if !(target.rel_sigma > 0.0 && target.h_min > 0.0 && target.abs_sigma >= 0.0) {
        return Err("--rel-sigma and --h-min must be positive, --abs-sigma not negative".into());
    }
    let grid = grid(m, h, g)?;
    eprintln!(
        "tracing up to {} lines (h = {h} cm), then refining to sigma / volume <= {}",
        grid.line_count(),
        target.rel_sigma
    );
    let t = Instant::now();
    let c = m
        .volumes_to(
            &grid,
            target,
            |g| g.status == llevant::Status::Inside,
            None,
            None,
        )
        .map_err(err)?;
    eprintln!("traced in {:.1} s", t.elapsed().as_secs_f64());
    Ok(c)
}

/// The messages on stderr for groups that did not reach the target, were
/// not found or are empty, and the exit status: 1 if some did not reach it.
fn report_converged(c: &llevant::Converged, rel_sigma: f64, h_min: f64) -> u8 {
    let names = |ix: &mut dyn Iterator<Item = usize>| {
        ix.take(20)
            .map(|i| c.groups[i].name.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let found: Vec<usize> = c
        .unconverged
        .iter()
        .copied()
        .filter(|i| !c.unseen.contains(i))
        .collect();
    if !found.is_empty() {
        eprintln!(
            "{} did not reach sigma / volume <= {rel_sigma} by h = {}: {}",
            found.len(),
            h_min,
            names(&mut found.iter().copied())
        );
    }
    if !c.unseen.is_empty() {
        eprintln!(
            "{} not found by h = {} (their boxes may hold something): {}",
            c.unseen.len(),
            h_min,
            names(&mut c.unseen.iter().copied())
        );
    }
    if !c.empty.is_empty() {
        eprintln!(
            "{} empty (their boxes are empty in the region): {}",
            c.empty.len(),
            names(&mut c.empty.iter().copied())
        );
    }
    if !c.listed_all {
        eprintln!(
            "more than {} instances: only those some line met are listed",
            llevant::MAX_LISTED_INSTANCES
        );
    }
    if c.unconverged.is_empty() {
        0
    } else {
        1
    }
}

/// The volumes of `cells` (MCNP numbers) alone, refined to `rel_sigma` if
/// given (else one pass at the first spacing every cell starts at).
#[allow(clippy::too_many_arguments)]
fn cell_volumes(
    m: &Model,
    cells: &[i64],
    h: f64,
    h_min: f64,
    rel_sigma: Option<f64>,
    abs_sigma: f64,
    max_probes: usize,
    g: &GridArgs,
) -> Result<llevant::Converged, String> {
    let grid = grid(m, h, g)?;
    if !(h_min > 0.0 && h_min <= h && abs_sigma >= 0.0) || rel_sigma.is_some_and(|r| !(r > 0.0)) {
        return Err(
            "--rel-sigma must be positive, --abs-sigma not negative and --h-min in (0, --h]".into(),
        );
    }
    let idx = cells
        .iter()
        .map(|&c| m.cell_index(c).ok_or_else(|| format!("no cell {c}")))
        .collect::<Result<Vec<u32>, String>>()?;
    let target = llevant::BinTarget {
        h_max: h,
        abs_sigma,
        max_probes,
        seed: grid.seed,
        probes: grid.probes,
        ..llevant::BinTarget::new(grid.region, rel_sigma.unwrap_or(f64::INFINITY), h_min)
    };
    eprintln!(
        "tracing around {} cells (h from {h} cm down to {h_min} cm)",
        idx.len()
    );
    let t = Instant::now();
    let c = m.cell_volumes(&idx, &target, None).map_err(err)?;
    eprintln!("traced in {:.1} s", t.elapsed().as_secs_f64());
    Ok(c)
}

/// Groups as a table; with `c`, the `h` each came from and whether it
/// reached the target.
fn print_volumes(
    m: &Model,
    by: GroupBy,
    groups: &[llevant::Group],
    c: Option<&llevant::Converged>,
    format: Format,
) {
    let name = by.name();
    let unconverged = |i: usize| c.is_some_and(|c| c.unconverged.binary_search(&i).is_ok());
    let status = |c: &llevant::Converged, i: usize| {
        if c.empty.binary_search(&i).is_ok() {
            "empty"
        } else if c.unseen.binary_search(&i).is_ok() {
            "not found"
        } else if unconverged(i) {
            "not converged"
        } else {
            "converged"
        }
    };
    match format {
        Format::Text => {
            print!(
                "{name:>12} {:>14} {:>12} {:>9}",
                "volume (cm3)", "sigma", "rel"
            );
            println!("{}", if c.is_some() { "        h" } else { "" });
            for (i, g) in groups.iter().enumerate() {
                let vol = &g.volume;
                print!(
                    "{:>12} {:>14.6e} {:>12.3e} {:>9.2e}",
                    g.name,
                    vol.estimate,
                    vol.sigma,
                    vol.sigma / vol.estimate
                );
                match c {
                    Some(c) => println!(
                        " {:>8}{}",
                        c.h[i],
                        match status(c, i) {
                            "converged" => String::new(),
                            s => format!("  {s}"),
                        }
                    ),
                    None => println!(),
                }
            }
            let total: f64 = groups.iter().map(|g| g.volume.estimate).sum();
            println!("{:>12} {total:>14.6e}", "total");
        }
        Format::Csv => {
            print!("{name},volume,sigma,segments,probes_hit");
            println!(
                "{}",
                if c.is_some() {
                    ",h,converged,status"
                } else {
                    ""
                }
            );
            for (i, g) in groups.iter().enumerate() {
                let vol = &g.volume;
                print!(
                    "{},{},{},{},{}",
                    csv_field(&g.name),
                    vol.estimate,
                    vol.sigma,
                    vol.segments,
                    vol.probes_hit()
                );
                match c {
                    Some(c) => println!(",{},{},{}", c.h[i], !unconverged(i), status(c, i)),
                    None => println!(),
                }
            }
        }
        Format::Json => {
            let rows: Vec<Value> = groups
                .iter()
                .enumerate()
                .map(|(i, g)| {
                    // A group is a sum over labels: its number (null for
                    // gaps, overlaps... and with --by label), and its name;
                    // a group that is one label has the label too.
                    let mut r = volume_json(&g.volume);
                    if name != "label" {
                        r[name] = json!(g.number);
                    }
                    r["name"] = json!(g.name);
                    r["status"] = json!(g.status.name());
                    r["label"] = g.label.as_ref().map_or(Value::Null, |l| json::label(m, l));
                    if let Some(c) = c {
                        r["h"] = json!(c.h[i]);
                        r["converged"] = json!(!unconverged(i));
                        r["outcome"] = json!(status(c, i));
                    }
                    r
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&rows).expect("json"));
        }
    }
}

fn volume_json(v: &Volume) -> Value {
    json!({
        "volume": v.estimate,
        "sigma": v.sigma,
        "segments": v.segments,
        "probes_hit": v.probes_hit(),
        "by_probe": v.by_probe,
        "box": json::bbox(&v.bbox),
    })
}

/// Print gaps and overlaps; true if there are any.
fn check(m: &Model, v: &llevant::Volumes, format: Format) -> bool {
    let p = v.problems();
    match format {
        Format::Text => {
            if p.is_empty() {
                println!("no gaps or overlaps found (h = {} cm)", v.grid.h);
            }
            for &i in &p {
                let (l, vol) = (&v.labels[i], &v.volumes[i]);
                println!(
                    "{}: {:.4e} ± {:.1e} cm3 in {:.3?} .. {:.3?}",
                    m.describe(l),
                    vol.estimate,
                    vol.sigma,
                    vol.bbox.lo,
                    vol.bbox.hi
                );
            }
            if !m.diagnostics().is_empty() {
                println!(
                    "({} compile diagnostics; see llevant info)",
                    m.diagnostics().len()
                );
            }
        }
        Format::Csv => {
            println!("status,label,volume,sigma,x0,y0,z0,x1,y1,z1");
            for &i in &p {
                let (l, vol) = (&v.labels[i], &v.volumes[i]);
                let (a, b) = (vol.bbox.lo, vol.bbox.hi);
                println!(
                    "{},{},{},{},{},{},{},{},{},{}",
                    l.status.name(),
                    csv_field(&m.describe(l)),
                    vol.estimate,
                    vol.sigma,
                    a[0],
                    a[1],
                    a[2],
                    b[0],
                    b[1],
                    b[2]
                );
            }
        }
        Format::Json => {
            let rows: Vec<Value> = p
                .iter()
                .map(|&i| {
                    let mut r = volume_json(&v.volumes[i]);
                    r["label"] = json::label(m, &v.labels[i]);
                    r
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&rows).expect("json"));
        }
    }
    !p.is_empty()
}
