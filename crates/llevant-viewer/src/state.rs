//! What the viewer shows, as plain data: the file `--state` reads and
//! "save view" writes, and what other tools send to open a model at a given
//! view. Every field has a default, so a file may give only some of them.
//!
//! The scene's fields ([`Scene`]: colours, hidden cells, clip, effects,
//! materials, pins) sit at the top level of the file, next to the camera,
//! the slice and the tracing settings.

use llevant::Axis;
use llevant_render::colours::ColourBy;
use serde::{Deserialize, Serialize};

use crate::scene::Scene;

/// The range of [`ViewState::budget_ms`]. Below ~16 ms previews only get
/// smaller, not more frequent: a frame is ~16 ms, and the smallest preview
/// of a large model takes longer (~35 ms for the reference model on 16
/// threads).
pub const BUDGET_MS: std::ops::RangeInclusive<f64> = 16.0..=100.0;

/// The default [`ViewState::budget_ms`]. Orbiting the reference model on
/// the laptop's GPU (2026-11-14), previews at 33 / 66 / 100 ms are at
/// scale 0.50 / 0.84 / 0.89, 19 / 11 / 11 a second, at the same frame
/// rate: low-resolution previews flicker as they replace one another, and
/// past ~66 ms previews reach their size limits.
pub const DEFAULT_BUDGET_MS: f64 = 66.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewState {
    /// The 3D camera; `None` frames the whole model.
    pub camera: Option<CameraState>,
    #[serde(flatten)]
    pub scene: Scene,
    pub slice: SliceState,
    pub backend: BackendChoice,
    /// Time (ms) the 3D view aims to take per preview image while the
    /// camera moves: longer is sharper and fewer previews. The window's
    /// frame rate does not depend on it: tracing runs beside it. Shown as
    /// "While moving", responsive to sharp.
    pub budget_ms: f64,
    /// Universes opened on their own, each within the one before
    /// (`Model::universe`, cut to what it holds): the view is of the last.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<i64>,
    /// Views saved by name, to go back to.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<SavedView>,
}

/// A view saved by name: the camera, the slice and the scene (its pins
/// left as they are when going back to it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedView {
    pub name: String,
    pub camera: CameraState,
    pub slice: SliceState,
    pub scene: Scene,
}

impl Default for ViewState {
    fn default() -> ViewState {
        ViewState {
            camera: None,
            scene: Scene::default(),
            slice: SliceState::default(),
            backend: BackendChoice::default(),
            budget_ms: DEFAULT_BUDGET_MS,
            open: Vec::new(),
            views: Vec::new(),
        }
    }
}

impl ViewState {
    pub fn new() -> ViewState {
        ViewState::default()
    }

    /// Read a state file (JSON).
    pub fn load(path: &std::path::Path) -> std::result::Result<ViewState, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut s: ViewState =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if !(s.budget_ms > 0.0) {
            s.budget_ms = DEFAULT_BUDGET_MS;
        }
        s.budget_ms = s.budget_ms.clamp(*BUDGET_MS.start(), *BUDGET_MS.end());
        Ok(s)
    }

    pub fn save(&self, path: &std::path::Path) -> std::result::Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// An orbit camera: looking at `target` from `distance` cm, at `yaw`
/// (about z, from +x) and `pitch` (above the xy plane), in degrees.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraState {
    pub target: [f64; 3],
    pub distance: f64,
    pub yaw_deg: f64,
    pub pitch_deg: f64,
    pub fov_deg: f64,
    /// Seen straight on (every ray parallel), as wide as the field of view
    /// is at the target.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ortho: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Colouring {
    #[default]
    Cell,
    Material,
    Universe,
    /// What materials look like (steel, concrete, water...).
    Realistic,
    /// Density in g/cm³, on a log scale.
    Density,
}

/// The colouring's name in the interface and the view file.
pub fn colouring_name(c: Colouring) -> &'static str {
    ColourBy::from(c).name()
}

