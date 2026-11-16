//! `locate` and `trace`: where points are, and what lines cross.

use std::path::PathBuf;

use llevant::{Depth, Line, Model};
use serde_json::{json, Value};

use crate::args::{Format, Open};
use crate::common::{csv_field, err, load, numbers, surface_name, vec3};
use crate::json;

#[derive(clap::Args)]
pub(crate) struct LocateArgs {
    /// The MCNP input file.
    model: PathBuf,
    /// Coordinates: x y z [x y z ...].
    #[arg(allow_negative_numbers = true)]
    coords: Vec<f64>,
    /// Read points from a file instead (`-`: stdin), one `x y z` (or
    /// `x,y,z`) per line; blank lines and lines starting with `#` are skipped.
    #[arg(long)]
    points: Option<PathBuf>,
    /// Paths down to universe level N at most (0: the top level, a
    /// filled cell there ends the path).
    #[arg(long)]
    level: Option<usize>,
    /// Output: a text table, CSV, or JSON (its fields: docs/guide/cli.md,
    /// "Scripting and agents";
    /// https://fusion4energy.github.io/llevant/guide/cli/#scripting-and-agents).
    #[arg(long, value_enum, default_value = "text")]
    pub(crate) format: Format,
}

impl LocateArgs {
    pub(crate) fn run(self, open: &Open) -> Result<u8, String> {
        let LocateArgs {
            model,
            coords,
            points,
            level,
            format,
        } = self;
        let m = load(&model, open)?;
        let pts = match points {
            Some(f) => read_points(&f)?,
            None => {
                if coords.is_empty() || coords.len() % 3 != 0 {
                    return Err("locate: give x y z triples or --points".into());
                }
                coords.chunks(3).map(|c| [c[0], c[1], c[2]]).collect()
            }
        };
        locate(&m, &pts, level, format)
    }
}

#[derive(clap::Args)]
pub(crate) struct TraceArgs {
    /// The MCNP input file.
    model: PathBuf,
    /// Start point x,y,z.
    #[arg(
        long,
        allow_hyphen_values = true,
        requires = "to",
        conflicts_with = "lines"
    )]
    from: Option<String>,
    /// End point x,y,z.
    #[arg(long, allow_hyphen_values = true, requires = "from")]
    to: Option<String>,
    /// Trace many lines instead, read from a file (`-`: stdin): one
    /// `x0 y0 z0 x1 y1 z1` (or comma separated) per line, from the first
    /// point to the second; blank lines and lines starting with `#` are
    /// skipped.
    #[arg(long, conflicts_with = "to")]
    lines: Option<PathBuf>,
    /// Output: a text table, CSV, or JSON (its fields: docs/guide/cli.md,
    /// "Scripting and agents";
    /// https://fusion4energy.github.io/llevant/guide/cli/#scripting-and-agents).
    #[arg(long, value_enum, default_value = "text")]
    pub(crate) format: Format,
}

impl TraceArgs {
    pub(crate) fn run(self, open: &Open) -> Result<u8, String> {
        let TraceArgs {
            model,
            from,
            to,
            lines,
            format,
        } = self;
        let ends: Vec<[[f64; 3]; 2]> = match (from, to, lines) {
            (Some(a), Some(b), None) => {
                let (a, b) = (vec3(&a, "--from")?, vec3(&b, "--to")?);
                if a == b {
                    return Err("trace: --from and --to are the same point".into());
                }
                vec![[a, b]]
            }
            (None, None, Some(f)) => {
                let rows = read_rows::<6>(&f)?;
                let mut ends = Vec::with_capacity(rows.len());
                for (k, r) in rows {
                    let (a, b) = ([r[0], r[1], r[2]], [r[3], r[4], r[5]]);
                    if a == b {
                        return Err(format!(
                            "{}:{k}: the line's two ends are the same point",
                            f.display()
                        ));
                    }
                    ends.push([a, b]);
                }
                if ends.is_empty() {
                    return Err(format!("{}: no lines", f.display()));
                }
                ends
            }
            _ => return Err("trace: give --from and --to, or --lines".into()),
        };
        let m = load(&model, open)?;
        trace(&m, &ends, format)
    }
}

/// The rows of `n` numbers in a file (`-`: stdin; spaces or commas between
/// them), with their 1-based line numbers; blank lines and `#` comments are
/// skipped.
fn read_rows<const N: usize>(f: &PathBuf) -> Result<Vec<(usize, [f64; N])>, String> {
    let text = if f.as_os_str() == "-" {
        std::io::read_to_string(std::io::stdin()).map_err(|e| format!("stdin: {e}"))?
    } else {
        std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?
    };
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|(k, l)| {
            let v = numbers(l, N, &format!("{}:{}", f.display(), k + 1))?;
            Ok((k + 1, v.try_into().expect("N numbers")))
        })
        .collect()
}

fn read_points(f: &PathBuf) -> Result<Vec<[f64; 3]>, String> {
    Ok(read_rows::<3>(f)?.into_iter().map(|(_, p)| p).collect())
}

