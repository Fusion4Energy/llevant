//! `--field`: results drawn over slices and renders (values per cell, instance
//! or mesh voxel), their colour scale, and the colour bar under the image.

use std::path::PathBuf;

use clap::ValueEnum;
use llevant::Model;
use llevant_render::field::{FieldLook, Overlay};
use llevant_render::legend;
use llevant_render::scale::{self, Scale};
use llevant_scene::field;

/// A field drawn over the image: values per cell, instance or mesh voxel.
#[derive(clap::Args)]
pub(crate) struct FieldArgs {
    /// Values to colour the cells by: a CSV file whose first column is
    /// `cell`, `material`, `instance` or `path` (an instance in MCNP
    /// notation, `12 < 100[1 0 0] < 5`) and whose second holds the values
    /// (its header is the name, `dose [Sv/h]`); an MCNP meshtal file
    /// (FMESH tallies, on their mesh: exact, with TR=, when the model's
    /// input has the FMESH card); a D1SUNED cuvmsh (FMESH with out=cuv,
    /// as each voxel's flux: the mesh without out=cuv); or an MCNP MCTAL
    /// file (F4, F6, F7, F8
    /// cell tallies, per cell or, for repeated structures and filled cells,
    /// per instance; bins as the model's input writes them).
    #[arg(help_heading = "Field", long)]
    field: Option<PathBuf>,
    /// Tally files: the tally number (default: the first; for MCTAL the
    /// first cell tally).
    #[arg(help_heading = "Field", long, requires = "field")]
    tally: Option<u32>,
    /// Tally files: energy bin, `total`, `sum` (of its bins) or an index from 0.
    #[arg(
        help_heading = "Field",
        long,
        requires = "field",
        default_value = "total"
    )]
    energy: String,
    /// Tally files: time bin, `total`, `sum` (of its bins) or an index from 0.
    #[arg(
        help_heading = "Field",
        long,
        requires = "field",
        default_value = "total"
    )]
    time: String,
    /// MCTAL: the bin along another dimension (d, u, s, m or c), `u=1` or
    /// `m=sum` (default: total, or the only bin). May be repeated.
    #[arg(
        help_heading = "Field",
        long,
        requires = "field",
        value_name = "DIM=BIN"
    )]
    bin: Vec<String>,
    /// Tally files: the MCNP input of the run that wrote them, for what
    /// they leave out (MCTAL bins' cells, FMESH meshes exactly; default:
    /// the model).
    #[arg(help_heading = "Field", long, requires = "field", value_name = "INPUT")]
    run_input: Option<PathBuf>,
    /// Tally files: the relative errors instead of the results.
    #[arg(help_heading = "Field", long, requires = "field")]
    error: bool,
    /// The colour scale's ends, `lo,hi` (default: fitted to the values).
    #[arg(
        help_heading = "Field colours",
        long,
        requires = "field",
        allow_hyphen_values = true
    )]
    scale: Option<String>,
    /// A logarithmic or a linear scale (default: logarithmic when the
    /// values are positive and span two decades).
    #[arg(
        help_heading = "Field colours",
        long,
        requires = "field",
        conflicts_with = "linear"
    )]
    log: bool,
    #[arg(help_heading = "Field colours", long, requires = "field")]
    linear: bool,
    /// How much the field's colours cover the cells' (0 to 1).
    #[arg(
        help_heading = "Field colours",
        long,
        requires = "field",
        default_value_t = 1.0
    )]
    opacity: f32,
    /// The cells' own colours under the field and where it has no value
    /// (default: a neutral grey, so that only the scale's colours read as
    /// values).
    #[arg(help_heading = "Field colours", long, requires = "field")]
    cell_colours_under: bool,
    /// Keep a fitted scale's ends at the values' own (default: moved out
    /// to round numbers, decades on a log scale).
    #[arg(
        help_heading = "Field colours",
        long,
        requires = "field",
        conflicts_with = "scale"
    )]
    exact_ends: bool,
    /// Only values from LO to HI drawn, the others as no value: `lo,hi`,
    /// either side left empty for no bound (`1e3,`).
    #[arg(
        help_heading = "Field colours",
        long,
        requires = "field",
        allow_hyphen_values = true
    )]
    threshold: Option<String>,
    /// Values below the scale (and zeros on a log scale) in this colour,
    /// `#rrggbb`, instead of the scale's low end's.
    #[arg(help_heading = "Field colours", long, requires = "field")]
    below_colour: Option<String>,
    /// Values above the scale in this colour, `#rrggbb`.
    #[arg(help_heading = "Field colours", long, requires = "field")]
    above_colour: Option<String>,
    /// The colour map: viridis, turbo, inferno or grey.
    #[arg(
        help_heading = "Field colours",
        long,
        requires = "field",
        default_value = "viridis"
    )]
    ramp: String,
    /// Discrete bands of one colour each (2 to 64; on a log scale, of equal
    /// decades) instead of a continuous ramp.
    #[arg(help_heading = "Field colours", long, requires = "field")]
    bands: Option<u32>,
    /// The colour bar's ticks: `few` (round values), `many` (every decade
    /// of a log scale), a number (exactly so many, evenly spaced, every one
    /// labelled) or `edges` (at the edges between --bands).
    #[arg(
        help_heading = "Legend",
        long,
        requires = "field",
        default_value = "few"
    )]
    ticks: String,
    /// How the colour bar writes its numbers (all in one notation).
    #[arg(
        help_heading = "Legend",
        long,
        value_enum,
        requires = "field",
        default_value = "auto"
    )]
    numbers: Numbers,
    /// At most this many significant digits in the colour bar's numbers.
    #[arg(help_heading = "Legend", long, requires = "field")]
    digits: Option<u8>,
    /// The colour bar's title (default: the field's name and unit).
    #[arg(help_heading = "Legend", long, requires = "field")]
    legend_title: Option<String>,
    /// The colour bar's text size in pixels (default: scaled with the
    /// image's width).
    #[arg(help_heading = "Legend", long, requires = "field")]
    legend_text: Option<f32>,
    /// No colour bar under the image.
    #[arg(help_heading = "Legend", long, requires = "field")]
    no_bar: bool,
}

