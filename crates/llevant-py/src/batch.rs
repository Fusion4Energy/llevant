//! Results of batch queries: numpy columns (moved from Rust, no copies)
//! with label ids into a table of distinct labels.

use std::sync::Arc;

use llevant::{Status, Vec3, NO_HIT, NO_SURFACE};
use numpy::{Element, IntoPyArray, PyArray1, PyArrayMethods, PyUntypedArrayMethods};
use pyo3::exceptions::PyIndexError;
use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::types::PyDict;

use crate::convert::{invalid, status_code};
use crate::results::{Label, Segment, Surface};

/// A numpy array that owns `v`, marked read-only (the columns of a result
/// belong together).
pub fn frozen_array<T: Element>(py: Python<'_>, v: Vec<T>) -> PyResult<Bound<'_, PyArray1<T>>> {
    let a = v.into_pyarray(py);
    a.getattr("flags")?.setattr("writeable", false)?;
    Ok(a)
}

/// `n × 3` rows as a read-only `(n, 3)` array.
pub fn frozen_rows<'py>(py: Python<'py>, v: &[Vec3]) -> PyResult<Bound<'py, PyAny>> {
    let flat: Vec<f64> = v.iter().flatten().copied().collect();
    let a = flat.into_pyarray(py).reshape([v.len(), 3])?;
    a.getattr("flags")?.setattr("writeable", false)?;
    Ok(a.into_any())
}

/// The distinct labels of a result, and per-item columns gathered from
/// them.
pub struct LabelTable {
    pub model: Arc<llevant::Model>,
    pub labels: Vec<llevant::Label>,
    objects: GILOnceCell<Vec<Py<Label>>>,
}

impl LabelTable {
    pub fn new(model: Arc<llevant::Model>, labels: Vec<llevant::Label>) -> LabelTable {
        LabelTable {
            model,
            labels,
            objects: GILOnceCell::new(),
        }
    }

    /// The labels as Python objects, made once.
    pub fn objects(&self, py: Python<'_>) -> PyResult<&Vec<Py<Label>>> {
        self.objects.get_or_try_init(py, || {
            self.labels
                .iter()
                .map(|l| Py::new(py, Label::new(&self.model, l)))
                .collect()
        })
    }

    /// The label with id `id`; None for [`NO_HIT`].
    pub fn label(&self, py: Python<'_>, id: u32) -> PyResult<Option<Py<Label>>> {
        if id == NO_HIT {
            return Ok(None);
        }
        let o = self.objects(py)?;
        o.get(id as usize)
            .map(|l| Some(l.clone_ref(py)))
            .ok_or_else(|| invalid(format!("label id {id} out of range")))
    }

    /// `f` of the label of every id (`none` for [`NO_HIT`] and ids out of
    /// range).
    pub fn gather<T: Copy>(
        &self,
        ids: &[u32],
        none: T,
        f: impl Fn(&llevant::Label) -> T,
    ) -> Vec<T> {
        let per: Vec<T> = self.labels.iter().map(f).collect();
        ids.iter()
            .map(|&i| per.get(i as usize).copied().unwrap_or(none))
            .collect()
    }

    pub fn cell(&self, ids: &[u32]) -> Vec<i64> {
        let m = &self.model;
        self.gather(ids, -1, |l| l.cell().map_or(-1, |c| m.cell_id(c)))
    }

    pub fn material(&self, ids: &[u32]) -> Vec<i64> {
        let m = &self.model;
        self.gather(ids, -1, |l| l.cell().map_or(-1, |c| m.cell(c).material))
    }

    pub fn universe(&self, ids: &[u32]) -> Vec<i64> {
        let m = &self.model;
        self.gather(ids, -1, |l| {
            l.cell()
                .map_or(-1, |c| m.units()[m.cell(c).unit as usize].universe)
        })
    }

