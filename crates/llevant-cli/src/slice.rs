//! `slice`: an image of a plane through the geometry, with exact boundaries.

use std::path::PathBuf;
use std::time::Instant;

use llevant::{Axis, Depth, Scope, Slice};
use llevant_render::colours::{self, Paint};
use llevant_render::png::write_png;

use crate::args::{Colour, LookArgs, Open, OutlineBy};
use crate::common::{csv_field, err, load, region};
use crate::field::{with_bar, FieldArgs};

#[derive(clap::Args)]
pub(crate) struct SliceArgs {
    /// The MCNP input file.
    model: PathBuf,
    /// The plane: x=…, y=… or z=….
    #[arg(long, allow_hyphen_values = true)]
    plane: String,
    /// Box to show, x0,y0,z0,x1,y1,z1 (default: the model's extent).
    #[arg(long, allow_hyphen_values = true)]
    region: Option<String>,
    /// Image width in pixels (the height keeps pixels square).
    #[arg(long, default_value_t = 1000)]
    px: usize,
    /// What the colours show.
    #[arg(long, value_enum, default_value = "cell")]
    colour: Colour,
    /// No lines along boundaries.
    #[arg(help_heading = "Outlines", long)]
    no_outline: bool,
    /// Which boundaries the lines are along: between cells, or between
    /// the materials (void is one) or universes of the deepest cells
    /// (cells with --level).
    #[arg(
        help_heading = "Outlines",
        long,
        value_enum,
        default_value = "cell",
        conflicts_with = "no_outline"
    )]
    outline_by: OutlineBy,
    /// Show universe level N only, as MCNP's plotter (LEVEL N): 0 is
    /// the top, where filled cells are shown as cells.
    #[arg(long)]
    level: Option<usize>,

    /// Also write the label of every pixel (MCNP terms) as CSV.
    #[arg(long)]
    labels: Option<PathBuf>,
    #[command(flatten)]
    looks: LookArgs,
    #[command(flatten)]
    fields: FieldArgs,
    /// The PNG image to write.
    #[arg(short, long)]
    output: PathBuf,
}

impl SliceArgs {
    pub(crate) fn run(self, open: &Open) -> Result<u8, String> {
        let SliceArgs {
            model,
            plane,
            region: r,
            px,
            colour,
            no_outline,
            outline_by,
            level,
            labels,
            looks,
            fields,
            output,
        } = self;
        let m = load(&model, open)?;
        let looks = looks.looks(&m)?;
        let field = fields.field(&m, &model)?;
        let (axis, at) = parse_plane(&plane)?;
        let s = Slice::axis(axis, at, &region(&m, &r, Scope::Contents)?, px);
        let t = Instant::now();
        let tr = m.slice(&s, Depth::from(level)).map_err(err)?;
        let ids = s.pixels(&tr);
        eprintln!(
            "sliced {}×{} in {:.0} ms",
            s.nx,
            s.ny,
            1e3 * t.elapsed().as_secs_f64()
        );
        let paint = Paint::new(&m, &looks, colour.into())
            .with_field(field.as_ref())
            .with_level(level);
        let outline = (!no_outline).then_some(outline_by.into());
        let img = colours::slice_rgb(&paint, &s, &tr.labels, &ids, outline);
        let (img, h) = with_bar(img, s.nx, s.ny, field.as_ref(), &fields);
        write_png(&output, s.nx, h, &img)?;
        if let Some(f) = labels {
            let mut out = String::from("i,j,x,y,z,label\n");
            for j in 0..s.ny {
                for i in 0..s.nx {
                    let p = s.pixel_centre(i, j);
                    let l = &tr.labels[ids[j * s.nx + i] as usize];
                    out += &format!(
                        "{i},{j},{},{},{},{}\n",
                        p[0],
                        p[1],
                        p[2],
                        csv_field(&m.describe(l))
                    );
                }
            }
            std::fs::write(&f, out).map_err(|e| format!("{}: {e}", f.display()))?;
        }
        eprintln!("wrote {}", output.display());
        Ok(0)
    }
}

fn parse_plane(s: &str) -> Result<(Axis, f64), String> {
    let (a, v) = s
        .split_once('=')
        .ok_or_else(|| format!("--plane: expected x=…, y=… or z=…, got `{s}`"))?;
    let axis = match a.trim().to_ascii_lowercase().as_str() {
        "x" | "px" => Axis::X,
        "y" | "py" => Axis::Y,
        "z" | "pz" => Axis::Z,
        _ => return Err(format!("--plane: unknown axis `{a}`")),
    };
    let at = v
        .trim()
        .parse()
        .map_err(|_| format!("--plane: bad position `{v}`"))?;
    Ok((axis, at))
}
