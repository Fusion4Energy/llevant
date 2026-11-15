//! The shapes every `--format json` output shares: a label, a surface and a
//! box. Each has one builder here, so the commands cannot drift apart, and the
//! fields mirror the Python module's (`Label`, `Surface`). Numbers are MCNP
//! numbers; a missing value is `null`, never `""`.

use llevant::{Aabb, Label, Model};
use serde_json::{json, Value};

/// Where a point (or a segment) is, as Python's `Label`:
/// `status`, `cell` (the deepest cell, inside only), `path` (cells from
/// level 0 down, with their lattice element), `involved` (for overlaps,
/// on-surface and unsupported points: the cells involved at the last level),
/// `universe`, `material` and `density` of the cell (inside only),
/// `instance` and `instance_path` (which copy of the cell), and `text`
/// (the label in MCNP terms, as text output prints it).
pub(crate) fn label(m: &Model, l: &Label) -> Value {
    let cell = l.cell().map(|c| m.cell(c));
    let instance = m.instance(l);
    json!({
        "status": l.status.name(),
        "cell": cell.map(|c| c.id),
        "path": l
            .path
            .iter()
            .map(|s| json!({"cell": m.cell_id(s.cell), "lattice": s.lattice}))
            .collect::<Vec<_>>(),
        "involved": l.involved.iter().map(|&c| m.cell_id(c)).collect::<Vec<_>>(),
        "universe": cell.map(|c| m.units()[c.unit as usize].universe),
        "material": cell.map(|c| c.material),
        "density": cell.map(|c| c.density),
        "instance": instance,
        "instance_path": instance.map(|_| m.format_path(&l.path)),
        "text": m.describe(l),
    })
}

/// A compiled surface as its MCNP number and facet (0: the whole surface),
/// as Python's `Surface`; `null` for none (the start or end of a line).
pub(crate) fn surface(m: &Model, shape: Option<u32>) -> Value {
    match shape.and_then(|s| m.surface_id(s)) {
        None => Value::Null,
        Some(s) => json!({"id": s.id, "facet": s.facet}),
    }
}

/// A box as `{"lo": [x, y, z], "hi": [x, y, z]}` in cm; `null` if empty.
pub(crate) fn bbox(b: &Aabb) -> Value {
    if b.is_empty() {
        Value::Null
    } else {
        json!({"lo": b.lo, "hi": b.hi})
    }
}
