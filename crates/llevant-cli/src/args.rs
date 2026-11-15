//! Arguments several commands share: which part of the model to open, output
//! formats, colours and groups, material looks, volume grids.

use std::path::PathBuf;

use clap::ValueEnum;
use llevant::{Aabb, Cut, Model};
use llevant_render::colours::{self, ColourBy};
use llevant_scene::hiding::ProblemScope;
use llevant_scene::materials::{Looks, Spec};

use crate::common::err;

/// Which part of the model to open.
#[derive(clap::Args, Clone)]
pub(crate) struct Open {
    /// Open this universe as the model, in its own frame, with everything it
    /// holds (cell and universe numbers stay the same).
    #[arg(long, global = true)]
    pub(crate) universe: Option<i64>,
    /// With --universe: the box it is cut to, beyond which lies one void
    /// IMP:N=0 cell. `own` (around what it holds, measured), `none`, or
    /// `x0,y0,z0,x1,y1,z1` in cm (in the universe's frame).
    #[arg(long, global = true, default_value = "own")]
    cut: String,
}

impl Open {
    pub(crate) fn cut(&self) -> Result<Cut, String> {
        match self.cut.as_str() {
            "own" => Ok(Cut::Own),
            "none" => Ok(Cut::None),
            s => {
                let v: Vec<f64> = s
                    .split(',')
                    .map(|x| x.trim().parse::<f64>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| {
                        format!("--cut: expected own, none or x0,y0,z0,x1,y1,z1, got `{s}`")
                    })?;
                if v.len() != 6 {
                    return Err(format!("--cut: expected 6 numbers, got {}", v.len()));
                }
                Ok(Cut::Box(Aabb {
                    lo: [v[0], v[1], v[2]],
                    hi: [v[3], v[4], v[5]],
                }))
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Format {
    Text,
    Csv,
    Json,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Problems {
    Selection,
    All,
}

impl From<Problems> for ProblemScope {
    fn from(p: Problems) -> ProblemScope {
        match p {
            Problems::Selection => ProblemScope::Selection,
            Problems::All => ProblemScope::All,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum OutlineBy {
    Cell,
    Material,
    Universe,
}

impl From<OutlineBy> for colours::Outlines {
    fn from(o: OutlineBy) -> colours::Outlines {
        match o {
            OutlineBy::Cell => colours::Outlines::Cell,
            OutlineBy::Material => colours::Outlines::Material,
            OutlineBy::Universe => colours::Outlines::Universe,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum GroupBy {
    Cell,
    Material,
    Universe,
    /// Every label: the full path through the universes.
    Label,
}

impl GroupBy {
    pub(crate) fn name(self) -> &'static str {
        match self {
            GroupBy::Cell => "cell",
            GroupBy::Material => "material",
            GroupBy::Universe => "universe",
            GroupBy::Label => "label",
        }
    }
}

impl From<GroupBy> for llevant::GroupBy {
    fn from(g: GroupBy) -> Self {
        match g {
            GroupBy::Cell => llevant::GroupBy::Cell,
            GroupBy::Material => llevant::GroupBy::Material,
            GroupBy::Universe => llevant::GroupBy::Universe,
            GroupBy::Label => llevant::GroupBy::Label,
        }
    }
}

/// What a colour stands for.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Colour {
    /// The deepest cell (MCNP number).
    Cell,
    /// The deepest cell's material (void is white).
    Material,
    /// The universe of the deepest cell.
    Universe,
    /// What the deepest cell's material looks like (steel, concrete,
    /// water...; `llevant materials` lists them).
    Realistic,
    /// The deepest cell's density in g/cm³, on a log scale.
    Density,
}

impl From<Colour> for ColourBy {
    fn from(c: Colour) -> Self {
        match c {
            Colour::Cell => ColourBy::Cell,
            Colour::Material => ColourBy::Material,
            Colour::Universe => ColourBy::Universe,
            Colour::Realistic => ColourBy::Realistic,
            Colour::Density => ColourBy::Density,
        }
    }
}

/// Looks of materials: a file and single overrides.
#[derive(clap::Args)]
pub(crate) struct LookArgs {
    /// File of material looks, one per line: `12 steel roughness=0.2`,
    /// `13 colour=#806040 metalness=0` (`#` after a space starts a comment).
    #[arg(help_heading = "Material looks", long)]
    materials: Option<PathBuf>,
    /// The look of one material, `ID=SPEC` (`--look "13=copper roughness=0.2"`);
    /// after --materials.
    #[arg(help_heading = "Material looks", long)]
    look: Vec<String>,
}

impl LookArgs {
    fn specs(&self) -> Result<Vec<(i64, Spec)>, String> {
        let mut out = match &self.materials {
            Some(f) => {
                let text =
                    std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
                llevant_scene::materials::parse_specs(&text)
                    .map_err(|e| format!("{}: {e}", f.display()))?
            }
            None => Vec::new(),
        };
        for l in &self.look {
            let (id, spec) = l
                .split_once('=')
                .ok_or_else(|| format!("--look: expected ID=SPEC, got `{l}`"))?;
            let id: i64 = id
                .trim()
                .trim_start_matches(['m', 'M'])
                .parse()
                .map_err(|_| format!("--look: bad material number `{id}`"))?;
            let spec: Spec = spec.parse().map_err(|e| format!("--look: {e}"))?;
            out.retain(|(i, _)| *i != id);
            out.push((id, spec));
        }
        Ok(out)
    }

    /// The looks of `m`'s materials with the overrides asked for.
    pub(crate) fn looks(&self, m: &Model) -> Result<Looks, String> {
        Looks::new(m, &self.specs()?).map_err(err)
    }
}

#[derive(clap::Args)]
pub(crate) struct GridArgs {
    /// Box to measure, x0,y0,z0,x1,y1,z1 (default: the model's extent).
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) region: Option<String>,
    /// Seed of the grid offset (the same seed gives the same lines).
    #[arg(long, default_value_t = 1)]
    pub(crate) seed: u64,
    /// Independent lattices of lines, each in a random direction; sigma is
    /// the standard error over them (at least 2).
    #[arg(long, default_value_t = llevant::DEFAULT_PROBES)]
    pub(crate) probes: usize,
}
