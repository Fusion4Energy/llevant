//! Slice and render images: RGB pixels, the label under each pixel, PNG
//! output (shown inline by Jupyter).

use std::sync::Arc;

use llevant::{Camera, Slice, Status, Visibility, NO_HIT};
use llevant_render::colours::{self, ColourBy};
use llevant_scene::materials::Looks;
use numpy::{IntoPyArray, PyArray2, PyArrayMethods};
use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::types::PyBytes;

use crate::batch::LabelTable;
use crate::convert::{err, invalid, xyz, Xyz};
use crate::results::Label;

enum Source {
    Slice(Slice),
    /// CPU render: `t` of each pixel's hit (inf for none).
    Render {
        camera: Camera,
        t: Vec<f64>,
    },
    /// GPU render: the labels are not kept, so [`Image::at`] casts the ray
    /// again on the CPU.
    Gpu {
        camera: Camera,
        vis: Visibility,
        t: Vec<f64>,
    },
}

/// An image of a model: a slice ([`Model.slice`]) or a render
/// ([`Model.render`]). Evaluating it in Jupyter shows it; `save()` writes a
/// PNG; `at(i, j)` tells what is under a pixel (column `i`, row `j` from
/// the top).
#[pyclass(frozen, module = "llevant")]
pub struct Image {
    model: Arc<llevant::Model>,
    width: usize,
    height: usize,
    rgb: Vec<u8>,
    /// MCNP cell under each pixel, -1 for none.
    cells: Vec<i64>,
    /// Label ids of each pixel and their labels (CPU images).
    labels: Option<(LabelTable, Vec<u32>)>,
    source: Source,
    png: GILOnceCell<Py<PyBytes>>,
}

impl Image {
    #[allow(clippy::too_many_arguments)]
    pub fn slice(
        model: Arc<llevant::Model>,
        looks: &Looks,
        field: Option<&llevant_render::field::Overlay>,
        s: Slice,
        tr: llevant::Tracing,
        by: ColourBy,
        outline: Option<colours::Outlines>,
        level: Option<usize>,
    ) -> Image {
        let ids = s.pixels(&tr);
        let paint = colours::Paint::new(&model, looks, by)
            .with_field(field)
            .with_level(level);
        let rgb = colours::slice_rgb(&paint, &s, &tr.labels, &ids, outline);
        let table = LabelTable::new(model.clone(), tr.labels);
        Image {
            width: s.nx,
            height: s.ny,
            rgb,
            cells: table.cell(&ids),
            labels: Some((table, ids)),
            source: Source::Slice(s),
            model,
            png: GILOnceCell::new(),
        }
    }

    /// A CPU render: the hits and their image.
    pub fn render(
        model: Arc<llevant::Model>,
        camera: Camera,
        h: llevant::Hits,
        rgb: Vec<u8>,
    ) -> Image {
        let table = LabelTable::new(model.clone(), h.labels);
        Image {
            width: camera.width,
            height: camera.height,
            rgb,
            cells: table.cell(&h.ids),
            labels: Some((table, h.ids)),
            source: Source::Render { camera, t: h.t },
            model,
            png: GILOnceCell::new(),
        }
    }

    /// A GPU render: the frame and its image.
    pub fn gpu(
        model: Arc<llevant::Model>,
        camera: Camera,
        vis: Visibility,
        f: &llevant_gpu::Frame,
        rgb: Vec<u8>,
    ) -> Image {
        let cells = f
            .hits
            .iter()
            .map(|h| match h.status {
                llevant_gpu::Status::Inside if h.cell != llevant_gpu::NONE => model.cell_id(h.cell),
                _ => -1,
            })
            .collect();
        let t = f
            .hits
            .iter()
            .map(|h| {
                if h.is_hit() {
                    h.t as f64
                } else {
                    f64::INFINITY
                }
            })
            .collect();
        Image {
            width: camera.width,
            height: camera.height,
            rgb,
            cells,
            labels: None,
            source: Source::Gpu { camera, vis, t },
            model,
            png: GILOnceCell::new(),
        }
    }

    fn pixel(&self, i: usize, j: usize) -> PyResult<usize> {
        if i >= self.width || j >= self.height {
            return Err(invalid(format!(
                "pixel ({i}, {j}) outside the {}×{} image",
                self.width, self.height
            )));
        }
        Ok(j * self.width + i)
    }

    fn png_bytes<'py>(&self, py: Python<'py>) -> PyResult<&Bound<'py, PyBytes>> {
        let p = self.png.get_or_try_init(py, || {
            let b = llevant_render::png::png_bytes(self.width, self.height, &self.rgb)
                .map_err(invalid)?;
            Ok::<_, PyErr>(PyBytes::new(py, &b).unbind())
        })?;
        Ok(p.bind(py))
    }

    fn shape<T: numpy::Element>(&self, py: Python<'_>, v: Vec<T>) -> PyResult<Py<PyArray2<T>>> {
        Ok(v.into_pyarray(py)
            .reshape([self.height, self.width])?
            .unbind())
    }
}

