//! `info`, `bbox` and `materials`: what the model holds, where cells are, and
//! what materials look like.

use std::path::PathBuf;
use std::time::Instant;

use llevant::{Aabb, Model, Scope};
use llevant_scene::bounds;
use llevant_scene::index::ModelIndex;
use llevant_scene::materials::{Category, Looks};
use llevant_scene::query::Query;
use serde_json::{json, Value};

use crate::args::{Format, LookArgs, Open};
use crate::common::{box_arg, csv_field, index, load, rule};
use crate::json;

#[derive(clap::Args)]
pub(crate) struct InfoArgs {
    /// The MCNP input file.
    model: PathBuf,
    /// Print every diagnostic, not just the first 20 (JSON has them all).
    #[arg(long)]
    all: bool,
    /// Text, or JSON (`csv` is taken as text: there is no table).
    #[arg(long, value_enum, default_value = "text")]
    pub(crate) format: Format,
}

impl InfoArgs {
    pub(crate) fn run(self, open: &Open) -> Result<u8, String> {
        info(&load(&self.model, open)?, self.all, self.format)
    }
}

#[derive(clap::Args)]
pub(crate) struct BboxArgs {
    /// The MCNP input file.
    model: PathBuf,
    /// The cells, as a rule (`c 10-20`, `u 5 & m 3`, `label "BSV"`, as
    /// --isolate), or
    /// `@file` with the rule or a list of cell numbers.
    #[arg(long)]
    cells: String,
    /// Text: a line per cell instance too.
    #[arg(long)]
    all: bool,
    /// Output: a text table, CSV, or JSON (its fields: docs/guide/cli.md,
    /// "Scripting and agents").
    #[arg(long, value_enum, default_value = "text")]
    pub(crate) format: Format,
}

impl BboxArgs {
    pub(crate) fn run(self, open: &Open) -> Result<u8, String> {
        let BboxArgs {
            model,
            cells,
            all,
            format,
        } = self;
        let m = load(&model, open)?;
        let q = rule(&cells, "--cells")?;
        let x = index(&m, &model, &[&q])?;
        bbox(&m, &x, &q, all, format)
    }
}

#[derive(clap::Args)]
pub(crate) struct MaterialsArgs {
    /// The MCNP input file.
    model: PathBuf,
    #[command(flatten)]
    looks: LookArgs,
    /// Output: a text table, CSV, or JSON (its fields: docs/guide/cli.md,
    /// "Scripting and agents").
    #[arg(long, value_enum, default_value = "text")]
    pub(crate) format: Format,
}

impl MaterialsArgs {
    pub(crate) fn run(self, open: &Open) -> Result<u8, String> {
        let MaterialsArgs {
            model,
            looks,
            format,
        } = self;
        let m = load(&model, open)?;
        materials(&looks.looks(&m)?, format);
        Ok(0)
    }
}

fn bbox(m: &Model, x: &ModelIndex, q: &Query, all: bool, format: Format) -> Result<u8, String> {
    let set = q.resolve(m, x).map_err(|e| format!("--cells `{q}`: {e}"))?;
    if set.is_empty() {
        return Err(format!("--cells `{q}`: no cells"));
    }
    let t = Instant::now();
    let boxes = bounds::instance_boxes(m, &set);
    eprintln!(
        "boxed {} instances of {} cells in {:.2} s",
        boxes.len(),
        set.count(),
        t.elapsed().as_secs_f64()
    );
    let union = bounds::union(&boxes);
    let outer = boxes.iter().filter(|b| !b.exact).count();
    let id = |b: &llevant::PlacedBox| m.cell_id(b.cell);
    match format {
        Format::Text => {
            let Some(u) = union else {
                println!("nowhere: no cell fills their universes (or not within the problem)");
                return Ok(1);
            };
            let size = [0, 1, 2].map(|k| u.hi[k] - u.lo[k]);
            println!("box      {:.3?} .. {:.3?}", u.lo, u.hi);
            println!("size     {:.3?} cm", size);
            println!("centre   {:.3?}", u.center());
            println!("region   --region={}", box_arg(&u));
            print!(
                "         {} cells, {} instances; conservative (padded outward)",
                set.count(),
                boxes.len()
            );
            if outer > 0 {
                print!(", {outer} only an outer bound (below a lattice)");
            }
            println!();
            if all {
                for b in &boxes {
                    println!(
                        "cell {}  {:.3?} .. {:.3?}{}",
                        id(b),
                        b.global.lo,
                        b.global.hi,
                        if b.exact { "" } else { "  (outer bound)" }
                    );
                }
            }
        }
        Format::Csv => {
            println!("cell,x0,y0,z0,x1,y1,z1,exact");
            let row = |c: &str, b: &Aabb, exact: bool| {
                println!(
                    "{c},{},{},{},{},{},{},{exact}",
                    b.lo[0], b.lo[1], b.lo[2], b.hi[0], b.hi[1], b.hi[2]
                )
            };
            for b in &boxes {
                row(&id(b).to_string(), &b.global, b.exact);
            }
            if let Some(u) = union {
                row("all", &u, outer == 0);
            }
        }
        Format::Json => {
            let v = json!({
                "box": union.map_or(Value::Null, |u| json::bbox(&u)),
                "cells": set.count(),
                "exact": outer == 0,
                "instances": boxes.iter().map(|b| json!({
                    "cell": id(b),
                    "box": json::bbox(&b.global),
                    "exact": b.exact,
                })).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&v).expect("json"));
        }
    }
    Ok(if union.is_some() { 0 } else { 1 })
}