impl From<Colouring> for ColourBy {
    fn from(c: Colouring) -> ColourBy {
        match c {
            Colouring::Cell => ColourBy::Cell,
            Colouring::Material => ColourBy::Material,
            Colouring::Universe => ColourBy::Universe,
            Colouring::Realistic => ColourBy::Realistic,
            Colouring::Density => ColourBy::Density,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipState {
    pub point: [f64; 3],
    pub normal: [f64; 3],
}

impl From<ClipState> for llevant::Clip {
    fn from(c: ClipState) -> llevant::Clip {
        llevant::Clip {
            point: c.point,
            normal: c.normal,
        }
    }
}

/// A cut of the 3D view (`llevant::Cutaway`), as the view file has it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CutState {
    /// The half-space `normal` points into, from `point`.
    Plane { point: [f64; 3], normal: [f64; 3] },
    /// The wedge about the axis through `point` along `axis`, from angle
    /// `from` to `to` (degrees, right-handed about the axis, from `zero`).
    Wedge {
        point: [f64; 3],
        axis: [f64; 3],
        zero: [f64; 3],
        from: f64,
        to: f64,
    },
    /// Everything outside the box (`keep`), or inside it.
    Box {
        lo: [f64; 3],
        hi: [f64; 3],
        keep: bool,
    },
}

impl CutState {
    pub fn cutaway(&self) -> llevant::Cutaway {
        use llevant::{Aabb, Clip, Cutaway};
        match *self {
            CutState::Plane { point, normal } => Cutaway::plane(Clip { point, normal }),
            CutState::Wedge {
                point,
                axis,
                zero,
                from,
                to,
            } => Cutaway::wedge(point, axis, zero, from, to),
            CutState::Box { lo, hi, keep } => {
                let b = Aabb { lo, hi };
                if keep {
                    Cutaway::keep_box(&b)
                } else {
                    Cutaway::remove_box(&b)
                }
            }
        }
    }

    /// For lists and the history.
    pub fn describe(&self) -> String {
        match self {
            CutState::Plane { .. } => "plane cut".into(),
            CutState::Wedge { from, to, .. } => format!("wedge {from:.0}°–{to:.0}°"),
            CutState::Box { keep: true, .. } => "section box".into(),
            CutState::Box { keep: false, .. } => "box cut".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AxisName {
    X,
    Y,
    #[default]
    Z,
}

impl From<AxisName> for Axis {
    fn from(a: AxisName) -> Axis {
        match a {
            AxisName::X => Axis::X,
            AxisName::Y => Axis::Y,
            AxisName::Z => Axis::Z,
        }
    }
}

/// Which boundaries the slice's outlines are drawn along.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutlineBy {
    /// Between cells (every cell instance: a filled universe's cells too).
    #[default]
    Cell,
    /// Between materials (void is one), of the deepest cells: what explains
    /// a field.
    Material,
    /// Between universes, of the deepest cells: where fills begin.
    Universe,
}

impl OutlineBy {
    pub const ALL: [OutlineBy; 3] = [OutlineBy::Cell, OutlineBy::Material, OutlineBy::Universe];

    pub fn name(self) -> &'static str {
        match self {
            OutlineBy::Cell => "cell",
            OutlineBy::Material => "material",
            OutlineBy::Universe => "universe",
        }
    }

    fn is_cell(&self) -> bool {
        *self == OutlineBy::Cell
    }
}

/// The 2D slice: the plane `axis = at`, centred on `centre` (the plane's two
/// coordinates, as in [`llevant::Slice::axis`]) and `width` cm wide.
/// `None` fields fit the model.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SliceState {
    pub axis: AxisName,
    pub at: Option<f64>,
    /// A plane through an axis at an angle, instead of `axis = at` (`at`
    /// then moves it off the axis).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radial: Option<Radial>,
    pub centre: Option<[f64; 2]>,
    pub width: Option<f64>,
    /// Lines along boundaries.
    pub outline: bool,
    /// Which boundaries the lines are along.
    #[serde(skip_serializing_if = "OutlineBy::is_cell")]
    pub outline_by: OutlineBy,
    /// The lines' colour (none: dark grey, `llevant_render::colours::OUTLINE`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_colour: Option<[u8; 3]>,
    /// Cell numbers written in the cells, and surface numbers along the
    /// boundaries (as MCNP's plotter labels them).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cell_labels: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub surface_labels: bool,
    /// A grid of the plane's coordinates, labelled along the edges.
    pub grid: bool,
}

impl Default for SliceState {
    fn default() -> SliceState {
        SliceState {
            axis: AxisName::default(),
            at: None,
            radial: None,
            centre: None,
            width: None,
            outline: true,
            outline_by: OutlineBy::Cell,
            outline_colour: None,
            cell_labels: false,
            surface_labels: false,
            grid: false,
        }
    }
}

/// Which plane the slice is on: what the slice plane's handles in 3D change, and
/// what undo goes back to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlaneState {
    pub axis: AxisName,
    pub radial: Option<Radial>,
    pub at: f64,
}

/// The plane through the axis through `point` along `axis`, at `phi`
/// degrees (right-handed about the axis) from `zero`: a vertical slice of a
/// torus about z, say.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Radial {
    pub point: [f64; 3],
    pub axis: [f64; 3],
    pub zero: [f64; 3],
    pub phi: f64,
}

