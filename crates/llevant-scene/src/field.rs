//! Fields: numbers over the geometry (results of a transport run, or
//! anything else per cell, per cell instance or per mesh voxel). This is
//! their data; how they are drawn over slices and renders (a colour scale,
//! an opacity) is `llevant_render::field::FieldLook`.
//!
//! What the numbers are attached to ([`Values`]):
//!
//! - **cells**: one value per cell, in every instance of it;
//! - **instances**: one value per cell instance ([`llevant::Instances`]),
//!   so that each copy of a repeated universe and each lattice element can
//!   differ;
//! - **a mesh** (rectangular or cylindrical, placed by MCNP's `TR=`, as
//!   [`aci_tallies_geom::Geometry`]): one value per voxel, or per voxel and
//!   cell (cell under voxel: the part of a voxel in each cell has a value
//!   of its own, as D1SUNED and R2SUNED give them).
//!
//! Fields are made in code ([`Field::cells`], [`Field::instances`],
//! [`Field::mesh`]) or read from files ([`read_csv`], and with the
//! `tallies` feature [`read_tallies`] for MCNP tally files: meshtal files
//! (FMESH tallies, on their meshes) and MCTAL files (`F` cell tallies, per
//! cell or per instance); or [`TallyFile::open`] and [`tally_field`] to
//! read several tallies or bins of one file).

use std::path::Path;

use aci_tallies_geom::Geometry;
use llevant::vec3::Vec3;
use llevant::{Error, Label, Model, Result, Status};

/// What a field's numbers are attached to.
#[derive(Clone, Debug)]
pub enum Values {
    /// Per cell index ([`Model::cells`]); NaN where there is none.
    Cells(Vec<f32>),
    /// Per cell instance id ([`Model::instances`]); NaN where there is none.
    Instances(Vec<f32>),
    Mesh(Box<Mesh>),
}

/// Values on a structured mesh.
#[derive(Clone, Debug)]
pub struct Mesh {
    pub geometry: Geometry,
    /// Per voxel, in the mesh's flat order; NaN where there is none.
    pub values: Vec<f32>,
    /// Values per voxel and cell, which take the place of `values` where
    /// given.
    pub by_cell: Option<CellUnderVoxel>,
}

/// Values per (voxel, cell): for voxel `v`, the pairs
/// `cells[offsets[v] .. offsets[v + 1]]` and `values[..]` alike.
#[derive(Clone, Debug, Default)]
pub struct CellUnderVoxel {
    pub offsets: Vec<u32>,
    /// Cell indices ([`Model::cells`]).
    pub cells: Vec<u32>,
    pub values: Vec<f32>,
}

impl CellUnderVoxel {
    /// From `(voxel, cell index, value)` triples, in any order, for a mesh
    /// of `voxels` voxels.
    pub fn new(voxels: usize, mut entries: Vec<(usize, u32, f32)>) -> Result<CellUnderVoxel> {
        if let Some(e) = entries.iter().find(|e| e.0 >= voxels) {
            return Err(Error::Invalid(format!(
                "voxel {} of a mesh of {voxels}",
                e.0
            )));
        }
        entries.sort_by_key(|e| (e.0, e.1));
        let mut offsets = Vec::with_capacity(voxels + 1);
        let mut k = 0;
        for v in 0..voxels {
            offsets.push(k as u32);
            while k < entries.len() && entries[k].0 == v {
                k += 1;
            }
        }
        offsets.push(k as u32);
        Ok(CellUnderVoxel {
            offsets,
            cells: entries.iter().map(|e| e.1).collect(),
            values: entries.iter().map(|e| e.2).collect(),
        })
    }

    /// The value of the part of `voxel` in `cell`.
    pub fn get(&self, voxel: usize, cell: u32) -> Option<f32> {
        let (a, b) = (
            *self.offsets.get(voxel)? as usize,
            *self.offsets.get(voxel + 1)? as usize,
        );
        let k = self.cells[a..b].binary_search(&cell).ok()?;
        Some(self.values[a + k])
    }
}

/// Numbers over the geometry.
#[derive(Clone, Debug)]
pub struct Field {
    /// What the numbers are (for legends).
    pub name: String,
    /// Their unit (for legends; may be empty).
    pub unit: String,
    pub values: Values,
}

impl Field {
    pub fn new(name: impl Into<String>, unit: impl Into<String>, values: Values) -> Field {
        Field {
            name: name.into(),
            unit: unit.into(),
            values,
        }
    }

    /// Values per cell, by MCNP cell number. Unknown numbers are an error.
    pub fn cells(
        m: &Model,
        name: impl Into<String>,
        values: impl IntoIterator<Item = (i64, f64)>,
    ) -> Result<Field> {
        let mut v = vec![f32::NAN; m.cells().len()];
        for (id, x) in values {
            let c = m.cell_index(id).ok_or(Error::NoCell(id))?;
            v[c as usize] = x as f32;
        }
        Ok(Field::new(name, "", Values::Cells(v)))
    }

    /// Values per cell of each material (every cell of it, filled ones
    /// left out), by material number.
    pub fn materials(
        m: &Model,
        name: impl Into<String>,
        values: impl IntoIterator<Item = (i64, f64)>,
    ) -> Result<Field> {
        let by: rustc_hash::FxHashMap<i64, f64> = values.into_iter().collect();
        let v = m
            .cells()
            .iter()
            .map(|c| match by.get(&c.material) {
                Some(&x) if c.fill.is_none() => x as f32,
                _ => f32::NAN,
            })
            .collect();
        Ok(Field::new(name, "", Values::Cells(v)))
    }

    /// Values per cell instance, by id ([`Model::instances`]). Ids past the
    /// model's are an error.
    pub fn instances(
        m: &Model,
        name: impl Into<String>,
        values: impl IntoIterator<Item = (u64, f64)>,
    ) -> Result<Field> {
        let total = m.instances().total();
        if total > 1 << 31 {
            return Err(Error::Invalid(format!(
                "{total} instances are too many for a field per instance"
            )));
        }
        let mut v = vec![f32::NAN; total as usize];
        for (id, x) in values {
            *v.get_mut(id as usize)
                .ok_or_else(|| Error::Invalid(format!("instance {id} of a model of {total}")))? =
                x as f32;
        }
        Ok(Field::new(name, "", Values::Instances(v)))
    }

    /// Values per voxel of a mesh, in its flat order; with `by_cell`,
    /// values per voxel and cell where given.
    pub fn mesh(
        name: impl Into<String>,
        unit: impl Into<String>,
        geometry: Geometry,
        values: Vec<f32>,
        by_cell: Option<CellUnderVoxel>,
    ) -> Result<Field> {
        geometry
            .validate()
            .map_err(|e| Error::Invalid(format!("mesh: {e}")))?;
        let n = geometry.n_voxels();
        if values.len() != n {
            return Err(Error::Invalid(format!(
                "mesh of {n} voxels given {} values",
                values.len()
            )));
        }
        if let Some(b) = &by_cell {
            if b.offsets.len() != n + 1 {
                return Err(Error::Invalid(format!(
                    "values by cell for {} voxels on a mesh of {n}",
                    b.offsets.len().saturating_sub(1)
                )));
            }
        }
        Ok(Field::new(
            name,
            unit,
            Values::Mesh(Box::new(Mesh {
                geometry,
                values,
                by_cell,
            })),
        ))
    }