fn info(m: &Model, all: bool, format: Format) -> Result<u8, String> {
    let s = m.stats();
    let b = m.extent(Scope::Problem, 128);
    if format == Format::Json {
        let boxed = json::bbox;
        let v = json!({
            "cells": s.cells,
            "surfaces": s.surfaces,
            "universes": s.units,
            "lattices": s.lattices,
            "unsupported_cells": s.unsupported_cells,
            "octrees": {
                "universes": s.grid_units,
                "bytes": s.grid_bytes,
                "pending": s.pending_units,
            },
            "extent": b.is_finite().then(|| boxed(&b)),
            "problem_box": m.problem_box().map(|p| boxed(&p)),
            "cut_cell": m.cut_cell().map(|c| m.cell_id(c)),
            "compile_s": m.timings().iter().map(|(n, t)| (n.to_string(), json!(t))).collect::<serde_json::Map<_, _>>(),
            "diagnostics": m.diagnostics().iter().map(|d| d.to_string()).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&v).expect("json"));
        return Ok(0);
    }
    println!("cells       {}", s.cells);
    println!(
        "surfaces    {} (compiled, macrobody facets apart)",
        s.surfaces
    );
    println!("universes   {}", s.units);
    println!("lattices    {}", s.lattices);
    println!("unsupported {} cells", s.unsupported_cells);
    print!(
        "octrees     {} universes, {:.0} MB",
        s.grid_units,
        s.grid_bytes as f64 / 1e6
    );
    if s.pending_units > 0 {
        print!(" ({} still to build)", s.pending_units);
    }
    println!();
    if b.is_finite() {
        println!("extent      {:?} .. {:?}", b.lo, b.hi);
    }
    if let Some(p) = m.problem_box() {
        println!("problem box {:?} .. {:?}", p.lo, p.hi);
    }
    if let Some(c) = m.cut_cell() {
        println!("cut         cell {} beyond the problem box", m.cell_id(c));
    }
    let times: Vec<String> = m
        .timings()
        .iter()
        .map(|(n, t)| format!("{n} {t:.2}"))
        .collect();
    println!("compile (s) {}", times.join(", "));
    let d = m.diagnostics();
    println!("diagnostics {}", d.len());
    let shown = if all { d.len() } else { d.len().min(20) };
    for x in &d[..shown] {
        println!("  {x}");
    }
    if shown < d.len() {
        println!("  ... {} more (--all)", d.len() - shown);
    }
    Ok(0)
}

fn materials(l: &Looks, format: Format) {
    let hex = |c: [u8; 3]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    let density = |d: (f64, f64)| {
        if d.0.is_nan() {
            String::new()
        } else if d.0 == d.1 {
            format!("{:.4}", d.0)
        } else {
            format!("{:.4}-{:.4}", d.0, d.1)
        }
    };
    match format {
        Format::Text => {
            println!(
                "{:>9} {:>6} {:>17} {:>11} {:>7} {:>5} {:>5} {:>5}  composition",
                "material", "cells", "density (g/cm3)", "look", "colour", "rough", "metal", "clear"
            );
            for x in &l.materials {
                let a = &x.appearance;
                println!(
                    "{:>9} {:>6} {:>17} {:>11} {:>7} {:>5.2} {:>5.2} {:>5.2}  {}{}",
                    x.id,
                    x.cells,
                    density(x.density),
                    a.category.name(),
                    hex(a.srgb()),
                    a.roughness,
                    a.metalness,
                    a.clarity,
                    x.mix,
                    if x.spec.is_some() { " (set)" } else { "" }
                );
            }
            let mut by: Vec<(Category, usize, usize)> = Vec::new();
            for x in &l.materials {
                let c = x.appearance.category;
                match by.iter_mut().find(|e| e.0 == c) {
                    Some(e) => {
                        e.1 += 1;
                        e.2 += x.cells;
                    }
                    None => by.push((c, 1, x.cells)),
                }
            }
            by.sort();
            println!();
            for (c, n, cells) in by {
                println!("{:>11}: {n} materials, {cells} cells", c.name());
            }
        }
        Format::Csv => {
            println!("material,cells,density_min,density_max,look,colour,roughness,metalness,clarity,set,composition");
            for x in &l.materials {
                let a = &x.appearance;
                println!(
                    "{},{},{},{},{},{},{},{},{},{},{}",
                    x.id,
                    x.cells,
                    x.density.0,
                    x.density.1,
                    a.category.name(),
                    hex(a.srgb()),
                    a.roughness,
                    a.metalness,
                    a.clarity,
                    x.spec.is_some(),
                    csv_field(&x.mix.to_string())
                );
            }
        }
        Format::Json => {
            let rows: Vec<Value> = l
                .materials
                .iter()
                .map(|x| {
                    let a = &x.appearance;
                    let num = |v: f64| if v.is_nan() { Value::Null } else { json!(v) };
                    json!({
                        "material": x.id,
                        "cells": x.cells,
                        "density": [num(x.density.0), num(x.density.1)],
                        "look": a.category.name(),
                        "colour": hex(a.srgb()),
                        "roughness": a.roughness,
                        "metalness": a.metalness,
                        "clarity": a.clarity,
                        "set": x.spec.is_some(),
                        "parts": x.mix.parts.iter().map(|(c, v)| json!({"category": c.name(), "volume_fraction": v})).collect::<Vec<_>>(),
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&rows).expect("json"));
        }
    }
}
