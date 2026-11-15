//! Python values to llevant arguments, and llevant errors to Python
//! exceptions.

use std::borrow::Cow;

use llevant::{Aabb, GridMode, GroupBy, Line, Scope, Status, Vec3};
use llevant_render::colours::ColourBy;
use numpy::{AllowTypeChange, PyArrayLike1, PyArrayLike2, PyUntypedArrayMethods};
use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;

pyo3::create_exception!(
    llevant,
    LlevantError,
    pyo3::exceptions::PyException,
    "An error from llevant that is not about an argument or a file (the GPU renderer)."
);

/// `Io` → `OSError`; bad arguments (lines, unknown cells or universes,
/// invalid values) → `ValueError`; anything else → `LlevantError`.
pub fn err(e: llevant::Error) -> PyErr {
    match e {
        llevant::Error::Io { .. } => PyOSError::new_err(e.to_string()),
        llevant::Error::Line { .. }
        | llevant::Error::NoCell(_)
        | llevant::Error::NoUniverse(_)
        | llevant::Error::Invalid(_) => PyValueError::new_err(e.to_string()),
        _ => LlevantError::new_err(e.to_string()),
    }
}

pub fn invalid(msg: impl Into<String>) -> PyErr {
    PyValueError::new_err(msg.into())
}

/// Points from anything `numpy.asarray` takes, shaped `(N, 3)` (or a single
/// point, `(3,)`). C-ordered float64 input is read in place.
pub struct Points<'py>(PyArrayLike2<'py, f64, AllowTypeChange>);

impl<'py> Points<'py> {
    pub fn extract(obj: &Bound<'py, PyAny>, what: &str) -> PyResult<Points<'py>> {
        let np = obj.py().import("numpy")?;
        let a = np.call_method1("asarray", (obj, "float64"))?;
        let a = if a.getattr("ndim")?.extract::<usize>()? == 1 {
            a.call_method1("reshape", (1, -1))?
        } else {
            a
        };
        let arr: PyArrayLike2<'py, f64, AllowTypeChange> = a.extract()?;
        if arr.shape()[1] != 3 {
            return Err(invalid(format!(
                "{what}: expected an (N, 3) array, got shape {:?}",
                arr.shape()
            )));
        }
        Ok(Points(arr))
    }

    pub fn len(&self) -> usize {
        self.0.shape()[0]
    }

    /// The points, borrowed from C-ordered input and copied from anything
    /// else (strided views, and Fortran-ordered arrays such as
    /// `polars`/`pandas` `to_numpy()` give: `as_slice` takes those too, in
    /// memory order, i.e. by columns).
    pub fn get(&self) -> Cow<'_, [Vec3]> {
        match self.0.as_slice() {
            Ok(s) if self.0.is_c_contiguous() => Cow::Borrowed(s.as_chunks::<3>().0),
            _ => Cow::Owned(
                self.0
                    .as_array()
                    .rows()
                    .into_iter()
                    .map(|r| [r[0], r[1], r[2]])
                    .collect(),
            ),
        }
    }
}

/// A number for every one of `n` items: a scalar, or an array of `n`.
pub fn per_item(obj: &Bound<'_, PyAny>, n: usize, what: &str) -> PyResult<Vec<f64>> {
    if let Ok(x) = obj.extract::<f64>() {
        return Ok(vec![x; n]);
    }
    let a: PyArrayLike1<'_, f64, AllowTypeChange> = obj.extract()?;
    if a.len() != n {
        return Err(invalid(format!(
            "{what}: expected a number or {n} numbers, got {}",
            a.len()
        )));
    }
    Ok(a.as_array().to_vec())
}

