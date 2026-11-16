//! Fields: numbers over a model's geometry (per cell, cell instance or mesh
//! voxel), drawn over slices and renders (`Model.slice(field=...)`,
//! `Model.render(field=...)`).

use llevant::Depth;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use aci_tallies_geom::{Geometry, Transform};
use llevant_render::field::Overlay;
use llevant_render::scale::Scale;
use llevant_scene::field::{self as rf, Bin, TallyChoice, Values};
use numpy::{AllowTypeChange, IntoPyArray, PyArray1, PyArrayLike1};
use pyo3::prelude::*;

use crate::convert::{err, invalid, vec3, Points};
use crate::model::Model;

/// Numbers over a model's geometry, with the colour scale and opacity they
/// are drawn with. Made for one model (`Field.cells(model, ...)` and the
/// like) and drawn over its images with `field=`.
///
/// Fields are values: `with_scale` and `with_opacity` give new ones.
#[pyclass(frozen, module = "llevant")]
pub struct Field {
    pub model: Arc<llevant::Model>,
    /// The numbers (shared by every look of them) and the look.
    pub inner: Overlay,
}

impl Field {
    fn new(m: &Model, f: rf::Field, name: &str, unit: &str) -> Field {
        let mut f = f;
        if !name.is_empty() {
            f.name = name.into();
        }
        if !unit.is_empty() {
            f.unit = unit.into();
        }
        Field {
            model: m.inner.clone(),
            inner: Overlay::new(Arc::new(f)),
        }
    }

    /// The field, if it was made for `m`.
    pub fn for_model(&self, m: &Arc<llevant::Model>) -> PyResult<&Overlay> {
        if !Arc::ptr_eq(&self.model, m) {
            return Err(invalid("this field was made for another model"));
        }
        Ok(&self.inner)
    }
}

/// `{number: value}` from a dict, or `(numbers, values)` pairs.
fn pairs<K: for<'py> FromPyObjectOwned<'py> + Eq + std::hash::Hash>(
    obj: &Bound<'_, PyAny>,
) -> PyResult<Vec<(K, f64)>> {
    let d: HashMap<K, f64> = obj
        .extract()
        .map_err(|_| invalid("give a dict of numbers to values"))?;
    Ok(d.into_iter().collect())
}

#[pymethods]
impl Field {
    /// Values per cell, `{cell number: value}`; every instance of a cell
    /// has its value.
    #[staticmethod]
    #[pyo3(signature = (model, values, *, name = "", unit = ""))]
    fn cells(model: &Model, values: &Bound<'_, PyAny>, name: &str, unit: &str) -> PyResult<Field> {
        let f = rf::Field::cells(&model.inner, "values", pairs::<i64>(values)?).map_err(err)?;
        Ok(Field::new(model, f, name, unit))
    }

    /// Values per material, `{material number: value}`, on every unfilled
    /// cell of each.
    #[staticmethod]
    #[pyo3(signature = (model, values, *, name = "", unit = ""))]
    fn materials(
        model: &Model,
        values: &Bound<'_, PyAny>,
        name: &str,
        unit: &str,
    ) -> PyResult<Field> {
        let f = rf::Field::materials(&model.inner, "values", pairs::<i64>(values)?).map_err(err)?;
        Ok(Field::new(model, f, name, unit))
    }

    /// Values per cell instance (`Label.instance`): an array of
    /// `model.instance_count` values (NaN for none), or a dict of instance
    /// ids or paths (`"12 < 100[1 0 0] < 5"`) to values.
    #[staticmethod]
    #[pyo3(signature = (model, values, *, name = "", unit = ""))]
    fn instances(
        model: &Model,
        values: &Bound<'_, PyAny>,
        name: &str,
        unit: &str,
    ) -> PyResult<Field> {
        let m = &model.inner;
        let v: Vec<(u64, f64)> = if let Ok(d) = values.extract::<HashMap<u64, f64>>() {
            d.into_iter().collect()
        } else if let Ok(d) = values.extract::<HashMap<String, f64>>() {
            d.into_iter()
                .map(|(p, x)| {
                    let steps = m.parse_path(&p).map_err(err)?;
                    let id = m
                        .instances()
                        .of(m, &steps)
                        .ok_or_else(|| invalid(format!("path `{p}` has no instance id")))?;
                    Ok((id, x))
                })
                .collect::<PyResult<_>>()?
        } else {
            let a: PyArrayLike1<'_, f64, AllowTypeChange> = values
                .extract()
                .map_err(|_| invalid("give an array of values or a dict"))?;
            let total = m.instances().total();
            let n = a.as_array().len();
            if n as u64 != total {
                return Err(invalid(format!("{n} values for {total} instances")));
            }
            a.as_array()
                .iter()
                .enumerate()
                .filter(|(_, x)| x.is_finite())
                .map(|(k, &x)| (k as u64, x))
                .collect()
        };
        let f = rf::Field::instances(m, "values", v).map_err(err)?;
        Ok(Field::new(model, f, name, unit))
    }