    pub fn instance(&self, ids: &[u32]) -> Vec<i64> {
        let m = &self.model;
        self.gather(ids, -1, |l| m.instance(l).map_or(-1, |i| i as i64))
    }

    pub fn status(&self, ids: &[u32]) -> Vec<i8> {
        self.gather(ids, -1, |l| status_code(l.status))
    }

    /// Label ids as `int64` codes, -1 for none (for categorical columns).
    pub fn codes(&self, ids: &[u32]) -> Vec<i64> {
        ids.iter()
            .map(|&i| if i == NO_HIT { -1 } else { i as i64 })
            .collect()
    }

    /// Columns shared by every result: label code, status, cell, material,
    /// universe, instance.
    pub fn columns<'py>(
        &self,
        py: Python<'py>,
        ids: &[u32],
        d: &Bound<'py, PyDict>,
    ) -> PyResult<()> {
        d.set_item("label", self.codes(ids).into_pyarray(py))?;
        d.set_item("status", self.status(ids).into_pyarray(py))?;
        d.set_item("cell", self.cell(ids).into_pyarray(py))?;
        d.set_item("material", self.material(ids).into_pyarray(py))?;
        d.set_item("universe", self.universe(ids).into_pyarray(py))?;
        d.set_item("instance", self.instance(ids).into_pyarray(py))?;
        Ok(())
    }
}

/// The contents of a column of a result.
fn slice<'a, T: Element>(py: Python<'a>, a: &'a Py<PyArray1<T>>) -> PyResult<&'a [T]> {
    // SAFETY: the result owns the array and made it read-only, and the
    // slice is only used while the GIL is held.
    unsafe { a.bind(py).as_slice() }.map_err(|e| invalid(e.to_string()))
}

fn ids_of<'a>(py: Python<'a>, a: &'a Py<PyArray1<u32>>) -> PyResult<&'a [u32]> {
    slice(py, a)
}

fn f64s<'a>(py: Python<'a>, a: &'a Py<PyArray1<f64>>) -> PyResult<&'a [f64]> {
    slice(py, a)
}

fn index(i: isize, n: usize) -> PyResult<usize> {
    let k = if i < 0 { i + n as isize } else { i };
    if k < 0 || k as usize >= n {
        return Err(PyIndexError::new_err(format!(
            "index {i} out of range for {n}"
        )));
    }
    Ok(k as usize)
}

/// `#[pymethods]` of a result with label ids: the shared columns, then
/// `$rest`.
macro_rules! label_columns {
    ($t:ty { $($rest:tt)* }) => {
        #[pymethods]
        impl $t {
            /// The distinct labels; `ids` index into them.
            #[getter]
            fn labels(&self, py: Python<'_>) -> PyResult<Vec<Py<Label>>> {
                Ok(self
                    .table
                    .objects(py)?
                    .iter()
                    .map(|l| l.clone_ref(py))
                    .collect())
            }

            /// Label id of each item (`uint32`), into `labels`.
            #[getter]
            fn ids(&self, py: Python<'_>) -> Py<PyArray1<u32>> {
                self.ids.clone_ref(py)
            }

            /// MCNP cell holding each item (`int64`), -1 when not inside one.
            #[getter]
            fn cell<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray1<i64>>> {
                Ok(self.table.cell(ids_of(py, &self.ids)?).into_pyarray(py))
            }

            /// Material of that cell (`int64`, 0 for void), -1 when not inside one.
            #[getter]
            fn material<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray1<i64>>> {
                Ok(self.table.material(ids_of(py, &self.ids)?).into_pyarray(py))
            }

            /// Universe of that cell (`int64`), -1 when not inside one.
            #[getter]
            fn universe<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray1<i64>>> {
                Ok(self.table.universe(ids_of(py, &self.ids)?).into_pyarray(py))
            }

            /// Instance of that cell (`int64`, `Label.instance`), -1 when
            /// not inside one.
            #[getter]
            fn instance<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray1<i64>>> {
                Ok(self.table.instance(ids_of(py, &self.ids)?).into_pyarray(py))
            }

            /// Status of each item (`int8`): an index into
            /// `llevant.STATUS_NAMES`, -1 for none.
            #[getter]
            fn status<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray1<i8>>> {
                Ok(self.table.status(ids_of(py, &self.ids)?).into_pyarray(py))
            }

            /// The columns as a Polars data frame (needs `polars`); labels
            /// and statuses are categorical.
            fn to_polars<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
                crate::frames(slf.py(), "to_polars", slf.as_any(), None)
            }

            /// The columns as a pandas data frame (needs `pandas`); labels
            /// and statuses are categorical.
            fn to_pandas<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
                crate::frames(slf.py(), "to_pandas", slf.as_any(), None)
            }

            $($rest)*
        }
    };
}