/// How a colour bar writes its numbers.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Numbers {
    /// Decimals when all read well so, else all with an exponent (`1e3`).
    Auto,
    /// Powers of ten (`10³`).
    Powers,
    /// Decimals, however small or large.
    Decimal,
}

impl FieldArgs {
    /// The field asked for, if any, with its scale and opacity.
    pub(crate) fn field(
        &self,
        m: &Model,
        model_path: &std::path::Path,
    ) -> Result<Option<Overlay>, String> {
        let Some(path) = &self.field else {
            return Ok(None);
        };
        let ctx = |e: llevant::Error| format!("--field: {e}");
        let mut choice = field::TallyChoice {
            tally: self.tally,
            energy: self.energy.parse().map_err(ctx)?,
            time: self.time.parse().map_err(ctx)?,
            error: self.error,
            ..field::TallyChoice::default()
        };
        for b in &self.bin {
            choice.set_bins(b).map_err(|e| format!("--bin: {e}"))?;
        }
        let input = self.run_input.as_deref().unwrap_or(model_path);
        let data = field::read(m, path, Some(input), &choice).map_err(ctx)?;
        let mut f = FieldLook::fitted(&data);
        if self.log || self.linear {
            let finite = data.numbers().map(f64::from).filter(|x| x.is_finite());
            f.scale = Scale::auto(finite, self.log).ok_or("--field: no values to scale")?;
        }
        if self.scale.is_none() && !self.exact_ends {
            f.scale = llevant_render::legend::rounded(&f.scale);
        }
        if let Some(s) = &self.scale {
            let v: Vec<f64> = s
                .split(',')
                .map(|x| x.trim().parse::<f64>())
                .collect::<Result<_, _>>()
                .map_err(|_| format!("--scale: expected lo,hi, got `{s}`"))?;
            if v.len() != 2 || !(v[0] < v[1]) || (f.scale.log && !(v[0] > 0.0)) {
                return Err(format!(
                    "--scale: give lo < hi (and lo > 0 on a log scale), got `{s}`"
                ));
            }
            f.scale.lo = v[0];
            f.scale.hi = v[1];
        }
        if !(0.0..=1.0).contains(&self.opacity) {
            return Err("--opacity: give a number from 0 to 1".into());
        }
        f.opacity = self.opacity;
        if self.cell_colours_under {
            f.under = llevant_render::field::Under::Geometry;
        }
        f.scale.ramp = scale::Ramp::from_name(&self.ramp).ok_or_else(|| {
            format!(
                "--ramp: viridis, turbo, inferno or grey, got `{}`",
                self.ramp
            )
        })?;
        if let Some(b) = self.bands {
            if !(2..=64).contains(&b) {
                return Err(format!("--bands: from 2 to 64, got {b}"));
            }
            f.scale.bands = b;
        }
        self.tick_style()?;
        if let Some(t) = &self.threshold {
            let bad = || format!("--threshold: expected lo,hi (either may be empty), got `{t}`");
            let (a, b) = t.split_once(',').ok_or_else(bad)?;
            let bound = |x: &str| -> Result<Option<f64>, String> {
                let x = x.trim();
                if x.is_empty() {
                    Ok(None)
                } else {
                    x.parse().map(Some).map_err(|_| bad())
                }
            };
            f.threshold = [bound(a)?, bound(b)?];
            if let [Some(a), Some(b)] = f.threshold {
                if !(a <= b) {
                    return Err(bad());
                }
            }
        }
        f.below = self
            .below_colour
            .as_deref()
            .map(|c| hex_colour(c, "--below-colour"))
            .transpose()?;
        f.above = self
            .above_colour
            .as_deref()
            .map(|c| hex_colour(c, "--above-colour"))
            .transpose()?;
        eprintln!(
            "field {}: {} to {}{}",
            data.title(),
            scale::tick_label(f.scale.lo),
            scale::tick_label(f.scale.hi),
            if f.scale.log { ", logarithmic" } else { "" }
        );
        Ok(Some(Overlay {
            field: std::sync::Arc::new(data),
            look: f,
        }))
    }
}

