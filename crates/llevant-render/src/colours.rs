//! Colours of slice and render images, as the `llevant` command draws them:
//! 8-bit RGB, rows from the top. Writing image files is left to the caller.

use llevant::vec3::{dot, Vec3};
use llevant::{Label, Model, Slice, Status};

use crate::field::Overlay;
use crate::scale::Scale;
use llevant_scene::materials::Looks;

/// An 8-bit RGB colour.
pub type Rgb = [u8; 3];

/// What a colour stands for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColourBy {
    /// The deepest cell (MCNP number).
    #[default]
    Cell,
    /// The deepest cell's material (void is near white).
    Material,
    /// The universe of the deepest cell.
    Universe,
    /// What the deepest cell's material looks like ([`Looks`]): steel
    /// grey, concrete, copper... Render images of it are lit physically
    /// ([`crate::shading`]).
    Realistic,
    /// The deepest cell's density in g/cm³ on a log scale ([`Scale::DENSITY`]).
    Density,
}

impl ColourBy {
    pub fn name(self) -> &'static str {
        match self {
            ColourBy::Cell => "cell",
            ColourBy::Material => "material",
            ColourBy::Universe => "universe",
            ColourBy::Realistic => "realistic",
            ColourBy::Density => "density",
        }
    }
}

/// Gaps: a checker of this and [`GAP_CHECK`] ([`marked`]).
pub const GAP: Rgb = [0, 0, 0];
/// The light squares of gaps.
pub const GAP_CHECK: Rgb = [120, 120, 120];
/// Overlaps, unsupported cells, too-deep nesting: stripes of this and
/// [`PROBLEM_STRIPE`] ([`marked`]).
pub const PROBLEM: Rgb = [255, 0, 255];
/// The dark stripes of problems.
pub const PROBLEM_STRIPE: Rgb = [45, 0, 45];
/// Width of the problems' stripes along a row (pixels; across them, this
/// over √2).
pub const STRIPE_PX: i64 = 6;
/// Side of the gaps' squares (pixels).
pub const CHECK_PX: i64 = 4;

/// What is wrong where no single cell is: a gap, or a problem (overlaps,
/// unsupported cells, too-deep nesting). Drawn as a pattern fixed to the
/// image's pixels ([`marked`]), which no cell colour can be mistaken for:
/// magenta stripes for problems, a black and grey checker for gaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mark {
    Gap,
    Problem,
}

impl Mark {
    /// The mark of a label of `status` with deepest cell `cell` (none:
    /// inside one cell, or on a surface).
    pub fn of(status: Status, cell: Option<u32>) -> Option<Mark> {
        match status {
            Status::Inside if cell.is_some() => None,
            Status::OnSurface => None,
            Status::Gap => Some(Mark::Gap),
            _ => Some(Mark::Problem),
        }
    }

    /// The flat colour that stands for the mark (legends, text).
    pub fn colour(self) -> Rgb {
        match self {
            Mark::Gap => GAP,
            Mark::Problem => PROBLEM,
        }
    }
}

/// Opacity of the pattern over what a ray sees beyond a gap (or overlap)
/// it went through ([`llevant::Visibility::veil`]).
pub const VEIL_ALPHA: f32 = 0.5;

/// `under`, the colour seen at pixel `(i, j)`, behind the veils `v` the
/// ray went through: the problems' stripes for an overlap, else the gaps'
/// checker, [`VEIL_ALPHA`] over it. The viewer's `colour.wgsl` draws the
/// same.
pub fn veiled(under: Rgb, v: llevant::Veils, i: usize, j: usize) -> Rgb {
    if !v.any() {
        return under;
    }
    let m = if v.overlap { Mark::Problem } else { Mark::Gap };
    let c = marked(m, i as i64, j as i64);
    [0, 1, 2]
        .map(|k| (under[k] as f32 + (c[k] as f32 - under[k] as f32) * VEIL_ALPHA).round() as u8)
}

/// Colour of pixel `(i, j)` (column, row from the top) marked `mark`:
/// diagonal stripes for problems, a checker for gaps, so that the pattern
/// stays put on screen and reads the same at any zoom. The viewer's
/// shaders (`colour.wgsl`, `ghost.wgsl`) draw the same.
pub fn marked(mark: Mark, i: i64, j: i64) -> Rgb {
    match mark {
        Mark::Problem if (i + j).div_euclid(STRIPE_PX) % 2 == 0 => PROBLEM,
        Mark::Problem => PROBLEM_STRIPE,
        Mark::Gap if (i.div_euclid(CHECK_PX) + j.div_euclid(CHECK_PX)) % 2 == 0 => GAP,
        Mark::Gap => GAP_CHECK,
    }
}
pub const ON_SURFACE: Rgb = [90, 90, 90];
/// Lines along cell boundaries in slices.
pub const OUTLINE: Rgb = [40, 40, 40];
/// Rays that hit nothing.
pub const BACKGROUND: Rgb = [255, 255, 255];
/// Void cells coloured by material.
pub const VOID: Rgb = [245, 245, 245];