impl Radial {
    /// About z through `point`, from +x.
    pub fn about_z(point: [f64; 3], phi: f64) -> Radial {
        Radial {
            point,
            axis: [0.0, 0.0, 1.0],
            zero: [1.0, 0.0, 0.0],
            phi,
        }
    }
}

/// Where a field's numbers come from: a CSV file (per cell, material,
/// instance or MCNP path) or a tally file (meshtal, MCTAL or cuvmsh) and which of
/// its tallies and bins.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FieldSource {
    pub path: std::path::PathBuf,
    /// Tally files: the tally (the first when left out).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tally: Option<u32>,
    /// Tally files: energy and time bin, `total`, `sum` or an index.
    #[serde(default = "total", skip_serializing_if = "is_total")]
    pub energy: String,
    #[serde(default = "total", skip_serializing_if = "is_total")]
    pub time: String,
    /// MCTAL: bins along the other dimensions, `u=1 m=0` (total, or the
    /// only bin, where not given).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bins: String,
    /// Tally files: relative errors instead of results.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub error: bool,
    /// Tally files: the MCNP input of the run, for what the file leaves
    /// out (MCTAL bins' cells, FMESH meshes exactly); the model's when left
    /// out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<std::path::PathBuf>,
}

fn total() -> String {
    "total".into()
}

fn is_total(s: &str) -> bool {
    s == "total"
}

impl FieldSource {
    pub fn new(path: std::path::PathBuf) -> FieldSource {
        FieldSource {
            path,
            tally: None,
            energy: total(),
            time: total(),
            bins: String::new(),
            error: false,
            input: None,
        }
    }

    /// The tally, bins and errors chosen, as read from a tally file.
    pub fn choice(&self) -> llevant::Result<llevant_scene::field::TallyChoice> {
        let mut c = llevant_scene::field::TallyChoice {
            tally: self.tally,
            energy: self.energy.parse()?,
            time: self.time.parse()?,
            error: self.error,
            ..Default::default()
        };
        c.set_bins(&self.bins)?;
        Ok(c)
    }

    /// This file (and run input) with the tally, bins and errors of `c`.
    pub fn with_choice(&self, c: &llevant_scene::field::TallyChoice) -> FieldSource {
        FieldSource {
            tally: c.tally,
            energy: c.energy.to_string(),
            time: c.time.to_string(),
            bins: c.other_bins(),
            error: c.error,
            ..self.clone()
        }
    }

