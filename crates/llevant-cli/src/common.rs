//! Helpers every command uses: loading the model (through the cache), numbers
//! and boxes from arguments, rules of cells, and how values are written out.

use std::time::Instant;

use llevant::{Aabb, Model, Options, Scope, SurfaceId};
use llevant_scene::bounds;
use llevant_scene::index::ModelIndex;
use llevant_scene::labels::Labels;
use llevant_scene::query::Query;

use crate::args::Open;

/// A library error as a message.
pub(crate) fn err(e: llevant::Error) -> String {
    e.to_string()
}

pub(crate) fn load(path: &std::path::Path, open: &Open) -> Result<Model, String> {
    let t = Instant::now();
    let opts = Options::default();
    let mut m = llevant_scene::store::Store::from_env()
        .load(path, &opts, None)
        .map_err(err)?;
    eprintln!(
        "loaded {} ({} cells) in {:.1} s",
        path.display(),
        m.cells().len(),
        t.elapsed().as_secs_f64()
    );
    if let Some(u) = open.universe {
        let t = Instant::now();
        m = m.universe(u, open.cut()?, opts.grid).map_err(err)?;
        eprintln!(
            "opened universe {u} ({} cells) in {:.1} s",
            m.cells().len(),
            t.elapsed().as_secs_f64()
        );
    }
    if !m.diagnostics().is_empty() {
        eprintln!(
            "{} diagnostics (llevant info {} lists them)",
            m.diagnostics().len(),
            path.display()
        );
    }
    Ok(m)
}

pub(crate) fn numbers(s: &str, n: usize, what: &str) -> Result<Vec<f64>, String> {
    let v: Vec<f64> = s
        .split([',', ' '])
        .filter(|x| !x.is_empty())
        .map(|x| x.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("{what}: expected {n} numbers, got `{s}`"))?;
    if v.len() != n {
        return Err(format!("{what}: expected {n} numbers, got `{s}`"));
    }
    Ok(v)
}

pub(crate) fn vec3(s: &str, what: &str) -> Result<[f64; 3], String> {
    let v = numbers(s, 3, what)?;
    Ok([v[0], v[1], v[2]])
}

pub(crate) fn region(m: &Model, s: &Option<String>, scope: Scope) -> Result<Aabb, String> {
    match s {
        Some(s) => {
            let v = numbers(s, 6, "region")?;
            let b = Aabb {
                lo: [v[0], v[1], v[2]],
                hi: [v[3], v[4], v[5]],
            };
            if (0..3).any(|i| !(b.lo[i] < b.hi[i])) {
                return Err(format!("region: empty box `{s}`"));
            }
            Ok(b)
        }
        None => {
            let t = Instant::now();
            let b = m.extent(scope, 128);
            if !b.is_finite() || b.is_empty() {
                return Err("could not find the model's extent; give --region".into());
            }
            eprintln!(
                "extent {:.1?} .. {:.1?} (measured in {:.2} s; --region to change)",
                b.lo,
                b.hi,
                t.elapsed().as_secs_f64()
            );
            Ok(b)
        }
    }
}

/// The box framing the cells of `q`: around every instance of each
/// ([`bounds::selection_box`]).
pub(crate) fn framing(m: &Model, x: &ModelIndex, q: &Query) -> Result<Aabb, String> {
    let t = Instant::now();
    let set = q
        .resolve(m, x)
        .map_err(|e| format!("--isolate `{q}`: {e}"))?;
    let b = bounds::selection_box(m, &set)
        .ok_or_else(|| format!("--isolate `{q}`: the cells are nowhere; give --region"))?;
    eprintln!(
        "framing the cells isolated: {} .. {} (boxed in {:.2} s; --region to change)",
        fmt_point(b.lo),
        fmt_point(b.hi),
        t.elapsed().as_secs_f64()
    );
    Ok(b)
}

/// A point to 0.1 cm, as the messages give boxes.
pub(crate) fn fmt_point(p: [f64; 3]) -> String {
    format!("[{:.1}, {:.1}, {:.1}]", p[0], p[1], p[2])
}

pub(crate) fn surface_name(s: Option<SurfaceId>) -> String {
    match s {
        None => String::new(),
        Some(SurfaceId { id, facet: 0 }) => id.to_string(),
        Some(SurfaceId { id, facet }) => format!("{id}.{facet}"),
    }
}

pub(crate) fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// A rule of cells given on the command line, for the option `what`:
/// the rule, or `@path` for a file holding it ([`rule_file`]).
pub(crate) fn rule(s: &str, what: &str) -> Result<Query, String> {
    let text = match s.strip_prefix('@') {
        Some(path) => {
            rule_file(&std::fs::read_to_string(path).map_err(|e| format!("{what} `{s}`: {e}"))?)
        }
        None => s.to_string(),
    };
    text.parse().map_err(|e| {
        let shown: String = text.chars().take(200).collect();
        format!("{what} `{shown}`: {e}")
    })
}

/// The rule a file gives (`--isolate @cells.txt`), `#` starting a comment:
/// a list of cell numbers and ranges (`10 11 20-25`, on any number of
/// lines, separated by spaces or commas) is those cells; otherwise each
/// line is a rule, and the file is any of them (`c 10-20` on one line and
/// `u 5 & m 3` on the next: either).
pub(crate) fn rule_file(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .collect();
    let words: Vec<&str> = lines
        .iter()
        .flat_map(|l| l.split([' ', '\t', ',']))
        .filter(|w| !w.is_empty())
        .collect();
    let number = |w: &str| {
        let (a, b) = w.split_once('-').unwrap_or((w, "0"));
        a.parse::<u64>().is_ok() && b.parse::<u64>().is_ok()
    };
    if !words.is_empty() && words.iter().all(|w| number(w)) {
        format!("c {}", words.join(" "))
    } else {
        lines
            .iter()
            .map(|l| format!("({l})"))
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

/// The index rules of `m` resolve against: with the cells' labels, read
/// from the input at `path`, when one of `rules` has a `label` term (the
/// compiled model, maybe read from the cache, has no text).
pub(crate) fn index(
    m: &Model,
    path: &std::path::Path,
    rules: &[&Query],
) -> Result<ModelIndex, String> {
    let x = ModelIndex::new(m);
    if rules.iter().any(|q| q.uses_labels()) {
        let t = Instant::now();
        let labels = Labels::read(path).map_err(err)?;
        eprintln!(
            "read {} labels of cells in {:.2} s",
            labels.len(),
            t.elapsed().as_secs_f64()
        );
        x.set_labels(m, std::sync::Arc::new(labels));
    }
    Ok(x)
}

/// `x0,y0,z0,x1,y1,z1`, as --region takes it.
pub(crate) fn box_arg(b: &Aabb) -> String {
    let v = [b.lo, b.hi].concat();
    v.iter()
        .map(|x| format!("{x:.3}"))
        .collect::<Vec<_>>()
        .join(",")
}