/// A colour that depends only on `(kind, n)`, so it stays the same between
/// runs and models.
fn hashed(kind: u64, n: i64) -> Rgb {
    let mut z = (n as u64)
        .wrapping_add(kind << 56)
        .wrapping_add(0x9e3779b97f4a7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^= z >> 31;
    [
        z as u8 | 0x40,
        (z >> 8) as u8 | 0x40,
        (z >> 16) as u8 | 0x40,
    ]
}

/// The colour of cell number `id` when coloured by cell (for legends).
pub fn cell_number_colour(id: i64) -> Rgb {
    hashed(1, id)
}

/// The colour of material `id` when coloured by material ([`VOID`] for 0).
pub fn material_colour(id: i64) -> Rgb {
    if id == 0 {
        VOID
    } else {
        hashed(2, id)
    }
}

/// The colour of universe `u` when coloured by universe.
pub fn universe_colour(u: i64) -> Rgb {
    hashed(3, u)
}

/// How cells are coloured: the model, what its materials look like, what
/// a colour stands for, and a field drawn over them. Every image of a model
/// (slices, renders, the viewer's palette) takes its colours from one
/// `Paint`, which the caller builds and owns, so that the model stays
/// read-only.
#[derive(Clone, Copy)]
pub struct Paint<'a> {
    pub model: &'a Model,
    /// The looks of `model`'s materials (realistic colours, and the atomic
    /// masses that put atom densities in g/cm³).
    pub looks: &'a Looks,
    pub by: ColourBy,
    /// Numbers over the geometry, coloured over the cells that have one.
    pub field: Option<&'a Overlay>,
    /// The universe level shown in slices (MCNP's plotter `LEVEL n`): cells
    /// at other levels are left blank ([`BACKGROUND`]), as their labels
    /// from [`Depth::Level`](llevant::Depth::Level) are shorter; `None`, every label.
    pub level: Option<usize>,
}

impl<'a> Paint<'a> {
    pub fn new(model: &'a Model, looks: &'a Looks, by: ColourBy) -> Paint<'a> {
        Paint {
            model,
            looks,
            by,
            field: None,
            level: None,
        }
    }

    /// With `field` drawn over the cells.
    pub fn with_field(self, field: Option<&'a Overlay>) -> Paint<'a> {
        Paint { field, ..self }
    }

    /// The same, showing universe level `level` only ([`Paint::level`]).
    pub fn with_level(self, level: Option<usize>) -> Paint<'a> {
        Paint { level, ..self }
    }

    /// Whether label `l` is left blank: inside a cell at another level than
    /// the one shown.
    pub fn blank(&self, l: &Label) -> bool {
        l.status == Status::Inside && self.level.is_some_and(|k| l.path.len() != k + 1)
    }

    /// Whether images are lit physically (realistic colours) rather than
    /// flat.
    pub fn pbr(&self) -> bool {
        self.by == ColourBy::Realistic
    }

    /// Colour of the cell at index `cell` ([`Model::cells`]).
    pub fn cell_colour(&self, cell: u32) -> Rgb {
        let (m, looks) = (self.model, self.looks);
        let c = m.cell(cell);
        match self.by {
            ColourBy::Cell => hashed(1, c.id),
            ColourBy::Material if c.material == 0 => VOID,
            ColourBy::Material => hashed(2, c.material),
            ColourBy::Universe => hashed(3, m.units()[c.unit as usize].universe),
            ColourBy::Realistic => looks.cell(cell).srgb(),
            ColourBy::Density if c.material == 0 => VOID,
            ColourBy::Density => {
                let mass = looks
                    .materials
                    .binary_search_by_key(&c.material, |l| l.id)
                    .map_or(0.0, |k| looks.materials[k].mix.atomic_mass);
                match llevant_scene::materials::grams_per_cm3(c.density, mass) {
                    Some(rho) => Scale::DENSITY.colour(rho),
                    None => UNKNOWN_DENSITY,
                }
            }
        }
    }

    /// Colour of `point`, whose label is `l`: the label's, or in a cell
    /// under a field the field's ([`FieldLook::over`]: its value's colour
    /// over the grey or the cell's colour, or that alone without a value).
    ///
    /// [`FieldLook::over`]: crate::field::FieldLook::over
    pub fn point_colour(&self, l: &Label, point: Vec3) -> Rgb {
        let base = self.label_colour(l);
        if self.blank(l) || l.status != Status::Inside || l.cell().is_none() {
            return base;
        }
        match self.field {
            Some(f) => f.look.over(base, f.field.value(self.model, l, point)),
            None => base,
        }
    }

    /// Whether colours with the field can change within one label (a
    /// field on a mesh), so that [`Paint::point_colour`] is needed per point.
    pub fn per_point(&self) -> bool {
        self.field.is_some_and(|f| f.field.per_point())
    }

    /// What is wrong in label `l` ([`Mark::of`]), none when blank.
    pub fn mark(&self, l: &Label) -> Option<Mark> {
        if self.blank(l) {
            return None;
        }
        Mark::of(l.status, l.cell())
    }

    /// Colour of a label: its cell's colour when inside one, else [`GAP`],
    /// [`ON_SURFACE`] or [`PROBLEM`] (without the field); [`BACKGROUND`]
    /// when [`Paint::blank`]. Images draw gaps and problems as patterns
    /// ([`marked`]) instead.
    pub fn label_colour(&self, l: &Label) -> Rgb {
        if self.blank(l) {
            return BACKGROUND;
        }
        match l.status {
            Status::Inside => match l.cell() {
                Some(c) => self.cell_colour(c),
                None => PROBLEM,
            },
            Status::Gap => GAP,
            Status::OnSurface => ON_SURFACE,
            _ => PROBLEM,
        }
    }
}