    /// Every number the field holds (NaN ones included).
    pub fn numbers(&self) -> Box<dyn Iterator<Item = f32> + '_> {
        match &self.values {
            Values::Cells(v) | Values::Instances(v) => Box::new(v.iter().copied()),
            Values::Mesh(m) => Box::new(
                m.values
                    .iter()
                    .chain(m.by_cell.iter().flat_map(|b| b.values.iter()))
                    .copied(),
            ),
        }
    }

    /// Whether the value can change within one cell instance (a mesh), so
    /// that it must be looked up per point rather than per label.
    pub fn per_point(&self) -> bool {
        matches!(self.values, Values::Mesh(_))
    }

    /// The value at `point`, in cell `cell` (index) and its instance
    /// `instance`, if the field has one there.
    pub fn value_at(&self, cell: u32, instance: Option<u64>, point: Vec3) -> Option<f64> {
        let v = match &self.values {
            Values::Cells(v) => *v.get(cell as usize)?,
            Values::Instances(v) => *v.get(instance? as usize)?,
            Values::Mesh(m) => {
                let voxel = m.geometry.locate(point)?;
                match m.by_cell.as_ref().and_then(|b| b.get(voxel, cell)) {
                    Some(x) => x,
                    None if m.by_cell.is_some() => return None,
                    None => m.values[voxel],
                }
            }
        };
        v.is_finite().then_some(v as f64)
    }

    /// The value at `point`, whose label is `l` (only points inside a cell
    /// have one).
    pub fn value(&self, m: &Model, l: &Label, point: Vec3) -> Option<f64> {
        if l.status != Status::Inside {
            return None;
        }
        let cell = l.cell()?;
        let instance = match self.values {
            Values::Instances(_) => m.instance(l),
            _ => None,
        };
        self.value_at(cell, instance, point)
    }

    /// The legend's title: the name, and the unit in brackets.
    pub fn title(&self) -> String {
        if self.unit.is_empty() {
            self.name.clone()
        } else {
            format!("{} [{}]", self.name, self.unit)
        }
    }
}

/// A short label for the number `v` (scale ticks, tally bin bounds): up to
/// three decimals between 0.01 and 10⁴, else a mantissa and an exponent.
pub fn tick_label(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let e = v.abs().log10().floor() as i32;
    let m = v / 10f64.powi(e);
    if (-2..=4).contains(&e) && !((m - 1.0).abs() < 1e-9 && e < 0) {
        let s = format!("{:.3}", v);
        return s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    let m = format!("{m:.2}");
    let m = m.trim_end_matches('0').trim_end_matches('.');
    format!("{m}e{e}")
}

/// A field from a CSV file whose first column says what the rows are, by
/// its header:
///
/// - `cell`: MCNP cell numbers;
/// - `material`: material numbers (every unfilled cell of each);
/// - `instance`: cell instance ids ([`Model::instances`]);
/// - `path`: cell instances in MCNP's repeated-structure notation
///   (`12 < 100[1 0 0] < 5`, [`Model::parse_path`]).
///
/// The second column holds the values; its header is the field's name, with
/// the unit in brackets if given (`dose [Sv/h]`). Fields may be quoted;
/// lines starting with `#` and empty values are skipped.
pub fn read_csv(m: &Model, path: &Path) -> Result<Field> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let at = |line: usize, why: String| Error::Invalid(format!("{}:{line}: {why}", path.display()));
    let mut rows = text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
    let (hl, header) = rows.next().ok_or_else(|| {
        at(
            1,
            "no header (cell, material, instance or path; then a value column)".into(),
        )
    })?;
    let head = split_csv(header);
    if head.len() < 2 {
        return Err(at(hl + 1, "give two columns: what, value".into()));
    }
    let (name, unit) = name_and_unit(&head[1]);
    let mut pairs: Vec<(String, f64, usize)> = Vec::new();
    for (k, line) in rows {
        let f = split_csv(line);
        if f.len() < 2 || f[1].trim().is_empty() {
            continue;
        }
        let v: f64 = f[1]
            .trim()
            .parse()
            .map_err(|_| at(k + 1, format!("`{}` is not a number", f[1].trim())))?;
        pairs.push((f[0].trim().to_string(), v, k + 1));
    }
    let int = |s: &str, line: usize| -> Result<i64> {
        s.parse()
            .map_err(|_| at(line, format!("`{s}` is not a number")))
    };
    let mut field = match head[0].trim().to_ascii_lowercase().as_str() {
        "cell" => {
            let v: Vec<(i64, f64)> = pairs
                .iter()
                .map(|(k, x, l)| Ok((int(k, *l)?, *x)))
                .collect::<Result<_>>()?;
            Field::cells(m, name, v)?
        }
        "material" => {
            let v: Vec<(i64, f64)> = pairs
                .iter()
                .map(|(k, x, l)| Ok((int(k, *l)?, *x)))
                .collect::<Result<_>>()?;
            Field::materials(m, name, v)?
        }
        "instance" => {
            let v: Vec<(u64, f64)> = pairs
                .iter()
                .map(|(k, x, l)| {
                    let id = k
                        .parse()
                        .map_err(|_| at(*l, format!("`{k}` is not an instance id")))?;
                    Ok((id, *x))
                })
                .collect::<Result<_>>()?;
            Field::instances(m, name, v)?
        }
        "path" => {
            let inst = m.instances();
            let v: Vec<(u64, f64)> = pairs
                .iter()
                .map(|(k, x, l)| {
                    let steps = m.parse_path(k).map_err(|e| at(*l, e.to_string()))?;
                    let id = inst
                        .of(m, &steps)
                        .ok_or_else(|| at(*l, format!("`{k}` has no instance id")))?;
                    Ok((id, *x))
                })
                .collect::<Result<_>>()?;
            Field::instances(m, name, v)?
        }
        other => {
            return Err(at(
                hl + 1,
                format!("first column `{other}`: give cell, material, instance or path"),
            ))
        }
    };
    field.unit = unit;
    Ok(field)
}

/// `dose [Sv/h]` or `dose (Sv/h)` as name and unit.
fn name_and_unit(s: &str) -> (String, String) {
    let s = s.trim();
    for (open, close) in [('[', ']'), ('(', ')')] {
        if let (Some(a), true) = (s.rfind(open), s.ends_with(close)) {
            return (
                s[..a].trim().to_string(),
                s[a + 1..s.len() - 1].trim().to_string(),
            );
        }
    }
    (s.to_string(), String::new())
}

/// The fields of a CSV line (double quotes around a field keep its commas).
fn split_csv(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                out.last_mut().expect("one field").push('"');
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(String::new()),
            c => out.last_mut().expect("one field").push(c),
        }
    }
    out
}