#[pymethods]
impl Image {
    /// Width in pixels.
    #[getter]
    fn width(&self) -> usize {
        self.width
    }

    /// Height in pixels.
    #[getter]
    fn height(&self) -> usize {
        self.height
    }

    /// `slice` or `render`.
    #[getter]
    fn kind(&self) -> &'static str {
        match self.source {
            Source::Slice(_) => "slice",
            _ => "render",
        }
    }

    /// The pixels, `(height, width, 3)` `uint8`, rows from the top (for
    /// `matplotlib.pyplot.imshow`).
    #[getter]
    fn rgb<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        Ok(self
            .rgb
            .clone()
            .into_pyarray(py)
            .reshape([self.height, self.width, 3])?
            .into_any())
    }

    /// MCNP cell under each pixel, `(height, width)` `int64`, -1 for none.
    #[getter]
    fn cell(&self, py: Python<'_>) -> PyResult<Py<PyArray2<i64>>> {
        self.shape(py, self.cells.clone())
    }

    /// Label id under each pixel, `(height, width)` `uint32` into `labels`
    /// (`NO_HIT` where a ray hits nothing); None for GPU renders.
    #[getter]
    fn ids(&self, py: Python<'_>) -> PyResult<Option<Py<PyArray2<u32>>>> {
        self.labels
            .as_ref()
            .map(|(_, ids)| self.shape(py, ids.clone()))
            .transpose()
    }

    /// The distinct labels in the image (empty for GPU renders).
    #[getter]
    fn labels(&self, py: Python<'_>) -> PyResult<Vec<Py<Label>>> {
        match &self.labels {
            Some((t, _)) => Ok(t.objects(py)?.iter().map(|l| l.clone_ref(py)).collect()),
            None => Ok(vec![]),
        }
    }

    /// What is under pixel `(i, j)` (column `i`, row `j` from the top): the
    /// label at the pixel centre (slices) or the first one the ray hits
    /// (renders; None if it hits nothing).
    fn at(&self, py: Python<'_>, i: usize, j: usize) -> PyResult<Option<Py<Label>>> {
        let k = self.pixel(i, j)?;
        if let Some((t, ids)) = &self.labels {
            return t.label(py, ids[k]);
        }
        let Source::Gpu { camera, vis, .. } = &self.source else {
            unreachable!("CPU images keep their labels")
        };
        let Some(ray) = camera.ray(i, j) else {
            return Ok(None);
        };
        let m = &self.model;
        let h = py
            .allow_threads(|| m.first_hits(&[ray], vis, None))
            .map_err(err)?;
        if h.ids[0] == NO_HIT {
            return Ok(None);
        }
        Ok(Some(Py::new(
            py,
            Label::new(m, &h.labels[h.ids[0] as usize]),
        )?))
    }

    /// The point under pixel `(i, j)`: its centre on the plane (slices) or
    /// where its ray hits (renders; None if it hits nothing).
    fn point(&self, i: usize, j: usize) -> PyResult<Option<Xyz>> {
        let k = self.pixel(i, j)?;
        Ok(match &self.source {
            Source::Slice(s) => Some(xyz(s.pixel_centre(i, j))),
            Source::Render { camera, t } | Source::Gpu { camera, t, .. } => {
                t[k].is_finite().then(|| {
                    let d = camera.direction(i, j);
                    xyz([0, 1, 2].map(|a| camera.eye[a] + t[k] * d[a]))
                })
            }
        })
    }

    /// Statuses found in the image other than `inside` (gaps, overlaps...),
    /// by number of pixels.
    fn problems(&self, py: Python<'_>) -> PyResult<Vec<(Py<Label>, usize)>> {
        let Some((t, ids)) = &self.labels else {
            return Ok(vec![]);
        };
        let mut count = vec![0usize; t.labels.len()];
        for &i in ids {
            if i != NO_HIT {
                count[i as usize] += 1;
            }
        }
        let mut out: Vec<(usize, usize)> = t
            .labels
            .iter()
            .enumerate()
            .filter(|(k, l)| {
                count[*k] > 0 && l.status != Status::Inside && l.status != Status::OnSurface
            })
            .map(|(k, _)| (k, count[k]))
            .collect();
        out.sort_by_key(|&(k, n)| (std::cmp::Reverse(n), k));
        let o = t.objects(py)?;
        Ok(out
            .into_iter()
            .map(|(k, n)| (o[k].clone_ref(py), n))
            .collect())
    }

    /// The image as PNG bytes.
    fn png<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        self.png_bytes(py).cloned()
    }

    fn _repr_png_<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        self.png(py)
    }

    /// Write the image as a PNG file.
    fn save(&self, py: Python<'_>, path: std::path::PathBuf) -> PyResult<()> {
        let b = self.png_bytes(py)?;
        std::fs::write(&path, b.as_bytes())
            .map_err(|e| pyo3::exceptions::PyOSError::new_err(format!("{}: {e}", path.display())))
    }

    fn __repr__(&self) -> String {
        format!("<Image {} {}×{}>", self.kind(), self.width, self.height)
    }
}