/// Cells whose density cannot be put in g/cm³.
pub const UNKNOWN_DENSITY: Rgb = [128, 128, 128];

/// `c` shaded by the angle between the ray direction `dir` and the
/// surface normal (no shading without a normal).
pub fn shade(c: Rgb, normal: Vec3, dir: Vec3) -> Rgb {
    let s = if normal == [0.0; 3] {
        1.0
    } else {
        0.25 + 0.75 * dot(normal, dir).abs()
    };
    c.map(|x| (x as f64 * s) as u8)
}

/// Which boundaries a slice's outlines are drawn along.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Outlines {
    /// Between labels: every cell instance (a filled universe's cells too).
    #[default]
    Cell,
    /// Between the materials of the deepest cells (void is one): the
    /// boundaries that explain a field.
    Material,
    /// Between the universes of the deepest cells: where fills begin.
    Universe,
}

impl Outlines {
    pub const ALL: [Outlines; 3] = [Outlines::Cell, Outlines::Material, Outlines::Universe];

    pub fn name(self) -> &'static str {
        match self {
            Outlines::Cell => "cell",
            Outlines::Material => "material",
            Outlines::Universe => "universe",
        }
    }

    /// The outlines called `name` (case aside).
    pub fn from_name(name: &str) -> Option<Outlines> {
        Outlines::ALL
            .into_iter()
            .find(|o| o.name().eq_ignore_ascii_case(name))
    }

    /// What tells the boundaries apart, per label of `labels`: the label
    /// itself, or its deepest cell's material or universe. Labels outside
    /// any cell, and every label when a level is shown (whose cells are not
    /// the deepest), are told apart as labels. The two kinds never match.
    pub fn classes(self, paint: &Paint, labels: &[Label]) -> Vec<u64> {
        let m = paint.model;
        labels
            .iter()
            .enumerate()
            .map(|(k, l)| {
                let own = k as u64 | 1 << 63;
                let cell = l.cell().filter(|_| paint.level.is_none());
                match (self, cell) {
                    (Outlines::Material, Some(c)) => m.cell(c).material as u64,
                    (Outlines::Universe, Some(c)) => {
                        m.units()[m.cell(c).unit as usize].universe as u64 & !(1 << 63)
                    }
                    _ => own,
                }
            })
            .collect()
    }
}