    /// For the interface: the file's name, and the tally and bins chosen.
    pub fn describe(&self) -> String {
        let name = self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        let mut s = name;
        if let Some(t) = self.tally {
            s += &format!(", tally {t}");
        }
        if !is_total(&self.energy) {
            s += &format!(", energy {}", self.energy);
        }
        if !is_total(&self.time) {
            s += &format!(", time {}", self.time);
        }
        if !self.bins.trim().is_empty() {
            s += &format!(", {}", self.bins.trim());
        }
        if self.error {
            s += ", errors";
        }
        if let Some(i) = self.input.as_ref().and_then(|i| i.file_name()) {
            s += &format!(", input {}", i.to_string_lossy());
        }
        s
    }
}

/// The ends of a field's colour scale.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScaleState {
    pub lo: f64,
    pub hi: f64,
    pub log: bool,
}

/// Where the legend (the colour bar) sits on a view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    TopLeft,
    TopRight,
    #[default]
    BottomLeft,
    BottomRight,
}

/// How long the legend's bar is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegendSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl LegendSize {
    /// The bar's length (points).
    pub fn length(self) -> f32 {
        match self {
            LegendSize::Small => 120.0,
            LegendSize::Medium => 180.0,
            LegendSize::Large => 280.0,
        }
    }
}

/// How large the legend's text is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl TextSize {
    /// The numbers' size (points).
    pub fn points(self) -> f32 {
        match self {
            TextSize::Small => 10.0,
            TextSize::Medium => 12.0,
            TextSize::Large => 15.0,
        }
    }
}

/// How the legend writes its numbers: whichever, all of a bar in one
/// notation ([`llevant_render::legend::Numbers`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberFormat {
    /// `1e12`, `0.35`, `25` (decimals when all read well so, else all
    /// with an exponent).
    #[default]
    Short,
    /// `10¹²` (powers of ten as such, others as `3.2·10⁶`).
    Powers,
    /// `1000000000000`, `0.001`: as decimals.
    Decimal,
}

/// The legend of a field: where, how big, which way, its title (the
/// field's own when left out), its numbers and how many ticks.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Legend {
    pub corner: Corner,
    pub size: LegendSize,
    pub vertical: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub numbers: NumberFormat,
    /// More ticks than the ends and a few between.
    pub many_ticks: bool,
    /// Exactly this many ticks, ends included, evenly spaced, every one
    /// labelled (0: as `many_ticks` says).
    #[serde(skip_serializing_if = "is_zero_u8")]
    pub tick_count: u8,
    /// A tick at each edge between discrete bands (with bands; first of
    /// the tick choices).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub band_ticks: bool,
    /// At most this many significant digits (0: as few as write each
    /// number to 0.05 %, up to 4).
    #[serde(skip_serializing_if = "is_zero_u8")]
    pub digits: u8,
    /// The bar's length and thickness (points) instead of `size`'s and
    /// the default's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thickness: Option<f32>,
    pub text: TextSize,
}

fn is_zero_u8(n: &u8) -> bool {
    *n == 0
}

/// The legend's bar thickness by default (points).
pub const BAR_THICKNESS: f32 = 10.0;

impl Legend {
    pub fn ticks(&self) -> llevant_render::legend::Ticks {
        use llevant_render::legend::Ticks;
        match (self.tick_count, self.many_ticks) {
            _ if self.band_ticks => Ticks::Edges,
            (n @ 2.., _) => Ticks::Count(n),
            (_, true) => Ticks::Many,
            _ => Ticks::Auto,
        }
    }

    pub fn tick_style(&self) -> llevant_render::legend::TickStyle {
        use llevant_render::legend::Numbers;
        llevant_render::legend::TickStyle {
            ticks: self.ticks(),
            numbers: match self.numbers {
                NumberFormat::Short => Numbers::Auto,
                NumberFormat::Powers => Numbers::Powers,
                NumberFormat::Decimal => Numbers::Decimal,
            },
            digits: (self.digits > 0).then_some(self.digits),
        }
    }