/// Labels of a batch of points ([`Model.classify`]): one label id per
/// point, into `labels`, and columns gathered from them. `result[i]` is the
/// label of point `i`.
#[pyclass(frozen, module = "llevant")]
pub struct Classification {
    table: LabelTable,
    ids: Py<PyArray1<u32>>,
}

impl Classification {
    pub fn new(
        py: Python<'_>,
        model: Arc<llevant::Model>,
        c: llevant::Classification,
    ) -> PyResult<Classification> {
        Ok(Classification {
            table: LabelTable::new(model, c.labels),
            ids: frozen_array(py, c.ids)?.unbind(),
        })
    }
}

label_columns!(Classification {
    /// Number of points.
    fn __len__(&self, py: Python<'_>) -> usize {
        PyUntypedArrayMethods::len(self.ids.bind(py))
    }

    /// The label of point `i`.
    fn __getitem__(&self, py: Python<'_>, i: isize) -> PyResult<Option<Py<Label>>> {
        let ids = ids_of(py, &self.ids)?;
        self.table.label(py, ids[index(i, ids.len())?])
    }

    /// Columns for data frames: label (code), status, cell, material,
    /// universe.
    fn _columns<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        self.table.columns(py, ids_of(py, &self.ids)?, &d)?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        format!(
            "<Classification: {} points, {} labels>",
            PyUntypedArrayMethods::len(self.ids.bind(py)),
            self.table.labels.len()
        )
    }
});

/// Segments of a batch of lines ([`Model.trace_lines`]), in ragged form:
/// line `i` has segments `offsets[i]:offsets[i + 1]`, in order, covering its
/// range (gaps included).
#[pyclass(frozen, module = "llevant")]
pub struct Tracing {
    table: LabelTable,
    ids: Py<PyArray1<u32>>,
    offsets: Py<PyArray1<usize>>,
    t0: Py<PyArray1<f64>>,
    t1: Py<PyArray1<f64>>,
    surface_in: Py<PyArray1<i64>>,
    surface_out: Py<PyArray1<i64>>,
    facet_in: Py<PyArray1<u8>>,
    facet_out: Py<PyArray1<u8>>,
}

/// MCNP surface numbers (-1 for none) and facets of compiled shapes.
fn surfaces(m: &llevant::Model, shapes: &[u32]) -> (Vec<i64>, Vec<u8>) {
    shapes
        .iter()
        .map(
            |&s| match (s != NO_SURFACE).then(|| m.surface_id(s)).flatten() {
                Some(id) => (id.id, id.facet),
                None => (-1, 0),
            },
        )
        .unzip()
}

impl Tracing {
    pub fn new(
        py: Python<'_>,
        model: Arc<llevant::Model>,
        t: llevant::Tracing,
    ) -> PyResult<Tracing> {
        let (si, fi) = surfaces(&model, &t.surface_in);
        let (so, fo) = surfaces(&model, &t.surface_out);
        Ok(Tracing {
            table: LabelTable::new(model, t.labels),
            ids: frozen_array(py, t.ids)?.unbind(),
            offsets: frozen_array(py, t.offsets)?.unbind(),
            t0: frozen_array(py, t.t0)?.unbind(),
            t1: frozen_array(py, t.t1)?.unbind(),
            surface_in: frozen_array(py, si)?.unbind(),
            surface_out: frozen_array(py, so)?.unbind(),
            facet_in: frozen_array(py, fi)?.unbind(),
            facet_out: frozen_array(py, fo)?.unbind(),
        })
    }