fn locate(m: &Model, pts: &[[f64; 3]], level: Option<usize>, format: Format) -> Result<u8, String> {
    let res = m.classify(pts, Depth::from(level));
    let row = |k: usize| {
        let l = &res.labels[res.ids[k] as usize];
        (l, l.cell().map(|c| m.cell(c)))
    };
    // The instance of each label's cell, by label.
    let instances: Vec<Option<u64>> = res.labels.iter().map(|l| m.instance(l)).collect();
    let instance = |k: usize| instances[res.ids[k] as usize];
    match format {
        Format::Text => {
            for (k, p) in pts.iter().enumerate() {
                let (l, cell) = row(k);
                match cell {
                    Some(c) => println!(
                        "{} {} {}: {}  material {} density {}{}",
                        p[0],
                        p[1],
                        p[2],
                        m.describe(l),
                        c.material,
                        c.density,
                        instance(k).map_or(String::new(), |i| format!("  instance {i}"))
                    ),
                    None => println!("{} {} {}: {}", p[0], p[1], p[2], m.describe(l)),
                }
            }
        }
        Format::Csv => {
            println!("x,y,z,status,cell,material,density,label,instance");
            for (k, p) in pts.iter().enumerate() {
                let (l, cell) = row(k);
                let (id, mat, rho) =
                    cell.map_or((String::new(), String::new(), String::new()), |c| {
                        (
                            c.id.to_string(),
                            c.material.to_string(),
                            c.density.to_string(),
                        )
                    });
                println!(
                    "{},{},{},{},{id},{mat},{rho},{},{}",
                    p[0],
                    p[1],
                    p[2],
                    l.status.name(),
                    csv_field(&m.describe(l)),
                    instance(k).map_or(String::new(), |i| i.to_string())
                );
            }
        }
        Format::Json => {
            let rows: Vec<Value> = pts
                .iter()
                .enumerate()
                .map(|(k, p)| {
                    let mut r = json::label(m, &res.labels[res.ids[k] as usize]);
                    r["point"] = json!(p);
                    r
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&rows).expect("json"));
        }
    }
    Ok(0)
}

/// Trace each line `[from, to]`, in one batch.
fn trace(m: &Model, ends: &[[[f64; 3]; 2]], format: Format) -> Result<u8, String> {
    let lines: Vec<Line> = ends.iter().map(|&[a, b]| Line::between(a, b)).collect();
    let tr = m.trace(&lines, Depth::All).map_err(err)?;
    let segments = |i: usize| tr.segments(i).map(|k| tr.segment(k));
    match format {
        Format::Text => {
            for (i, [a, b]) in ends.iter().enumerate() {
                if ends.len() > 1 {
                    if i > 0 {
                        println!();
                    }
                    println!("# line {i}: {a:?} .. {b:?}");
                }
                println!(
                    "{:>12} {:>12} {:>12} {:>9} {:>9}  label",
                    "t0", "t1", "length", "in", "out"
                );
                for s in segments(i) {
                    println!(
                        "{:>12.6} {:>12.6} {:>12.6} {:>9} {:>9}  {}",
                        s.t0,
                        s.t1,
                        s.t1 - s.t0,
                        surface_name(s.surface_in.and_then(|x| m.surface_id(x))),
                        surface_name(s.surface_out.and_then(|x| m.surface_id(x))),
                        m.describe(&s.label)
                    );
                }
            }
        }
        Format::Csv => {
            println!("line,t0,t1,length,surface_in,surface_out,status,cell,label");
            for i in 0..ends.len() {
                for s in segments(i) {
                    let cell = s
                        .label
                        .cell()
                        .map_or(String::new(), |c| m.cell_id(c).to_string());
                    println!(
                        "{i},{},{},{},{},{},{},{cell},{}",
                        s.t0,
                        s.t1,
                        s.t1 - s.t0,
                        surface_name(s.surface_in.and_then(|x| m.surface_id(x))),
                        surface_name(s.surface_out.and_then(|x| m.surface_id(x))),
                        s.label.status.name(),
                        csv_field(&m.describe(&s.label))
                    );
                }
            }
        }
        Format::Json => {
            // Each distinct label once, as the engine interns them.
            let labels: Vec<Value> = tr.labels.iter().map(|l| json::label(m, l)).collect();
            let rows: Vec<Value> = ends
                .iter()
                .enumerate()
                .map(|(i, [a, b])| {
                    let segs: Vec<Value> = tr
                        .segments(i)
                        .map(|k| {
                            let s = tr.segment(k);
                            json!({
                                "t0": s.t0,
                                "t1": s.t1,
                                "length": s.t1 - s.t0,
                                "surface_in": json::surface(m, s.surface_in),
                                "surface_out": json::surface(m, s.surface_out),
                                "label": labels[tr.ids[k] as usize],
                            })
                        })
                        .collect();
                    json!({"from": a, "to": b, "segments": segs})
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&rows).expect("json"));
        }
    }
    Ok(0)
}