/// Lines from start points and either end points or directions (normalised)
/// with a range of `t`.
pub fn lines(
    start: &Bound<'_, PyAny>,
    end: Option<&Bound<'_, PyAny>>,
    dir: Option<&Bound<'_, PyAny>>,
    t_min: &Bound<'_, PyAny>,
    t_max: Option<&Bound<'_, PyAny>>,
    default_t_max: Option<f64>,
) -> PyResult<Vec<Line>> {
    let a = Points::extract(start, "start")?;
    let n = a.len();
    let a = a.get();
    let same = |p: &Points, what: &str| {
        if p.len() == n {
            Ok(())
        } else {
            Err(invalid(format!(
                "{what}: {} points for {n} start points",
                p.len()
            )))
        }
    };
    match (end, dir) {
        (Some(e), None) => {
            if t_max.is_some() {
                return Err(invalid("give `end` or `dir` with `t_max`, not both"));
            }
            let b = Points::extract(e, "end")?;
            same(&b, "end")?;
            Ok(a.iter()
                .zip(b.get().iter())
                .map(|(&a, &b)| Line::between(a, b))
                .collect())
        }
        (None, Some(d)) => {
            let d = Points::extract(d, "dir")?;
            same(&d, "dir")?;
            let lo = per_item(t_min, n, "t_min")?;
            let hi = match (t_max, default_t_max) {
                (Some(t), _) => per_item(t, n, "t_max")?,
                (None, Some(t)) => vec![t; n],
                (None, None) => return Err(invalid("`dir` needs `t_max`")),
            };
            Ok(a.iter()
                .zip(d.get().iter())
                .enumerate()
                .map(|(k, (&o, &d))| Line::new(o, d, lo[k], hi[k]))
                .collect())
        }
        _ => Err(invalid("give either `end` or `dir`")),
    }
}

pub fn vec3(obj: &Bound<'_, PyAny>, what: &str) -> PyResult<Vec3> {
    obj.extract::<[f64; 3]>()
        .map_err(|_| invalid(format!("{what}: expected 3 numbers")))
}

/// A box: `(x0, y0, z0, x1, y1, z1)` or `((x0, y0, z0), (x1, y1, z1))`.
pub fn region(obj: &Bound<'_, PyAny>) -> PyResult<Aabb> {
    let b = if let Ok(v) = obj.extract::<[f64; 6]>() {
        Aabb {
            lo: [v[0], v[1], v[2]],
            hi: [v[3], v[4], v[5]],
        }
    } else if let Ok((lo, hi)) = obj.extract::<([f64; 3], [f64; 3])>() {
        Aabb { lo, hi }
    } else {
        return Err(invalid(
            "region: expected (x0, y0, z0, x1, y1, z1) or ((x0, y0, z0), (x1, y1, z1))",
        ));
    };
    if (0..3).any(|i| !(b.lo[i] < b.hi[i])) {
        return Err(invalid(format!(
            "region: empty box {:?} .. {:?}",
            b.lo, b.hi
        )));
    }
    Ok(b)
}

/// A point as a Python tuple.
pub type Xyz = (f64, f64, f64);

pub fn xyz(v: Vec3) -> Xyz {
    (v[0], v[1], v[2])
}

/// A box as `((x0, y0, z0), (x1, y1, z1))`, None if empty.
pub fn bbox(b: &Aabb) -> Option<(Xyz, Xyz)> {
    (!b.is_empty()).then_some((xyz(b.lo), xyz(b.hi)))
}

fn choice<T: Copy>(s: &str, what: &str, options: &[(&str, T)]) -> PyResult<T> {
    options
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(s))
        .map(|&(_, t)| t)
        .ok_or_else(|| {
            let names: Vec<&str> = options.iter().map(|o| o.0).collect();
            invalid(format!("{what}: `{s}` is not one of {}", names.join(", ")))
        })
}

pub fn scope(s: &str) -> PyResult<Scope> {
    choice(
        s,
        "scope",
        &[("contents", Scope::Contents), ("problem", Scope::Problem)],
    )
}

pub fn colour(s: &str) -> PyResult<ColourBy> {
    choice(
        s,
        "colour",
        &[
            ("cell", ColourBy::Cell),
            ("material", ColourBy::Material),
            ("universe", ColourBy::Universe),
            ("realistic", ColourBy::Realistic),
        ],
    )
}

pub fn group(s: &str) -> PyResult<GroupBy> {
    choice(
        s,
        "by",
        &[
            ("cell", GroupBy::Cell),
            ("material", GroupBy::Material),
            ("universe", GroupBy::Universe),
            ("label", GroupBy::Label),
        ],
    )
}

pub fn grid(s: &str) -> PyResult<GridMode> {
    choice(
        s,
        "grid",
        &[
            ("auto", GridMode::Auto),
            ("always", GridMode::Always),
            ("off", GridMode::Off),
        ],
    )
}

/// Status names, indexed by [`status_code`].
pub const STATUS_NAMES: [&str; 6] = [
    "inside",
    "gap",
    "overlap",
    "on-surface",
    "unsupported",
    "too-deep",
];

/// Index of a status in [`STATUS_NAMES`].
pub fn status_code(s: Status) -> i8 {
    STATUS_NAMES
        .iter()
        .position(|&n| n == s.name())
        .map_or(-1, |k| k as i8)
}