/// A field from a file: an MCNP tally file, meshtal, MCTAL or D1SUNED
/// cuvmsh (by its contents, [`tally_format`]; [`read_tallies`], with
/// `input` for what the file leaves out), or else a CSV file
/// ([`read_csv`]).
pub fn read(m: &Model, path: &Path, input: Option<&Path>, choice: &TallyChoice) -> Result<Field> {
    #[cfg(feature = "tallies")]
    if tally_format(path)?.is_some() {
        return read_tallies(m, path, input, choice);
    }
    let _ = (input, choice);
    read_csv(m, path)
}

/// Which bin of a tally along one of its dimensions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bin {
    /// MCNP's Total, or the only bin when there is one.
    #[default]
    Total,
    Index(usize),
    /// The bins added up, errors in quadrature as if independent (MCNP's
    /// Total when it printed one, which has the right error).
    Sum,
}

impl std::str::FromStr for Bin {
    type Err = Error;
    fn from_str(s: &str) -> Result<Bin> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("total") {
            return Ok(Bin::Total);
        }
        if s.eq_ignore_ascii_case("sum") {
            return Ok(Bin::Sum);
        }
        s.parse()
            .map(Bin::Index)
            .map_err(|_| Error::Invalid(format!("bin `{s}`: give `total`, `sum` or an index")))
    }
}

/// As [`Bin`]'s `from_str` reads it: `total`, `sum` or the index.
impl std::fmt::Display for Bin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Bin::Total => f.write_str("total"),
            Bin::Sum => f.write_str("sum"),
            Bin::Index(k) => write!(f, "{k}"),
        }
    }
}

/// The dimensions of an MCTAL tally other than its cells (`f`), energy and
/// time: direct or flagged, user, segment, multiplier and cosine bins, in
/// the order of [`TallyChoice::other`].
pub const OTHER_DIMS: [char; 5] = ['d', 'u', 's', 'm', 'c'];

/// What to read from a tally file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TallyChoice {
    /// The tally number; `None` for the first in the file (for MCTAL, the
    /// first that can be drawn).
    pub tally: Option<u32>,
    pub energy: Bin,
    pub time: Bin,
    /// MCTAL: the bins along [`OTHER_DIMS`].
    pub other: [Bin; 5],
    /// The relative error instead of the result.
    pub error: bool,
}

impl TallyChoice {
    /// Set the bin along dimension `dim`: `e` (energy), `t` (time) or one
    /// of [`OTHER_DIMS`].
    pub fn set_bin(&mut self, dim: &str, bin: Bin) -> Result<()> {
        let d = dim.trim().to_ascii_lowercase();
        match d.as_str() {
            "e" => self.energy = bin,
            "t" => self.time = bin,
            _ => {
                let k = OTHER_DIMS
                    .iter()
                    .position(|c| d.len() == 1 && d.starts_with(*c))
                    .ok_or_else(|| {
                        Error::Invalid(format!("bin of `{dim}`: give d, u, s, m, c, e or t"))
                    })?;
                self.other[k] = bin;
            }
        }
        Ok(())
    }