    fn offsets<'a>(&'a self, py: Python<'a>) -> PyResult<&'a [usize]> {
        slice(py, &self.offsets)
    }

    fn line_count(&self, py: Python<'_>) -> PyResult<usize> {
        Ok(self.offsets(py)?.len().saturating_sub(1))
    }
}

label_columns!(Tracing {
    /// Start of each line's segments (`uint64`, one more than the lines).
    #[getter(offsets)]
    fn get_offsets(&self, py: Python<'_>) -> Py<PyArray1<usize>> {
        self.offsets.clone_ref(py)
    }

    /// `t` (cm along the line) where each segment starts.
    #[getter]
    fn t0(&self, py: Python<'_>) -> Py<PyArray1<f64>> {
        self.t0.clone_ref(py)
    }

    /// `t` where each segment ends.
    #[getter]
    fn t1(&self, py: Python<'_>) -> Py<PyArray1<f64>> {
        self.t1.clone_ref(py)
    }

    /// Surface each segment is entered through (MCNP number, `int64`), -1 at
    /// the start of a line.
    #[getter]
    fn surface_in(&self, py: Python<'_>) -> Py<PyArray1<i64>> {
        self.surface_in.clone_ref(py)
    }

    /// Surface each segment is left through, -1 at the end of a line.
    #[getter]
    fn surface_out(&self, py: Python<'_>) -> Py<PyArray1<i64>> {
        self.surface_out.clone_ref(py)
    }

    /// Macrobody facet of `surface_in` (`uint8`, 0 for ordinary surfaces).
    #[getter]
    fn facet_in(&self, py: Python<'_>) -> Py<PyArray1<u8>> {
        self.facet_in.clone_ref(py)
    }

    /// Macrobody facet of `surface_out`.
    #[getter]
    fn facet_out(&self, py: Python<'_>) -> Py<PyArray1<u8>> {
        self.facet_out.clone_ref(py)
    }

    /// Line of each segment (`int64`).
    #[getter]
    fn line<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray1<i64>>> {
        let off = self.offsets(py)?;
        let mut v = Vec::with_capacity(off.last().copied().unwrap_or(0));
        for (i, w) in off.windows(2).enumerate() {
            v.extend(std::iter::repeat_n(i as i64, w[1] - w[0]));
        }
        Ok(v.into_pyarray(py))
    }

    /// Number of lines.
    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        self.line_count(py)
    }

    /// The segments of line `i`.
    fn __getitem__(&self, py: Python<'_>, i: isize) -> PyResult<Vec<Segment>> {
        let i = index(i, self.line_count(py)?)?;
        let off = self.offsets(py)?;
        let ids = ids_of(py, &self.ids)?;
        let (t0, t1) = (f64s(py, &self.t0)?, f64s(py, &self.t1)?);
        let (si, so) = (slice(py, &self.surface_in)?, slice(py, &self.surface_out)?);
        let (fi, fo) = (slice(py, &self.facet_in)?, slice(py, &self.facet_out)?);
        let surf = |s: i64, f: u8| {
            (s >= 0).then(|| Surface::new(llevant::SurfaceId { id: s, facet: f }))
        };
        (off[i]..off[i + 1])
            .map(|k| {
                Ok(Segment {
                    t0: t0[k],
                    t1: t1[k],
                    label: self
                        .table
                        .label(py, ids[k])?
                        .expect("segments have labels"),
                    surface_in: surf(si[k], fi[k]),
                    surface_out: surf(so[k], fo[k]),
                })
            })
            .collect()
    }

    /// Total length (cm) by `cell`, `material`, `universe` or `label`.
    /// Segments not inside a cell are keyed by their status name (`gap`,
    /// `overlap`...) except with `by="label"`, where every key is a label.
    #[pyo3(signature = (by = "cell"))]
    fn lengths<'py>(&self, py: Python<'py>, by: &str) -> PyResult<Bound<'py, PyDict>> {
        let ids = ids_of(py, &self.ids)?;
        let (t0, t1) = (f64s(py, &self.t0)?, f64s(py, &self.t1)?);
        let mut per = vec![0.0; self.table.labels.len()];
        for k in 0..ids.len() {
            per[ids[k] as usize] += t1[k] - t0[k];
        }
        let m = &self.table.model;
        let d = PyDict::new(py);
        let add = |key: Bound<'py, PyAny>, x: f64| -> PyResult<()> {
            let old: f64 = match d.get_item(&key)? {
                Some(v) => v.extract()?,
                None => 0.0,
            };
            d.set_item(key, old + x)
        };
        let by = crate::convert::group(by)?;
        for (l, (label, &x)) in self.table.labels.iter().zip(&per).enumerate() {
            if x == 0.0 && label.status != Status::Inside {
                continue;
            }
            let number = label.cell().map(|c| {
                let cell = m.cell(c);
                match by {
                    llevant::GroupBy::Cell => cell.id,
                    llevant::GroupBy::Material => cell.material,
                    _ => m.units()[cell.unit as usize].universe,
                }
            });
            let key = match (by, number) {
                (llevant::GroupBy::Label, _) => self.table.objects(py)?[l].bind(py).clone().into_any(),
                (_, Some(n)) => n.into_pyobject(py)?.into_any(),
                (_, None) => label.status.name().into_pyobject(py)?.into_any(),
            };
            add(key, x)?;
        }
        Ok(d)
    }

    /// Columns for data frames: line, t0, t1, length, label (code), status,
    /// cell, material, universe, surface_in, facet_in, surface_out, facet_out.
    fn _columns<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        let (t0, t1) = (f64s(py, &self.t0)?, f64s(py, &self.t1)?);
        d.set_item("line", self.line(py)?)?;
        d.set_item("t0", self.t0.bind(py))?;
        d.set_item("t1", self.t1.bind(py))?;
        let len: Vec<f64> = t0.iter().zip(t1).map(|(a, b)| b - a).collect();
        d.set_item("length", len.into_pyarray(py))?;
        self.table.columns(py, ids_of(py, &self.ids)?, &d)?;
        d.set_item("surface_in", self.surface_in.bind(py))?;
        d.set_item("facet_in", self.facet_in.bind(py))?;
        d.set_item("surface_out", self.surface_out.bind(py))?;
        d.set_item("facet_out", self.facet_out.bind(py))?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "<Tracing: {} lines, {} segments, {} labels>",
            self.line_count(py)?,
            PyUntypedArrayMethods::len(self.ids.bind(py)),
            self.table.labels.len()
        ))
    }
});