/// `#rrggbb` (or `rrggbb`) as a colour.
fn hex_colour(s: &str, what: &str) -> Result<[u8; 3], String> {
    let h = s.trim().trim_start_matches('#');
    let byte = |k: usize| h.get(k..k + 2).and_then(|x| u8::from_str_radix(x, 16).ok());
    match (h.len(), byte(0), byte(2), byte(4)) {
        (6, Some(r), Some(g), Some(b)) => Ok([r, g, b]),
        _ => Err(format!("{what}: expected #rrggbb, got `{s}`")),
    }
}

impl FieldArgs {
    /// The colour bar's ticks and numbers.
    fn tick_style(&self) -> Result<legend::TickStyle, String> {
        let ticks = match self.ticks.as_str() {
            "few" => legend::Ticks::Auto,
            "many" => legend::Ticks::Many,
            "edges" if self.bands.is_some() => legend::Ticks::Edges,
            "edges" => return Err("--ticks edges: give --bands too".into()),
            n => match n.parse::<u8>() {
                Ok(n) if n >= 2 => legend::Ticks::Count(n),
                _ => {
                    return Err(format!(
                        "--ticks: few, many, edges or a number from 2, got `{n}`"
                    ))
                }
            },
        };
        if self.digits.is_some_and(|d| !(1..=6).contains(&d)) {
            return Err("--digits: from 1 to 6".into());
        }
        Ok(legend::TickStyle {
            ticks,
            numbers: match self.numbers {
                Numbers::Auto => legend::Numbers::Auto,
                Numbers::Powers => legend::Numbers::Powers,
                Numbers::Decimal => legend::Numbers::Decimal,
            },
            digits: self.digits,
        })
    }
}

/// `rgb` (`w` wide) with the colour bar of `f` under it, unless not wanted
/// (`llevant_render::legend::strip`: as the viewer's legend, across the
/// width); and the new height.
pub(crate) fn with_bar(
    rgb: Vec<u8>,
    w: usize,
    h: usize,
    f: Option<&Overlay>,
    args: &FieldArgs,
) -> (Vec<u8>, usize) {
    match f {
        Some(f) if !args.no_bar => {
            let title = args.legend_title.clone().unwrap_or_else(|| f.field.title());
            let look = &f.look;
            let spec = legend::Spec {
                scale: &look.scale,
                title: &title,
                // Checked when the field was read.
                style: args.tick_style().unwrap_or_default(),
                vertical: false,
                length: 0.0,
                thickness: 12.0,
                text: args.legend_text.unwrap_or(0.0).max(0.0),
                corner: legend::Corner::TopLeft,
                below: look.below,
                above: look.above,
            };
            let (bar, bh) = legend::strip(&spec, w);
            let mut out = rgb;
            out.extend(bar);
            (out, h + bh)
        }
        _ => (rgb, h),
    }
}