    /// The bins along [`OTHER_DIMS`] that are not Total, as
    /// [`TallyChoice::set_bins`] reads them (`m=0 c=sum`).
    pub fn other_bins(&self) -> String {
        OTHER_DIMS
            .iter()
            .zip(&self.other)
            .filter(|(_, b)| **b != Bin::Total)
            .map(|(d, b)| format!("{d}={b}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Set bins written as `u=1 s=total` (spaces or commas between them).
    pub fn set_bins(&mut self, s: &str) -> Result<()> {
        for part in s.split([' ', ',']).filter(|p| !p.is_empty()) {
            let (dim, bin) = part
                .split_once('=')
                .ok_or_else(|| Error::Invalid(format!("bin `{part}`: write it as u=1")))?;
            self.set_bin(dim, bin.trim().parse()?)?;
        }
        Ok(())
    }
}

#[cfg(feature = "tallies")]
pub use tallies::*;

#[cfg(feature = "tallies")]
mod tallies {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, PoisonError};

    use aci_tallies::mctal::{self, Detector, Modifier, TallyBin};
    use aci_tallies::{cuv, meshtal, Axis, ReadError};
    use llevant::{Error, Model, Result, Step, NO_BIN};

    use super::{Bin, Field, TallyChoice, Values, OTHER_DIMS};

    pub use aci_tallies::common::Input;
    pub use aci_tallies::Format;

    /// The format of the tally file at `path` from its first 64 kB (only
    /// that much is read): `None` for any other file.
    pub fn tally_format(path: &Path) -> Result<Option<Format>> {
        use std::io::Read;
        let mut head = Vec::with_capacity(1 << 16);
        std::fs::File::open(path)
            .and_then(|f| f.take(1 << 16).read_to_end(&mut head))
            .map_err(io(path))?;
        Ok(aci_tallies::detect(&head))
    }

    /// The MCNP input at `path`, parsed once to open several tally files
    /// of its runs with ([`TallyFile::open`]).
    pub fn read_input(path: &Path) -> Result<Input> {
        Input::read(path).map_err(io(path))
    }

    /// A field from an MCNP tally file ([`TallyFile::open`] and
    /// [`tally_field`]); with `input`, the MCNP input of the run.
    pub fn read_tallies(
        m: &Model,
        path: &Path,
        input: Option<&Path>,
        choice: &TallyChoice,
    ) -> Result<Field> {
        let input = input.map(read_input).transpose()?;
        tally_field(m, &TallyFile::open(path, input.as_ref())?, choice)
    }

    /// An MCNP tally file, indexed: one parallel pass that keeps where each
    /// tally and its values are, not the file; values are read from disk
    /// when a field is made of them ([`tally_field`]).
    ///
    /// A D1SUNED cuvmsh (an `FMESH` with `out=cuv`: each voxel's cells,
    /// each with its flux) is drawn as its voxels' fluxes, what the `FMESH`
    /// without `out=cuv` would have scored (aci_tallies'
    /// `Cuvmsh::to_mesh`). Those are made from every pair of a mesh at
    /// once, so the last mesh read is kept for other bins of it. Its
    /// meshinfo (materials and densities, no values) is not a field.
    pub struct TallyFile {
        path: PathBuf,
        reader: Reader,
    }

    enum Reader {
        Meshtal(meshtal::Reader),
        Mctal(mctal::Reader),
        /// The cuvmsh, and the last mesh read as voxel fluxes.
        Cuv(cuv::Cuvmsh, Mutex<Option<Arc<meshtal::Tally>>>),
    }

    impl TallyFile {
        /// Index the meshtal, MCTAL or cuvmsh file at `path`. With `input`,
        /// the MCNP input of the run: it gives meshtal and cuvmsh meshes
        /// their exact geometry (`FMESH` cards, `TR=` included) and MCTAL
        /// tallies what each of their bins is (the `F` cards; the file names
        /// only the first level of a bin). A meshinfo is an error that says
        /// to open its cuvmsh.
        pub fn open(path: &Path, input: Option<&Input>) -> Result<TallyFile> {
            let format = tally_format(path)?.ok_or_else(|| {
                Error::Invalid(format!(
                    "{}: neither a meshtal, an MCTAL nor a cuvmsh file",
                    path.display()
                ))
            })?;
            let reader = match (format, input) {
                (Format::Meshtal, Some(i)) => {
                    Reader::Meshtal(meshtal::Reader::open_with_input(path, i).map_err(io(path))?)
                }
                (Format::Meshtal, None) => {
                    Reader::Meshtal(meshtal::Reader::open(path).map_err(io(path))?)
                }
                (Format::Mctal, Some(i)) => {
                    Reader::Mctal(mctal::Reader::open_with_input(path, i).map_err(io(path))?)
                }
                (Format::Mctal, None) => {
                    Reader::Mctal(mctal::Reader::open(path).map_err(io(path))?)
                }
                (Format::Cuvmsh, input) => {
                    let file = match input {
                        Some(i) => cuv::Cuvmsh::open_with_input(path, i),
                        None => cuv::Cuvmsh::open(path),
                    };
                    Reader::Cuv(file.map_err(io(path))?, Mutex::default())
                }
                (Format::Meshinfo, _) => {
                    return Err(Error::Invalid(format!(
                        "{}: a D1SUNED meshinfo says which cells are in each voxel but \
                         holds no tally values: open the run's cuvmsh",
                        path.display()
                    )))
                }
            };
            Ok(TallyFile {
                path: path.to_path_buf(),
                reader,
            })
        }

        pub fn path(&self) -> &Path {
            &self.path
        }

        pub fn format(&self) -> Format {
            match self.reader {
                Reader::Meshtal(_) => Format::Meshtal,
                Reader::Mctal(_) => Format::Mctal,
                Reader::Cuv(..) => Format::Cuvmsh,
            }
        }

        /// Whether the file is still the one indexed (same size and time).
        pub fn is_current(&self) -> bool {
            match &self.reader {
                Reader::Meshtal(r) => r.is_current(),
                Reader::Mctal(r) => r.is_current(),
                Reader::Cuv(r, _) => r.is_current(),
            }
        }

        /// The numbers of the tallies that can be read, in file order.
        pub fn tallies(&self) -> Vec<u32> {
            match &self.reader {
                Reader::Meshtal(r) => r.tallies().map(|t| t.id).collect(),
                Reader::Mctal(r) => r.tallies().map(|t| t.id).collect(),
                Reader::Cuv(r, _) => r.tallies().map(|t| t.id).collect(),
            }
        }

        /// What the file holds, to choose from: the run, and each tally
        /// with its bins (from the index; no values are read).
        pub fn summary(&self) -> FileSummary {
            let warnings = |w: &[aci_tallies::Warning]| w.iter().map(|w| w.to_string()).collect();
            let failed = |f: &mut dyn Iterator<Item = (u32, &str)>| {
                f.map(|(id, why)| (id, why.to_string())).collect()
            };
            match &self.reader {
                Reader::Meshtal(r) => {
                    let h = r.header();
                    FileSummary {
                        format: Format::Meshtal,
                        code: format!("{} {}", h.code, h.version).trim().to_string(),
                        title: h.title.trim().to_string(),
                        histories: h.histories,
                        tallies: r.tallies().map(meshtal_summary).collect(),
                        failed: failed(&mut r.failed()),
                        warnings: Vec::new(),
                        default: r.tallies().next().map(|t| t.id),
                    }
                }
                Reader::Cuv(r, _) => {
                    let h = r.header();
                    FileSummary {
                        format: Format::Cuvmsh,
                        code: format!("{} {}", h.code, h.version).trim().to_string(),
                        title: h.title.trim().to_string(),
                        histories: h.histories,
                        tallies: r.tallies().map(cuv_summary).collect(),
                        failed: failed(&mut r.failed()),
                        warnings: Vec::new(),
                        default: r.tallies().next().map(|t| t.id),
                    }
                }
                Reader::Mctal(r) => {
                    let h = r.header();
                    FileSummary {
                        format: Format::Mctal,
                        code: format!("{} {}", h.code, h.version).trim().to_string(),
                        title: h.title.trim().to_string(),
                        histories: Some(h.histories as f64),
                        tallies: r.tallies().map(mctal_summary).collect(),
                        failed: failed(&mut r.failed()),
                        warnings: warnings(r.warnings()),
                        default: r
                            .tallies()
                            .find(|t| ready(t))
                            .or_else(|| r.tallies().find(|t| not_drawn(t).is_none()))
                            .map(|t| t.id),
                    }
                }
            }
        }
    }

    /// What a tally file holds ([`TallyFile::summary`]).
    #[derive(Clone, Debug)]
    pub struct FileSummary {
        pub format: Format,
        /// The code that wrote it and its version (`mcnp 6.mpi`).
        pub code: String,
        /// The run's title (its input's first line).
        pub title: String,
        pub histories: Option<f64>,
        /// The tallies indexed, in file order.
        pub tallies: Vec<TallySummary>,
        /// Tallies that could not be indexed, and why.
        pub failed: Vec<(u32, String)>,
        /// Problems with the file as a whole.
        pub warnings: Vec<String>,
        /// The tally read when none is chosen ([`TallyChoice::tally`]
        /// `None`): for MCTAL, the first that can be drawn with its Total
        /// bins and has cells it names.
        pub default: Option<u32>,
    }

    impl FileSummary {
        pub fn tally(&self, id: u32) -> Option<&TallySummary> {
            self.tallies.iter().find(|t| t.id == id)
        }
    }

    /// One tally of a file: what it is and its bins.
    #[derive(Clone, Debug)]
    pub struct TallySummary {
        pub id: u32,
        /// As the input would write it: `+F6:n,p`, `*FMESH14:n`.
        pub kind: String,
        /// Its `FC` comment.
        pub comment: Option<String>,
        /// Its cell bins (MCTAL) or voxels (meshtal, cuvmsh).
        pub cells: usize,
        /// Why it can't be drawn, when it can't.
        pub not_drawn: Option<String>,
        /// What a reader should know: bins or a mesh only the run's input
        /// says exactly.
        pub notes: Vec<String>,
        /// Whether the run's input (its F or FMESH card) would draw it
        /// better: bins named, exact mesh.
        pub wants_input: bool,
        /// The axes with more than one bin to choose from (energy `e`,
        /// time `t`, and for MCTAL `d`, `u`, `s`, `m`, `c`).
        pub axes: Vec<AxisSummary>,
        /// MCTAL cell tallies that can be drawn: each cell bin, in file
        /// order.
        pub bins: Vec<BinSummary>,
        pub warnings: Vec<String>,
    }

    /// The result and relative error of each cell bin of an MCTAL tally
    /// ([`tally_field_and_bins`]).
    pub type BinValues = Vec<(f64, f64)>;

    /// One cell bin of an MCTAL tally.
    #[derive(Clone, Debug, PartialEq)]
    pub struct BinSummary {
        /// As MCNP's output names it: `1`, `(1 2)`, `(8<7[0 0 0]<3)`, `T`;
        /// `?` for a bin that only the run's input names (not given).
        pub label: String,
        /// The cells it scores (its first level), whose instances and all
        /// they hold make it; empty for the T bin and unnamed bins.
        pub cells: Vec<i64>,
        /// Whether it scores every instance of `cells`: not for chains
        /// through particular envelopes or lattice elements (nor, without
        /// the input, for a cell the file names in several bins: chains).
        pub whole_cells: bool,
        /// The `T` bin (every other bin together).
        pub total: bool,
    }

    impl TallySummary {
        pub fn axis(&self, dim: char) -> Option<&AxisSummary> {
            self.axes.iter().find(|a| a.dim == dim)
        }

        /// The choice of this tally with each axis at its Total, or at its
        /// first bin when it has none.
        pub fn choice(&self) -> TallyChoice {
            let mut c = TallyChoice {
                tally: Some(self.id),
                ..TallyChoice::default()
            };
            for a in &self.axes {
                if !a.total {
                    c.set_bin(&a.dim.to_string(), Bin::Index(0))
                        .expect("a dimension");
                }
            }
            c
        }
    }

    /// One axis of a tally, with a label for each of its bins.
    #[derive(Clone, Debug)]
    pub struct AxisSummary {
        /// `e`, `t`, or one of [`OTHER_DIMS`].
        pub dim: char,
        /// What the axis is: `energy`, `time`, `multiplier`...
        pub name: &'static str,
        /// One per bin (`0.1–1 MeV`, `m 2`); the Total's not included.
        pub labels: Vec<String>,
        /// Whether MCNP printed a Total bin after them.
        pub total: bool,
        /// Whether each bin holds everything up to its bound (`C` option).
        pub cumulative: bool,
    }

    impl AxisSummary {
        /// What `bin` is along this axis, for a menu.
        pub fn label(&self, bin: Bin) -> String {
            match bin {
                Bin::Total => "Total".into(),
                Bin::Sum if self.total => "Total".into(),
                Bin::Sum => "sum of all".into(),
                Bin::Index(k) => self
                    .labels
                    .get(k)
                    .cloned()
                    .unwrap_or_else(|| format!("{} {k}", self.dim)),
            }
        }
    }

    fn axis_summary(dim: char, axis: &Axis) -> Option<AxisSummary> {
        if axis.len() <= 1 {
            return None;
        }
        let (name, unit) = match dim {
            'e' => ("energy", " MeV"),
            't' => ("time", " sh"),
            'd' => ("direct or flagged", ""),
            'u' => ("user", ""),
            's' => ("segment", ""),
            'm' => ("multiplier", ""),
            'c' => ("cosine", ""),
            _ => ("", ""),
        };
        let n = axis.n_bins();
        let b = super::tick_label;
        let labels = match (axis.upper(), axis.bounds()) {
            (Some(up), bounds) => (0..n)
                .map(|k| {
                    let lo = bounds.map(|b| b[k]).or((k > 0).then(|| up[k - 1]));
                    match lo {
                        _ if axis.cumulative => format!("≤ {}{unit}", b(up[k])),
                        Some(lo) => format!("{}–{}{unit}", b(lo), b(up[k])),
                        None => format!("< {}{unit}", b(up[k])),
                    }
                })
                .collect(),
            (None, _) => (0..n).map(|k| format!("{dim} {k}")).collect(),
        };
        Some(AxisSummary {
            dim,
            name,
            labels,
            total: axis.total,
            cumulative: axis.cumulative,
        })
    }

    fn meshtal_summary(t: &meshtal::TallyInfo) -> TallySummary {
        let (i, j, k) = t.geometry.dims();
        let star = if t.energy_weighted == Some(true) {
            "*"
        } else {
            ""
        };
        let mut notes = Vec::new();
        if !t.geometry_from_input {
            notes.push(
                "mesh from the file's header: rounded, and without TR= (the run's input \
                 has it exactly)"
                    .to_string(),
            );
        }
        TallySummary {
            id: t.id,
            kind: format!("{star}FMESH{}:{}", t.id, t.particle.trim()),
            comment: t.comment.clone(),
            cells: i * j * k,
            not_drawn: None,
            wants_input: !t.geometry_from_input,
            notes,
            axes: [('e', &t.energy), ('t', &t.time)]
                .into_iter()
                .filter_map(|(d, a)| axis_summary(d, a))
                .collect(),
            bins: Vec::new(),
            warnings: t.warnings.iter().map(|w| w.to_string()).collect(),
        }
    }

    fn cuv_summary(t: &cuv::CuvInfo) -> TallySummary {
        let (i, j, k) = t.geometry.dims();
        let mut notes = vec![
            "cell under voxel: each voxel's flux is its cells' fluxes weighted by their \
             share of its volume (errors as if the cells were independent: an estimate, \
             usually low)"
                .to_string(),
        ];
        if !t.geometry_from_input {
            notes.push(
                "mesh from the file's header: rounded, and without TR= (the run's input \
                 has it exactly)"
                    .to_string(),
            );
        }
        TallySummary {
            id: t.id,
            kind: format!(
                "FMESH{}:{}",
                t.id,
                t.particle.as_deref().unwrap_or("?").trim()
            ),
            comment: t.comment.clone(),
            cells: i * j * k,
            not_drawn: None,
            wants_input: !t.geometry_from_input,
            notes,
            axes: axis_summary('e', &cuv_energy(t)).into_iter().collect(),
            bins: Vec::new(),
            warnings: t.warnings.iter().map(|w| w.to_string()).collect(),
        }
    }

    /// The energy axis of a CuV mesh's voxel fluxes, as `Cuvmsh::to_mesh`
    /// makes it: the spectra's bins, and a Total when there are several.
    fn cuv_energy(t: &cuv::CuvInfo) -> Axis {
        let bounds = t
            .energy
            .as_ref()
            .and_then(|e| e.bounds().map(<[f64]>::to_vec))
            .unwrap_or_else(|| vec![0.0, 1e36]);
        Axis::mcnp(bounds)
    }

    fn mctal_summary(t: &mctal::TallyInfo) -> TallySummary {
        let modifier = match t.modifier {
            Modifier::None => "",
            Modifier::Star => "*",
            Modifier::Plus => "+",
        };
        let particles: Vec<String> = t.particles.iter().map(|c| c.to_string()).collect();
        let kind = if t.mesh.is_some() {
            format!("TMESH {}", t.id)
        } else {
            format!("{modifier}F{}:{}", t.kind(), particles.join(","))
        };
        let unnamed = unnamed_bins(t);
        let mut notes = Vec::new();
        if unnamed > 0 && not_drawn(t).is_none() {
            notes.push(format!(
                "{unnamed} of its {} bins are groups, levels or the T bin: only the run's \
                 input (its F{} card) says which cells",
                t.numbers.len(),
                t.id
            ));
        }
        let dims = [
            ('d', mctal::D),
            ('u', mctal::U),
            ('s', mctal::S),
            ('m', mctal::M),
            ('c', mctal::C),
            ('e', mctal::E),
            ('t', mctal::T),
        ];
        TallySummary {
            id: t.id,
            kind,
            comment: t.comment.as_deref().map(str::trim).map(String::from),
            cells: t.axes[mctal::F].len(),
            not_drawn: not_drawn(t),
            wants_input: unnamed > 0,
            notes,
            axes: dims
                .into_iter()
                .filter_map(|(c, d)| axis_summary(c, &t.axes[d]))
                .collect(),
            bins: if not_drawn(t).is_none() {
                bin_summaries(t)
            } else {
                Vec::new()
            },
            warnings: t.warnings.iter().map(|w| w.to_string()).collect(),
        }
    }

    /// What each cell bin of `t` is: from the input's F card when given,
    /// else from the file's numbers (a bin's first cell; a cell in several
    /// bins is in chains the file can't show).
    fn bin_summaries(t: &mctal::TallyInfo) -> Vec<BinSummary> {
        if !t.bins.is_empty() {
            return t
                .bins
                .iter()
                .map(|b| match b {
                    TallyBin::Total => BinSummary {
                        label: "T".into(),
                        cells: Vec::new(),
                        whole_cells: false,
                        total: true,
                    },
                    TallyBin::Levels(levels) => {
                        let first = levels.first().map(Vec::as_slice).unwrap_or_default();
                        let mut cells: Vec<i64> = first.iter().map(|r| r.number).collect();
                        cells.dedup();
                        BinSummary {
                            label: b.to_string(),
                            cells,
                            whole_cells: levels.len() == 1
                                && first.iter().all(|r| r.element.is_none()),
                            total: false,
                        }
                    }
                })
                .collect();
        }
        let mut count = rustc_hash::FxHashMap::default();
        for n in t.numbers.iter().flatten() {
            *count.entry(n.number).or_insert(0) += 1;
        }
        t.numbers
            .iter()
            .map(|n| match n {
                Some(n) if n.number > 0 => BinSummary {
                    label: n.number.to_string(),
                    cells: vec![n.number],
                    whole_cells: count[&n.number] == 1,
                    total: false,
                },
                _ => BinSummary {
                    label: "?".into(),
                    cells: Vec::new(),
                    whole_cells: false,
                    total: false,
                },
            })
            .collect()
    }

    /// How many of a tally's bins only the input names: without its F
    /// card, those whose first level the file gives no number for.
    fn unnamed_bins(t: &mctal::TallyInfo) -> usize {
        if !t.bins.is_empty() {
            return 0;
        }
        t.numbers
            .iter()
            .filter(|n| n.is_none_or(|n| n.number <= 0))
            .count()
    }

    /// Whether a tally can be drawn as it is: drawn, with a Total (or one
    /// bin) along every axis but its cells, and some cells it names.
    fn ready(t: &mctal::TallyInfo) -> bool {
        not_drawn(t).is_none()
            && t.axes[1..].iter().all(|a| a.total || a.n_bins() <= 1)
            && unnamed_bins(t) < t.numbers.len().max(1)
    }

    /// The field of `choice` from a tally file; only that tally is read.
    ///
    /// - A meshtal tally gives a field on its mesh, at one energy group and
    ///   time bin.
    /// - An MCTAL cell tally (F4, F6, F7, F8, with `*` or `+`) gives a field
    ///   per cell when its bins are plain cells (a group `(1 2)` gives each
    ///   of its cells the group's value), and per cell instance when some
    ///   are repeated-structure chains (`(8 < 7[1 0 0] < 3)`) or cells that
    ///   are filled (such a bin scores, and colours, everything inside
    ///   them). Where bins overlap, the one holding a point most closely
    ///   gives its value, as [`llevant::Instances::innermost_bins`] says:
    ///   the deepest cell, then more levels, then a single cell at the first
    ///   level rather than a union, then the first. The T bin is left out.
    ///   Bins along
    ///   every other dimension are chosen as for energy and time
    ///   ([`TallyChoice::other`]). Surface tallies, detectors and TMESH
    ///   mesh tallies are not drawn (an error).
    ///
    /// A zero result (nothing scored there) is a value, 0, drawn in the
    /// colour of its scale's low end; its relative error (0 as MCNP writes
    /// it, which says nothing of precision) is no value.
    pub fn tally_field(m: &Model, file: &TallyFile, choice: &TallyChoice) -> Result<Field> {
        tally_field_and_bins(m, file, choice).map(|f| f.0)
    }

    /// [`tally_field`], and for an MCTAL tally the result and relative
    /// error of each of its cell bins (in the order of
    /// [`TallySummary::bins`]) at the bins chosen along the others.
    pub fn tally_field_and_bins(
        m: &Model,
        file: &TallyFile,
        choice: &TallyChoice,
    ) -> Result<(Field, Option<BinValues>)> {
        let err = |e: ReadError| match e {
            ReadError::Io(e) => Error::Io {
                path: file.path.clone(),
                source: e,
            },
            e => Error::Invalid(format!("{}: {e}", file.path.display())),
        };
        match &file.reader {
            Reader::Meshtal(r) => Ok((meshtal_field(r, choice, err)?, None)),
            Reader::Mctal(r) => mctal_field(m, r, choice, err).map(|(f, b)| (f, Some(b))),
            Reader::Cuv(r, last) => Ok((cuv_field(r, last, choice, err)?, None)),
        }
    }

    /// The indices along `axis` whose values `bin` adds up: one, or for a
    /// sum on an axis without a Total (nor cumulative), all its bins.
    fn pick(id: u32, axis: &Axis, bin: Bin, what: &str) -> Result<Vec<usize>> {
        let n = axis.n_bins();
        match bin {
            Bin::Index(k) if k < n => Ok(vec![k]),
            Bin::Index(k) => Err(Error::Invalid(format!("tally {id}: {what} {k} of {n}"))),
            Bin::Total => axis
                .total_index()
                .or((n == 1).then_some(0))
                .map(|k| vec![k])
                .ok_or_else(|| {
                    Error::Invalid(format!(
                        "tally {id}: no total {what}: give one of its {n} (0 to {}) or `sum`",
                        n - 1
                    ))
                }),
            Bin::Sum => Ok(match axis.total_index() {
                Some(t) => vec![t],
                None if axis.cumulative && n > 0 => vec![n - 1],
                None => (0..n).collect(),
            }),
        }
    }

    /// Values and relative errors added up, errors in quadrature (as
    /// aci_tallies' `values` does): the result, or its relative error
    /// ([`as_value`]).
    fn combine(values: impl Iterator<Item = (f64, f64)>, error: bool) -> f32 {
        as_value(sum_up(values), error)
    }

    /// The result or the relative error of `(r, e)`. Nothing scored (a
    /// zero result) is a result of 0, but no relative error.
    fn as_value((r, e): (f64, f64), error: bool) -> f32 {
        match (r == 0.0, error) {
            (true, true) => f32::NAN,
            (_, true) => e as f32,
            (_, false) => r as f32,
        }
    }

    /// Values and relative errors added up, errors in quadrature (as
    /// aci_tallies' `values` does); one is itself.
    fn sum_up(values: impl Iterator<Item = (f64, f64)>) -> (f64, f64) {
        let (mut sum, mut var, mut one) = (0.0, 0.0, None);
        for (k, (r, e)) in values.enumerate() {
            one = (k == 0).then_some((r, e));
            sum += r;
            var += (r * e) * (r * e);
        }
        match one {
            Some(single) => single,
            None => (
                sum,
                if var == 0.0 {
                    0.0
                } else {
                    var.sqrt() / sum.abs()
                },
            ),
        }
    }

    fn meshtal_field(
        r: &meshtal::Reader,
        choice: &TallyChoice,
        err: impl Fn(ReadError) -> Error,
    ) -> Result<Field> {
        let t = match choice.tally {
            Some(id) => r.tally(id).map_err(&err)?,
            None => r.tallies().next().ok_or_else(|| {
                err(ReadError::NoTally {
                    id: None,
                    tallies: Vec::new(),
                })
            })?,
        };
        mesh_field(t, choice, |e, k| {
            let b = r.read_bin(t.id, e, k).map_err(&err)?;
            Ok((
                b.result.iter().copied().collect(),
                b.rel_error.iter().copied().collect(),
            ))
        })
    }

    /// A CuV mesh's voxel fluxes, as a meshtal tally's ([`TallyFile`]).
    fn cuv_field(
        r: &cuv::Cuvmsh,
        last: &Mutex<Option<Arc<meshtal::Tally>>>,
        choice: &TallyChoice,
        err: impl Fn(ReadError) -> Error,
    ) -> Result<Field> {
        let id = match choice.tally {
            Some(id) => id,
            None => r.tallies().next().map(|t| t.id).ok_or_else(|| {
                err(ReadError::NoTally {
                    id: None,
                    tallies: Vec::new(),
                })
            })?,
        };
        let t = {
            let mut last = last.lock().unwrap_or_else(PoisonError::into_inner);
            match &*last {
                Some(t) if t.id == id => t.clone(),
                _ => {
                    let t = Arc::new(r.to_mesh(id).map_err(&err)?);
                    *last = Some(t.clone());
                    t
                }
            }
        };
        mesh_field(&t.info(), choice, |e, k| {
            let [result, error] = [&t.result, &t.rel_error].map(|a| {
                let energy = a.outer_iter().nth(e).expect("an energy bin of the tally");
                let bin = energy.outer_iter().nth(k).expect("a time bin of the tally");
                bin.iter().copied().collect::<Vec<f64>>()
            });
            Ok((result, error))
        })
    }

    /// A field of mesh tally `t`, whose energy bin `e` and time bin `k`
    /// `read` gives (results and relative errors, in the mesh's flat
    /// order).
    fn mesh_field(
        t: &meshtal::TallyInfo,
        choice: &TallyChoice,
        mut read: impl FnMut(usize, usize) -> Result<(Vec<f64>, Vec<f64>)>,
    ) -> Result<Field> {
        if let Some(k) = choice.other.iter().position(|&b| b != Bin::Total) {
            return Err(Error::Invalid(format!(
                "tally {}: a mesh tally has no {} bins",
                t.id, OTHER_DIMS[k]
            )));
        }
        let es = pick(t.id, &t.energy, choice.energy, "energy group")?;
        let ks = pick(t.id, &t.time, choice.time, "time bin")?;
        // `[i, j, k]`, k fastest as the mesh's flat order; a sum is read
        // bin by bin.
        let mut flat = Vec::with_capacity(es.len() * ks.len());
        for &e in &es {
            for &k in &ks {
                flat.push(read(e, k)?);
            }
        }
        let n = flat[0].0.len();
        let values = (0..n)
            .map(|v| combine(flat.iter().map(|(r, e)| (r[v], e[v])), choice.error))
            .collect();
        let what = if choice.error { "rel. error" } else { "result" };
        let unit = match (choice.error, t.energy_weighted) {
            (true, _) => "",
            (false, Some(true)) => "MeV/cm²",
            (false, _) => "1/cm²",
        };
        Field::mesh(
            format!("tally {} {} ({})", t.id, what, t.particle.trim()),
            unit,
            t.geometry.clone(),
            values,
            None,
        )
    }

    /// Why an MCTAL tally can't be drawn, if it can't.
    fn not_drawn(t: &mctal::TallyInfo) -> Option<String> {
        let why = if t.mesh.is_some() {
            "TMESH mesh tallies are not drawn yet".into()
        } else if t.detector != Detector::None || t.kind() == 5 {
            "detectors are not drawn yet".into()
        } else if matches!(t.kind(), 1 | 2) {
            "surface tallies are not drawn yet".into()
        } else if !matches!(t.kind(), 4 | 6 | 7 | 8) {
            format!("tallies of type {} are not drawn", t.kind())
        } else {
            return None;
        };
        Some(format!("tally {}: {why}", t.id))
    }

    fn mctal_field(
        m: &Model,
        r: &mctal::Reader,
        choice: &TallyChoice,
        err: impl Fn(ReadError) -> Error,
    ) -> Result<(Field, BinValues)> {
        let info = match choice.tally {
            Some(id) => r.tally(id).map_err(&err)?,
            None => r
                .tallies()
                .find(|t| ready(t))
                .or_else(|| r.tallies().find(|t| not_drawn(t).is_none()))
                .ok_or_else(|| {
                    Error::Invalid(format!(
                        "no cell tally to draw (tallies: {:?})",
                        r.tallies().map(|t| t.id).collect::<Vec<_>>()
                    ))
                })?,
        };
        if let Some(why) = not_drawn(info) {
            return Err(Error::Invalid(why));
        }
        let id = info.id;

        // What each f bin covers: its levels, from the cell scored outwards.
        let bins: Vec<Option<Vec<Vec<Step>>>> = if !info.bins.is_empty() {
            let step = |b: &mctal::BinRef| -> Result<Step> {
                Ok(Step {
                    cell: m.cell_index(b.number).ok_or(Error::NoCell(b.number))?,
                    lattice: b.element.map(|e| e.map(|x| x as i32)),
                })
            };
            info.bins
                .iter()
                .map(|b| match b {
                    TallyBin::Total => Ok(None),
                    TallyBin::Levels(levels) => levels
                        .iter()
                        .map(|l| l.iter().map(step).collect())
                        .collect::<Result<_>>()
                        .map(Some),
                })
                .collect::<Result<_>>()?
        } else {
            // Without the input, the file's numbers: a bin's first level
            // only, so a cell twice means chains it can't show.
            let mut seen = rustc_hash::FxHashSet::default();
            info.numbers
                .iter()
                .map(|n| match n {
                    Some(n) if n.number > 0 => {
                        if !seen.insert(n.number) {
                            let why = info
                                .warnings
                                .first()
                                .map_or(String::new(), |w| format!(" ({})", w.message));
                            return Err(Error::Invalid(format!(
                                "tally {id}: cell {} is in several bins: the MCTAL file names \
                                 only the first level of a bin, read it with the model's MCNP \
                                 input{why}",
                                n.number
                            )));
                        }
                        let cell = m.cell_index(n.number).ok_or(Error::NoCell(n.number))?;
                        Ok(Some(vec![vec![Step {
                            cell,
                            lattice: None,
                        }]]))
                    }
                    _ => Ok(None),
                })
                .collect::<Result<_>>()?
        };

        // The value of each f bin, at the bins chosen along the others (all
        // the combinations of them, added up, for sums).
        let mut at: [Vec<usize>; 8] = Default::default();
        at[mctal::F] = vec![0];
        for (k, &d) in [mctal::D, mctal::U, mctal::S, mctal::M, mctal::C]
            .iter()
            .enumerate()
        {
            let what = format!("{} bin", OTHER_DIMS[k]);
            at[d] = pick(id, &info.axes[d], choice.other[k], &what)?;
        }
        at[mctal::E] = pick(id, &info.axes[mctal::E], choice.energy, "energy bin")?;
        at[mctal::T] = pick(id, &info.axes[mctal::T], choice.time, "time bin")?;
        let t = r.read(id).map_err(&err)?;
        let standard = || Error::Invalid(format!("tally {id}: values not in standard order"));
        let result = t.result.as_slice().ok_or_else(standard)?;
        let rel_error = t.rel_error.as_slice().ok_or_else(standard)?;
        let shape = info.shape();
        let (mut offsets, mut stride) = (vec![0usize], 1);
        for d in (1..8).rev() {
            offsets = at[d]
                .iter()
                .flat_map(|&i| offsets.iter().map(move |&o| o + i * stride))
                .collect();
            stride *= shape[d];
        }
        let per_bin: Vec<(f64, f64)> = (0..shape[mctal::F])
            .map(|f| {
                sum_up(offsets.iter().map(|&o| {
                    let k = f * stride + o;
                    (result[k], rel_error[k])
                }))
            })
            .collect();
        let value = |f: usize| as_value(per_bin[f], choice.error);

        // A field per cell when every bin is plain cells, none filled (a
        // filled one scores what is inside it); per instance otherwise.
        // Where bins overlap, as `Instances::innermost_bins` picks.
        let plain = bins.iter().flatten().all(|l| {
            l.len() == 1
                && l[0]
                    .iter()
                    .all(|s| s.lattice.is_none() && m.cell(s.cell).fill.is_none())
        });
        let values = if plain {
            let mut best: Vec<Option<(bool, usize)>> = vec![None; m.cells().len()];
            for (f, levels) in bins.iter().enumerate() {
                let Some(levels) = levels else { continue };
                let single = levels[0].len() == 1;
                for s in &levels[0] {
                    let b = &mut best[s.cell as usize];
                    if b.is_none_or(|(one, _)| single && !one) {
                        *b = Some((single, f));
                    }
                }
            }
            Values::Cells(
                best.iter()
                    .map(|b| b.map_or(f32::NAN, |b| value(b.1)))
                    .collect(),
            )
        } else {
            let total = m.instances().total();
            if total > 1 << 31 {
                return Err(Error::Invalid(format!(
                    "{total} instances are too many for a field per instance"
                )));
            }
            // Bins left out (T) stay in the numbering as empty chains.
            let chains: Vec<Vec<Vec<Step>>> =
                bins.iter().map(|b| b.clone().unwrap_or_default()).collect();
            let holder = m.instances().innermost_bins(m, &chains);
            Values::Instances(
                holder
                    .iter()
                    .map(|&k| {
                        if k == NO_BIN {
                            f32::NAN
                        } else {
                            value(k as usize)
                        }
                    })
                    .collect(),
            )
        };

        let what = if choice.error { "rel. error" } else { "result" };
        let particles: String = info.particles.iter().collect();
        // Per source particle; an FM card changes the unit (only one of
        // several bins is noticed: MCTAL writes `m 0` and `m 1` alike).
        let unit = match (info.kind(), info.modifier) {
            _ if choice.error || info.axes[mctal::M].len() > 1 => "",
            (4, Modifier::Star) => "MeV/cm²",
            (4, _) => "1/cm²",
            (6, Modifier::Star) => "jerks/g",
            (6 | 7, _) => "MeV/g",
            (8, Modifier::Star) => "MeV",
            _ => "",
        };
        Ok((
            Field::new(format!("tally {id} {what} ({particles})"), unit, values),
            per_bin,
        ))
    }

    fn io(path: &Path) -> impl Fn(std::io::Error) -> Error + '_ {
        move |e| Error::Io {
            path: path.to_path_buf(),
            source: e,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llevant::Options;

    const MODEL: &str = "t
1 1 -2.0 -1 imp:n=1
2 0 1 -9 imp:n=1
9 0 9 imp:n=0

1 rpp -5 5 -5 5 -5 5
9 so 100

m1 26056 1
";

    #[test]
    fn values_by_cell_and_by_mesh() {
        let m = Model::from_text(MODEL, &Options::default());
        let f = Field::cells(&m, "x", [(1, 3.0)]).unwrap();
        let inside = m.classify_point([0.0; 3]);
        assert_eq!(f.value(&m, &inside, [0.0; 3]), Some(3.0));
        let outside = m.classify_point([50.0, 0.0, 0.0]);
        assert_eq!(f.value(&m, &outside, [50.0, 0.0, 0.0]), None);
        assert!(Field::cells(&m, "x", [(7, 1.0)]).is_err());
        // Two voxels along x across cell 1.
        let g = Geometry::Rectangular {
            x_bounds: vec![-5.0, 0.0, 5.0],
            y_bounds: vec![-5.0, 5.0],
            z_bounds: vec![-5.0, 5.0],
            transform: None,
        };
        let f = Field::mesh("flux", "1/cm²", g.clone(), vec![1.0, 100.0], None).unwrap();
        assert!(f.per_point());
        assert_eq!(f.value(&m, &inside, [-1.0, 0.0, 0.0]), Some(1.0));
        assert_eq!(f.value(&m, &inside, [1.0, 0.0, 0.0]), Some(100.0));
        assert_eq!(f.title(), "flux [1/cm²]");
        // Cell under voxel: only cell 1's part of voxel 1 has a value.
        let c1 = m.cell_index(1).unwrap();
        let cuv = CellUnderVoxel::new(2, vec![(1, c1, 7.0)]).unwrap();
        let f = Field::mesh("d", "", g, vec![1.0, 100.0], Some(cuv)).unwrap();
        assert_eq!(f.value(&m, &inside, [1.0, 0.0, 0.0]), Some(7.0));
        assert_eq!(f.value(&m, &inside, [-1.0, 0.0, 0.0]), None);
    }

    #[test]
    fn csv_by_cell_material_and_path() {
        let m = Model::from_text(MODEL, &Options::default());
        let dir = std::env::temp_dir().join(format!("llevant-field-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.csv");
        std::fs::write(&p, "# a comment\ncell,dose [Sv/h]\n1,2.5\n2,\n").unwrap();
        let f = read_csv(&m, &p).unwrap();
        assert_eq!((f.name.as_str(), f.unit.as_str()), ("dose", "Sv/h"));
        let c1 = m.cell_index(1).unwrap() as usize;
        let Values::Cells(v) = &f.values else {
            panic!()
        };
        assert_eq!(v[c1], 2.5);
        assert_eq!(v.iter().filter(|x| x.is_finite()).count(), 1);
        std::fs::write(&p, "material,x\n1,4\n").unwrap();
        let Values::Cells(v) = read_csv(&m, &p).unwrap().values else {
            panic!()
        };
        assert_eq!(v[c1], 4.0);
        std::fs::write(&p, "path,x\n\"1\",5\n").unwrap();
        let f = read_csv(&m, &p).unwrap();
        let inside = m.classify_point([0.0; 3]);
        assert_eq!(f.value(&m, &inside, [0.0; 3]), Some(5.0));
        std::fs::write(&p, "voxel,x\n1,5\n").unwrap();
        assert!(read_csv(&m, &p).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