/// First visible hits of a batch of rays ([`Model.first_hit`]): per ray,
/// `t` where it enters its first visible segment (inf if none), that
/// segment's label id (`NO_HIT` if none), the surface it enters through and
/// the surface's unit normal, facing the ray.
#[pyclass(frozen, module = "llevant")]
pub struct Hits {
    table: LabelTable,
    ids: Py<PyArray1<u32>>,
    t: Py<PyArray1<f64>>,
    surface: Py<PyArray1<i64>>,
    facet: Py<PyArray1<u8>>,
    normal: Py<PyAny>,
    ghost_count: Py<PyArray1<u32>>,
    ghost_cell: Py<PyArray1<i64>>,
}

impl Hits {
    pub fn new(py: Python<'_>, model: Arc<llevant::Model>, h: llevant::Hits) -> PyResult<Hits> {
        let (s, f) = surfaces(&model, &h.surface);
        let n = h.ids.len();
        let (count, cell): (Vec<u32>, Vec<i64>) = if h.ghosts.is_empty() {
            (vec![0; n], vec![-1; n])
        } else {
            h.ghosts
                .iter()
                .map(|g| {
                    let cell = match g.cell {
                        llevant::NO_CELL => -1,
                        c => model.cell_id(c),
                    };
                    (g.count, cell)
                })
                .unzip()
        };
        Ok(Hits {
            ghost_count: frozen_array(py, count)?.unbind(),
            ghost_cell: frozen_array(py, cell)?.unbind(),
            normal: frozen_rows(py, &h.normal)?.unbind(),
            table: LabelTable::new(model, h.labels),
            ids: frozen_array(py, h.ids)?.unbind(),
            t: frozen_array(py, h.t)?.unbind(),
            surface: frozen_array(py, s)?.unbind(),
            facet: frozen_array(py, f)?.unbind(),
        })
    }
}