    /// Values on a structured mesh, one per voxel in C order of its three
    /// axes (an array of that shape, or flat; NaN for none). A rectangular
    /// mesh gives the bounds along `x`, `y` and `z` (cm), placed by
    /// `transform` (`(rotation, translation)`: world = rotation · local +
    /// translation, as MCNP's TR=); a cylindrical one `r`, `z` and `theta`
    /// (revolutions, 0 to 1) about `axis` through `origin`, theta 0 along
    /// `vec`.
    #[staticmethod]
    #[pyo3(signature = (model, values, *, x = None, y = None, z = None, r = None, theta = None, origin = None, axis = None, vec = None, transform = None, name = "", unit = ""))]
    #[allow(clippy::too_many_arguments)]
    fn mesh(
        model: &Model,
        values: &Bound<'_, PyAny>,
        x: Option<Vec<f64>>,
        y: Option<Vec<f64>>,
        z: Option<Vec<f64>>,
        r: Option<Vec<f64>>,
        theta: Option<Vec<f64>>,
        origin: Option<&Bound<'_, PyAny>>,
        axis: Option<&Bound<'_, PyAny>>,
        vec: Option<&Bound<'_, PyAny>>,
        transform: Option<(Vec<Vec<f64>>, Vec<f64>)>,
        name: &str,
        unit: &str,
    ) -> PyResult<Field> {
        let point = |o: Option<&Bound<'_, PyAny>>, d: [f64; 3], w: &str| match o {
            Some(o) => vec3(o, w),
            None => Ok(d),
        };
        let z = z.ok_or_else(|| invalid("give z bounds"))?;
        let geometry = match (x, y, r, theta) {
            (Some(x), Some(y), None, None) => Geometry::Rectangular {
                x_bounds: x,
                y_bounds: y,
                z_bounds: z,
                transform: match transform {
                    None => None,
                    Some((rot, t)) => {
                        let row = |k: usize| -> PyResult<[f64; 3]> {
                            rot.get(k)
                                .filter(|r| r.len() == 3)
                                .map(|r| [r[0], r[1], r[2]])
                                .ok_or_else(|| invalid("transform: a 3 × 3 rotation"))
                        };
                        if rot.len() != 3 || t.len() != 3 {
                            return Err(invalid("transform: a 3 × 3 rotation and 3 numbers"));
                        }
                        Some(Transform {
                            rotation: [row(0)?, row(1)?, row(2)?],
                            translation: [t[0], t[1], t[2]],
                        })
                    }
                },
            },
            (None, None, Some(r), Some(theta)) => {
                if transform.is_some() {
                    return Err(invalid(
                        "a cylindrical mesh is placed by origin, axis and vec",
                    ));
                }
                Geometry::Cylindrical {
                    r_bounds: r,
                    z_bounds: z,
                    theta_bounds: theta,
                    origin: point(origin, [0.0; 3], "origin")?,
                    axis: point(axis, [0.0, 0.0, 1.0], "axis")?,
                    vec: point(vec, [1.0, 0.0, 0.0], "vec")?,
                }
            }
            _ => return Err(invalid("give x, y and z bounds, or r, z and theta")),
        };
        let flat = values
            .py()
            .import("numpy")?
            .call_method1("ravel", (values,))?;
        let a: PyArrayLike1<'_, f64, AllowTypeChange> = flat
            .extract()
            .map_err(|_| invalid("values: an array of numbers"))?;
        let v: Vec<f32> = a.as_array().iter().map(|&x| x as f32).collect();
        let f = rf::Field::mesh("values", "", geometry, v, None).map_err(err)?;
        Ok(Field::new(model, f, name, unit))
    }

    /// A field from a file: a CSV file (first column `cell`, `material`,
    /// `instance` or `path`; second the values, its header the name with
    /// the unit in brackets) or an MCNP meshtal file (exact meshes from
    /// the model's input, when it has their FMESH cards), D1SUNED cuvmsh
    /// (`out=cuv`, as each voxel's flux) or MCTAL file (cell
    /// tallies, per cell or per instance, bins as the model's input writes
    /// them): `tally` (default the first), `energy` and `time` (`"total"`
    /// or an index), `bins` for MCTAL's other dimensions (`{"m": 1}`),
    /// `error` for relative errors.
    #[staticmethod]
    #[pyo3(signature = (model, path, *, tally = None, energy = "total", time = "total", bins = None, error = false))]
    #[allow(clippy::too_many_arguments)]
    fn read(
        py: Python<'_>,
        model: &Model,
        path: PathBuf,
        tally: Option<u32>,
        energy: &str,
        time: &str,
        bins: Option<std::collections::HashMap<String, Bound<'_, PyAny>>>,
        error: bool,
    ) -> PyResult<Field> {
        let mut choice = TallyChoice {
            tally,
            energy: energy.parse::<Bin>().map_err(err)?,
            time: time.parse::<Bin>().map_err(err)?,
            error,
            ..TallyChoice::default()
        };
        for (dim, bin) in bins.unwrap_or_default() {
            let bin = bin.str()?.to_string().parse::<Bin>().map_err(err)?;
            choice.set_bin(&dim, bin).map_err(err)?;
        }
        let m = model.inner.clone();
        let input = model.input_path();
        let f = py
            .detach(|| rf::read(&m, &path, input.as_deref(), &choice))
            .map_err(err)?;
        Ok(Field::new(model, f, "", ""))
    }

    /// What the values are (shown on the legend).
    #[getter]
    fn name(&self) -> &str {
        &self.inner.field.name
    }

    /// The values' unit (shown on the legend).
    #[getter]
    fn unit(&self) -> &str {
        &self.inner.field.unit
    }

    /// `"cells"`, `"instances"` or `"mesh"`.
    #[getter]
    fn kind(&self) -> &'static str {
        match self.inner.field.values {
            Values::Cells(_) => "cells",
            Values::Instances(_) => "instances",
            Values::Mesh(_) => "mesh",
        }
    }

    /// The colour scale: `(low, high, logarithmic)`.
    #[getter]
    fn scale(&self) -> (f64, f64, bool) {
        let s = self.inner.look.scale;
        (s.lo, s.hi, s.log)
    }

    /// How far the field covers the cells' colours, 0 to 1.
    #[getter]
    fn opacity(&self) -> f32 {
        self.inner.look.opacity
    }

    /// Whether the cells' own colours are under the field (and where it
    /// has no value) rather than a neutral grey.
    #[getter]
    fn cell_colours_under(&self) -> bool {
        self.inner.look.under == llevant_render::field::Under::Geometry
    }

    /// The same field on another scale: ends left out are kept, and a
    /// change of `log` fits the scale to the values again.
    #[pyo3(signature = (low = None, high = None, *, log = None))]
    fn with_scale(
        &self,
        low: Option<f64>,
        high: Option<f64>,
        log: Option<bool>,
    ) -> PyResult<Field> {
        let mut f = self.inner.clone();
        let s = &mut f.look.scale;
        if let Some(l) = log.filter(|&l| l != s.log) {
            let finite = f.field.numbers().map(f64::from).filter(|x| x.is_finite());
            *s = Scale::auto(finite, l).ok_or_else(|| invalid("no values to scale"))?;
        }
        s.lo = low.unwrap_or(s.lo);
        s.hi = high.unwrap_or(s.hi);
        if !(s.lo < s.hi) || (s.log && !(s.lo > 0.0)) {
            return Err(invalid(
                "scale: give low < high (and low > 0 on a log scale)",
            ));
        }
        Ok(Field {
            model: self.model.clone(),
            inner: f,
        })
    }

    /// The same field covering the cells' colours `opacity` of the way.
    fn with_opacity(&self, opacity: f32) -> PyResult<Field> {
        if !(0.0..=1.0).contains(&opacity) {
            return Err(invalid("opacity: from 0 to 1"));
        }
        let mut f = self.inner.clone();
        f.look.opacity = opacity;
        Ok(Field {
            model: self.model.clone(),
            inner: f,
        })
    }

    /// The same field over the cells' own colours (`True`) or over a
    /// neutral grey, which cells without a value show (`False`, the
    /// default).
    fn with_cell_colours_under(&self, on: bool) -> Field {
        let mut f = self.inner.clone();
        f.look.under = if on {
            llevant_render::field::Under::Geometry
        } else {
            llevant_render::field::Under::Neutral
        };
        Field {
            model: self.model.clone(),
            inner: f,
        }
    }

    /// The value at a point, None where the field has none.
    #[pyo3(signature = (x, y = None, z = None))]
    fn value(&self, x: &Bound<'_, PyAny>, y: Option<f64>, z: Option<f64>) -> PyResult<Option<f64>> {
        let p = match (y, z) {
            (Some(y), Some(z)) => [x.extract::<f64>()?, y, z],
            (None, None) => vec3(x, "point")?,
            _ => return Err(invalid("give x, y and z, or one (x, y, z)")),
        };
        let l = self.model.classify_point(p);
        Ok(self.inner.field.value(&self.model, &l, p))
    }

    /// The values at many points (an `(N, 3)` array), NaN where none.
    fn values<'py>(
        &self,
        py: Python<'py>,
        points: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyArray1<f64>>> {
        let pts = Points::extract(points, "points")?;
        let pts = pts.get();
        let (m, f) = (&self.model, &self.inner.field);
        let v: Vec<f64> = py.detach(|| {
            let c = m.classify(&pts, Depth::All);
            pts.iter()
                .zip(&c.ids)
                .map(|(p, &id)| f.value(m, &c.labels[id as usize], *p).unwrap_or(f64::NAN))
                .collect()
        });
        Ok(v.into_pyarray(py))
    }

    fn __repr__(&self) -> String {
        let s = self.inner.look.scale;
        format!(
            "<Field {} ({}), {} to {}{}>",
            self.inner.field.title(),
            self.kind(),
            s.lo,
            s.hi,
            if s.log { ", log" } else { "" }
        )
    }
}