    pub fn length(&self) -> f32 {
        self.length
            .unwrap_or(self.size.length())
            .clamp(40.0, 2000.0)
    }

    pub fn thickness(&self) -> f32 {
        self.thickness.unwrap_or(BAR_THICKNESS).clamp(4.0, 60.0)
    }

    pub fn corner(&self) -> llevant_render::legend::Corner {
        use llevant_render::legend::Corner as C;
        match self.corner {
            Corner::TopLeft => C::TopLeft,
            Corner::TopRight => C::TopRight,
            Corner::BottomLeft => C::BottomLeft,
            Corner::BottomRight => C::BottomRight,
        }
    }
}

/// How a field looks: its colour map (by name: viridis, turbo, inferno,
/// grey), discrete bands (0: continuous) and its legend.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FieldStyle {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ramp: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub bands: u32,
    pub legend: Legend,
    /// The colours of values below and above the scale, instead of its
    /// ends' (marked by triangles at the bar's ends).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub below: Option<[u8; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub above: Option<[u8; 3]>,
    /// A fitted scale keeps the values' own ends rather than rounding them
    /// out (to decades, or to a 1-2-5 step).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub exact_ends: bool,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl FieldStyle {
    pub fn is_default(&self) -> bool {
        *self == FieldStyle::default()
    }

    /// The colour map (viridis unless named otherwise).
    pub fn ramp(&self) -> llevant_render::scale::Ramp {
        llevant_render::scale::Ramp::from_name(&self.ramp).unwrap_or_default()
    }
}

/// A field drawn over the cells: where its numbers come from and how it is
/// drawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldState {
    #[serde(flatten)]
    pub source: FieldSource,
    /// The colour scale; fitted to the values when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<ScaleState>,
    /// How much the field's colours cover the cells' (0 to 1).
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub shown: bool,
    /// Colour map, bands and legend.
    #[serde(default, skip_serializing_if = "FieldStyle::is_default")]
    pub style: FieldStyle,
    /// The geometry's colours under the field (and where it has no value)
    /// rather than a neutral grey.
    #[serde(default, skip_serializing_if = "is_false")]
    pub geometry_under: bool,
    /// Only values from the first to the second are drawn (each end
    /// included; none, no bound), the others as no value.
    #[serde(default, skip_serializing_if = "no_threshold")]
    pub threshold: [Option<f64>; 2],
}

fn no_threshold(t: &[Option<f64>; 2]) -> bool {
    t.iter().all(Option::is_none)
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

impl FieldState {
    pub fn new(source: FieldSource) -> FieldState {
        FieldState {
            source,
            scale: None,
            opacity: 1.0,
            shown: true,
            style: FieldStyle::default(),
            geometry_under: false,
            threshold: [None; 2],
        }
    }
}

/// Post-process effects of the 3D view, on or off.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Effects {
    pub outline: bool,
    pub ambient_occlusion: bool,
    /// The outlines' colour (none: black, darkening the image).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_colour: Option<[u8; 3]>,
}

impl Default for Effects {
    fn default() -> Effects {
        Effects {
            outline: true,
            ambient_occlusion: true,
            outline_colour: None,
        }
    }
}