label_columns!(Hits {
    /// `t` (cm along the ray) of each hit (`float64`), inf where the ray
    /// hit nothing.
    #[getter]
    fn t(&self, py: Python<'_>) -> Py<PyArray1<f64>> {
        self.t.clone_ref(py)
    }

    /// Ghost boundaries each ray crossed before its hit (0 without ghosts).
    #[getter]
    fn ghost_count(&self, py: Python<'_>) -> Py<PyArray1<u32>> {
        self.ghost_count.clone_ref(py)
    }

    /// MCNP number of the ghost cell at each ray's first ghost boundary,
    /// -1 for none.
    #[getter]
    fn ghost_cell(&self, py: Python<'_>) -> Py<PyArray1<i64>> {
        self.ghost_cell.clone_ref(py)
    }

    /// Whether each ray hit something visible.
    #[getter]
    fn hit<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray1<bool>>> {
        let ids = ids_of(py, &self.ids)?;
        Ok(ids
            .iter()
            .map(|&i| i != NO_HIT)
            .collect::<Vec<_>>()
            .into_pyarray(py))
    }

    /// Surface entered (MCNP number, `int64`), -1 when the ray starts inside
    /// or misses.
    #[getter]
    fn surface(&self, py: Python<'_>) -> Py<PyArray1<i64>> {
        self.surface.clone_ref(py)
    }

    /// Macrobody facet of `surface` (`uint8`, 0 for ordinary surfaces).
    #[getter]
    fn facet(&self, py: Python<'_>) -> Py<PyArray1<u8>> {
        self.facet.clone_ref(py)
    }

    /// Unit normal of the surface entered, `(N, 3)`, zero without one.
    #[getter]
    fn normal(&self, py: Python<'_>) -> Py<PyAny> {
        self.normal.clone_ref(py)
    }

    /// Number of rays.
    fn __len__(&self, py: Python<'_>) -> usize {
        PyUntypedArrayMethods::len(self.ids.bind(py))
    }

    /// Label hit by ray `i`, None if it hit nothing.
    fn __getitem__(&self, py: Python<'_>, i: isize) -> PyResult<Option<Py<Label>>> {
        let ids = ids_of(py, &self.ids)?;
        self.table.label(py, ids[index(i, ids.len())?])
    }

    /// Columns for data frames: t, label (code, -1 for a miss), status,
    /// cell, material, universe, surface, facet.
    fn _columns<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("t", self.t.bind(py))?;
        self.table.columns(py, ids_of(py, &self.ids)?, &d)?;
        d.set_item("surface", self.surface.bind(py))?;
        d.set_item("facet", self.facet.bind(py))?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let ids = ids_of(py, &self.ids)?;
        Ok(format!(
            "<Hits: {} rays, {} hit, {} labels>",
            ids.len(),
            ids.iter().filter(|&&i| i != NO_HIT).count(),
            self.table.labels.len()
        ))
    }
});