/// Image of `slice` from [`Slice::pixels`] ids into `labels`: each pixel
/// coloured by `paint` (one colour per label, but on a mesh field; gaps
/// and problems as their patterns, [`marked`]), and
/// with `outline` a line where what it tells apart ([`Outlines::classes`])
/// changes from one pixel to the next (right or below), but between blank
/// labels ([`Paint::blank`]). `3 · nx · ny` bytes.
pub fn slice_rgb(
    paint: &Paint,
    slice: &Slice,
    labels: &[Label],
    ids: &[u32],
    outline: Option<Outlines>,
) -> Vec<u8> {
    let (nx, ny) = (slice.nx, slice.ny);
    let per_point = paint.per_point();
    let cols: Vec<Rgb> = labels
        .iter()
        .map(|l| match per_point {
            true => paint.label_colour(l),
            false => paint.point_colour(l, slice.origin),
        })
        .collect();
    let blank: Vec<bool> = labels.iter().map(|l| paint.blank(l)).collect();
    let marks: Vec<Option<Mark>> = labels.iter().map(|l| paint.mark(l)).collect();
    let class = outline.map_or_else(Vec::new, |o| o.classes(paint, labels));
    let differ = |a: u32, b: u32| {
        a != b
            && class[a as usize] != class[b as usize]
            && !(blank[a as usize] && blank[b as usize])
    };
    let outline = outline.is_some();
    let mut img = Vec::with_capacity(3 * nx * ny);
    for j in 0..ny {
        for i in 0..nx {
            let id = ids[j * nx + i];
            let edge = outline
                && ((i + 1 < nx && differ(ids[j * nx + i + 1], id))
                    || (j + 1 < ny && differ(ids[(j + 1) * nx + i], id)));
            let c = if edge {
                OUTLINE
            } else if let Some(m) = marks[id as usize] {
                marked(m, i as i64, j as i64)
            } else if per_point {
                paint.point_colour(&labels[id as usize], slice.pixel_centre(i, j))
            } else {
                cols[id as usize]
            };
            img.extend_from_slice(&c);
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use llevant::Options as ModelOptions;

    /// Cells 1 and 2 of material 1 side by side, then cell 3 filled with
    /// universe 1: cells 11 and 12, both of material 2, split at y = 0.
    const MODEL: &str = "outlines
1 1 -1 -1 imp:n=1
2 1 -1 -2 imp:n=1
3 0 -3 fill=1 imp:n=1
11 2 -1 -4 u=1 imp:n=1
12 2 -1 4 u=1 imp:n=1
8 0 1 2 3 -9 imp:n=1
9 0 9 imp:n=0

1 rpp -10 0 -10 10 -10 10
2 rpp 0 10 -10 10 -10 10
3 rpp 10 20 -10 10 -10 10
4 py 0
9 so 100

m1 1001.80c 1
m2 8016.80c 1
";

    #[test]
    fn outlines_between_cells_materials_and_universes() {
        let m = Model::from_text(MODEL, &ModelOptions::default());
        assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
        let looks = Looks::new(&m, &[]).unwrap();
        let paint = Paint::new(&m, &looks, ColourBy::Cell);
        // 10 pixels a cm, over x from -10 to 20 and y from -10 to 10.
        let s = Slice {
            origin: [-10.0, -10.0, 0.5],
            u: [1.0, 0.0, 0.0],
            v: [0.0, 1.0, 0.0],
            width: 30.0,
            height: 20.0,
            nx: 300,
            ny: 200,
        };
        let tr = m.slice(&s, llevant::Depth::All).unwrap();
        let ids = s.pixels(&tr);
        let lines = |o: Option<Outlines>| {
            let img = slice_rgb(&paint, &s, &tr.labels, &ids, o);
            let at = |i: usize, j: usize| img[3 * (j * s.nx + i)..][..3] == OUTLINE;
            // Between cells 1 and 2 (x = 0), cell 2 and the fill (x = 10),
            // cells 11 and 12 (y = 0, at x = 15).
            let y0 = (95..105).any(|j| at(250, j));
            (at(99, 50), at(199, 50), y0)
        };
        assert_eq!(lines(None), (false, false, false));
        assert_eq!(lines(Some(Outlines::Cell)), (true, true, true));
        assert_eq!(lines(Some(Outlines::Material)), (false, true, false));
        assert_eq!(lines(Some(Outlines::Universe)), (false, true, false));
        // Material 1 against 2 at x = 10, as above; at x = 15 the two cells
        // of universe 1 are one universe and one material.
        assert_eq!(Outlines::from_name("Material"), Some(Outlines::Material));
        assert_eq!(Outlines::from_name("surface"), None);
    }

    #[test]
    fn marks_are_patterns() {
        // Stripes and squares alternate, both colours in any 12 × 12 patch.
        for m in [Mark::Gap, Mark::Problem] {
            let mut seen = std::collections::HashSet::new();
            for j in 0..12 {
                for i in 0..12 {
                    seen.insert(marked(m, i, j));
                }
            }
            assert_eq!(seen.len(), 2, "{m:?}");
        }
        assert_eq!(marked(Mark::Problem, 0, 0), PROBLEM);
        assert_eq!(marked(Mark::Problem, STRIPE_PX, 0), PROBLEM_STRIPE);
        assert_eq!(marked(Mark::Gap, 0, 0), GAP);
        assert_eq!(marked(Mark::Gap, CHECK_PX, 0), GAP_CHECK);
        assert_eq!(Mark::of(Status::Inside, Some(3)), None);
        assert_eq!(Mark::of(Status::Inside, None), Some(Mark::Problem));
        assert_eq!(Mark::of(Status::Overlap, None), Some(Mark::Problem));
        assert_eq!(Mark::of(Status::Gap, None), Some(Mark::Gap));
        assert_eq!(Mark::of(Status::OnSurface, None), None);
    }
}