/// Who traces the 3D view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendChoice {
    /// On a hardware GPU, previews on the CPU (which keeps the window
    /// smooth) and full resolution on the GPU (fast in bulk); all on the CPU
    /// on a software adapter.
    #[default]
    Auto,
    /// Everything on the GPU.
    Gpu,
    /// Everything on the CPU (exact).
    Cpu,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip() {
        let mut s = ViewState::new();
        s.camera = Some(CameraState {
            target: [1.0, 2.0, 3.0],
            distance: 400.0,
            yaw_deg: 30.0,
            pitch_deg: -10.0,
            fov_deg: 30.0,
            ortho: false,
        });
        s.scene.hiding.cells = vec![10, 20];
        s.scene.clip = Some(ClipState {
            point: [0.0; 3],
            normal: [0.0, 1.0, 0.0],
        });
        s.scene.colour_by = Colouring::Material;
        s.backend = BackendChoice::Cpu;
        s.scene.materials.insert(12, "copper roughness=0.2".into());
        s.scene.pins.push(crate::scene::Pin {
            point: [1.0, -2.0, 3.5],
            note: "flange".into(),
        });
        s.scene.hiding.level = Some(0);
        s.scene.hiding.ghost = vec!["c 300".parse().unwrap(), "u 20040 & m 5".parse().unwrap()];
        let text = serde_json::to_string(&s).unwrap();
        assert!(
            text.contains(r#""level":0"#) && text.contains(r#""ghost":["#),
            "{text}"
        );
        assert_eq!(serde_json::from_str::<ViewState>(&text).unwrap(), s);
    }

    /// A tally choice as the panel makes it, in the view file as before
    /// (strings), with the run's input when given.
    #[test]
    fn field_sources_hold_tally_choices() {
        use llevant_scene::field::Bin;
        let s: FieldSource =
            serde_json::from_str(r#"{"path": "run.m", "tally": 54, "bins": "m=1"}"#).unwrap();
        let mut c = s.choice().unwrap();
        assert_eq!(c.other[3], Bin::Index(1));
        c.set_bins("m=sum u=2").unwrap();
        c.energy = Bin::Index(3);
        let t = s.with_choice(&c);
        assert_eq!((t.bins.as_str(), t.energy.as_str()), ("u=2 m=sum", "3"));
        assert_eq!(t.choice().unwrap(), c);
        let json = serde_json::to_string(&FieldSource {
            input: Some("run.i".into()),
            ..t.clone()
        })
        .unwrap();
        assert!(json.contains(r#""input":"run.i""#), "{json}");
        assert!(!serde_json::to_string(&t).unwrap().contains("input"));
    }

    #[test]
    fn partial_files() {
        let s: ViewState =
            serde_json::from_str(r#"{"colour_by": "universe", "slice": {"axis": "x"}}"#).unwrap();
        assert_eq!(s.scene.colour_by, Colouring::Universe);
        assert_eq!(s.slice.axis, AxisName::X);
        assert_eq!(s.camera, None);
        assert!(s.scene.effects.outline);
        assert!(s.scene.hiding.void);
        let s: ViewState =
            serde_json::from_str(r#"{"colour_by": "realistic", "materials": {"12": "steel"}}"#)
                .unwrap();
        assert_eq!(s.scene.materials[&12], "steel");
        // Files of before lighting followed the colours.
        let s: ViewState = serde_json::from_str(r#"{"shading": "pbr"}"#).unwrap();
        assert_eq!(s.scene.colour_by, Colouring::Cell);
    }

    #[test]
    fn readme_view_file_reads() {
        let readme = include_str!("../README.md");
        let start = readme.find("```json\n").expect("a JSON example") + 8;
        let len = readme[start..].find("```").unwrap();
        let s: ViewState = serde_json::from_str(&readme[start..start + len]).unwrap();
        assert_eq!(s.scene.materials[&13], "copper roughness=0.2");
        assert_eq!(s.scene.pins[0].note, "flange");
        assert_eq!(s.scene.colour_by, Colouring::Realistic);
    }

    #[test]
    fn example_view_file_reads() {
        let s: ViewState =
            serde_json::from_str(include_str!("../examples/tokamak.view.json")).unwrap();
        assert!(s.camera.is_some());
    }

    #[test]
    fn scene_fields_stay_at_the_top_level() {
        let mut s = ViewState::new();
        s.scene.hiding.cells = vec![7];
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        assert_eq!(v["hidden_cells"], serde_json::json!([7]));
        assert_eq!(v["colour_by"], "cell");
        assert!(v.get("scene").is_none() && v.get("pins").is_none());
    }
}
