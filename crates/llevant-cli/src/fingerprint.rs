//! `fingerprint`: digests of every cell and universe, or the differences
//! between two models.

use std::path::PathBuf;

use llevant::{Change, Changes, FingerprintDiff, Model, DEFAULT_PRECISION};
use serde_json::{json, Value};

use crate::args::{Format, Open};
use crate::common::{err, load};

#[derive(clap::Args)]
pub(crate) struct FingerprintArgs {
    /// The MCNP input file.
    model: PathBuf,
    /// Compare with a second model instead of printing digests. The exit
    /// status is then 1 if they differ (beyond numbering), as `diff`'s is.
    #[arg(long)]
    diff: Option<PathBuf>,
    /// Relative precision of hashed values.
    #[arg(long, default_value_t = DEFAULT_PRECISION)]
    precision: f64,
    /// Write to this file instead of stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// With --diff: the report as text, or JSON (`csv` is taken as text).
    /// The digests are always JSON.
    #[arg(long, value_enum, default_value = "text")]
    pub(crate) format: Format,
}

impl FingerprintArgs {
    /// Whether this compares two models (`--diff`).
    pub(crate) fn diffs(&self) -> bool {
        self.diff.is_some()
    }

    pub(crate) fn run(self, open: &Open) -> Result<u8, String> {
        let FingerprintArgs {
            model,
            diff,
            precision,
            output,
            format,
        } = self;
        let a = load(&model, open)?;
        let (text, status) = match diff {
            None => (
                serde_json::to_string_pretty(&fingerprint_json(&a, precision)?).expect("json")
                    + "\n",
                0,
            ),
            Some(other) => {
                let d = a
                    .fingerprint_diff(&load(&other, open)?, precision)
                    .map_err(err)?;
                let text = match format {
                    Format::Json => {
                        serde_json::to_string_pretty(&diff_json(&d)).expect("json") + "\n"
                    }
                    Format::Text | Format::Csv => d.to_string(),
                };
                (text, u8::from(!d.is_empty()))
            }
        };
        match output {
            Some(f) => std::fs::write(&f, text).map_err(|e| format!("{}: {e}", f.display()))?,
            None => print!("{text}"),
        }
        Ok(status)
    }
}

fn fingerprint_json(m: &Model, precision: f64) -> Result<Value, String> {
    let fp = m.fingerprints(precision).map_err(err)?;
    Ok(json!({
        "version": fp.version,
        "precision": fp.precision,
        "cells": m.cells().iter().zip(&fp.cells).map(|(c, f)| json!({
            "cell": c.id, "geometry": f.geometry.to_string(), "content": f.content.to_string()
        })).collect::<Vec<_>>(),
        "universes": m.units().iter().zip(&fp.units).map(|(u, f)| json!({
            "universe": u.universe, "geometry": f.geometry.to_string(), "content": f.content.to_string()
        })).collect::<Vec<_>>(),
    }))
}

/// A diff as Python's `Diff`: per kind (cells, universes), how many are
/// unchanged and the numbers of those that changed content or geometry,
/// were renumbered, removed or are new.
fn diff_json(d: &FingerprintDiff) -> Value {
    let changes = |c: &Changes| {
        let pick =
            |f: fn(&Change) -> Option<i64>| c.changes.iter().filter_map(f).collect::<Vec<_>>();
        json!({
            "unchanged": c.unchanged,
            "content": pick(|x| match x { Change::Content(n) => Some(*n), _ => None }),
            "geometry": pick(|x| match x { Change::Geometry(n) => Some(*n), _ => None }),
            "renumbered": c.changes.iter().filter_map(|x| match x {
                Change::Renumbered { from, to } => Some(json!({"from": from, "to": to})),
                _ => None,
            }).collect::<Vec<_>>(),
            "removed": pick(|x| match x { Change::Removed(n) => Some(*n), _ => None }),
            "new": pick(|x| match x { Change::New(n) => Some(*n), _ => None }),
        })
    };
    json!({
        "same": d.is_empty(),
        "cells": changes(&d.cells),
        "universes": changes(&d.universes),
    })
}
