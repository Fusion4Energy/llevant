//! `render`: a 3D view, the first visible cell along each pixel's ray, shaded
//! (on the CPU, or the GPU with `--gpu`).

use std::path::PathBuf;
use std::time::Instant;

use llevant::{Aabb, Camera, Clip, Cutaway, Scope, Visibility, NO_HIT};
use llevant_render::colours::{ColourBy, Paint};
use llevant_render::ghost;
use llevant_render::png::write_png;
use llevant_render::shading::{self, Probe};
use llevant_scene::bounds;
use llevant_scene::hiding::Hiding;
use llevant_scene::query::Query;

use crate::args::{Colour, LookArgs, Open, Problems};
use crate::common::{box_arg, err, fmt_point, framing, index, load, numbers, region, rule, vec3};
use crate::field::{with_bar, FieldArgs};

/// `render --from` when it is not given.
const DEFAULT_FROM: [f64; 3] = [-0.6, -0.7, 0.4];

#[derive(clap::Args)]
pub(crate) struct RenderArgs {
    /// The MCNP input file.
    model: PathBuf,
    /// Eye position x,y,z (with --target; default: fit the region).
    #[arg(
        help_heading = "Camera",
        long,
        allow_hyphen_values = true,
        requires = "target"
    )]
    eye: Option<String>,
    #[arg(help_heading = "Camera", long, allow_hyphen_values = true)]
    target: Option<String>,
    /// Direction from the region's centre towards the eye, when fitting
    /// (default -0.6,-0.7,0.4; with --isolate, the direction from a
    /// cube's faces, edges and corners where the cells show most, unless
    /// the default shows nearly as much).
    #[arg(help_heading = "Camera", long, allow_hyphen_values = true)]
    from: Option<String>,
    /// Box to frame, x0,y0,z0,x1,y1,z1 (default: around the cells of
    /// --isolate, or the model's extent).
    #[arg(help_heading = "Camera", long, allow_hyphen_values = true)]
    region: Option<String>,
    /// Image size WxH.
    #[arg(help_heading = "Camera", long, default_value = "1000x1000")]
    size: String,
    /// Vertical field of view in degrees (with --eye).
    #[arg(help_heading = "Camera", long, default_value_t = 30.0)]
    fov: f64,
    /// Cut away the half-space in front of a plane: px,py,pz,nx,ny,nz
    /// (the normal points into the part removed). May be repeated.
    #[arg(help_heading = "Cuts", long, allow_hyphen_values = true)]
    clip: Vec<String>,
    /// Cut away a wedge about an axis: px,py,pz,ax,ay,az,from,to (the
    /// axis through p along a; angles in degrees, right-handed about the
    /// axis, from x, or from y for an axis along x). May be repeated.
    #[arg(help_heading = "Cuts", long, allow_hyphen_values = true)]
    wedge: Vec<String>,
    /// Keep only what is inside a box: x0,y0,z0,x1,y1,z1.
    #[arg(help_heading = "Cuts", long = "box", allow_hyphen_values = true)]
    keep_box: Option<String>,
    /// Cut away what is inside a box: x0,y0,z0,x1,y1,z1. May be repeated.
    #[arg(help_heading = "Cuts", long, allow_hyphen_values = true)]
    remove_box: Vec<String>,
    /// See it straight on: an orthographic view this many cm high.
    #[arg(help_heading = "Camera", long)]
    ortho: Option<f64>,
    /// The image's up direction x,y,z (with --eye; default z, or y when
    /// looking along z).
    #[arg(help_heading = "Camera", long, allow_hyphen_values = true)]
    up: Option<String>,
    /// Hide the cells of a rule (`u 20040 & m 5`, `label "BSV"` for the
    /// cells whose `$` comments contain BSV, as the viewer's).
    /// May be repeated. Rule options take `@file` too: a file holding
    /// the rule, one per line (any of them), or a list of cell numbers
    /// (`#` starts a comment).
    #[arg(help_heading = "Hiding", long)]
    hide_rule: Vec<String>,
    /// Show only the cells of a rule, what they hold and the cells
    /// holding them; the view is framed on every instance of them
    /// (`llevant bbox` gives the box).
    #[arg(help_heading = "Hiding", long)]
    isolate: Option<String>,
    /// With --isolate, whose gaps and overlaps are drawn: those of the
    /// cells isolated and what they hold, or every one (`llevant check`
    /// lists them).
    #[arg(help_heading = "Hiding", long, value_enum, default_value = "selection")]
    problems: Problems,
    /// Show universe level N only, as MCNP's plotter (LEVEL N): 0 is
    /// the top, where filled cells are shown as cells.
    #[arg(help_heading = "Hiding", long)]
    level: Option<usize>,
    /// Draw the cells of a rule as ghosts: shells over what is behind
    /// them, what fills them shown (`c 300`: an envelope over its
    /// filler). May be repeated.
    #[arg(help_heading = "Hiding", long)]
    ghost: Vec<String>,
    /// See through gaps (and overlaps): their pattern half over what
    /// is behind them, instead of solid.
    #[arg(help_heading = "Hiding", long)]
    see_through_gaps: bool,
    /// Hide these cells (MCNP numbers, comma separated).
    #[arg(help_heading = "Hiding", long, value_delimiter = ',')]
    hide: Vec<i64>,
    /// Hide every cell of these universes.
    #[arg(help_heading = "Hiding", long, value_delimiter = ',')]
    hide_universe: Vec<i64>,
    /// Hide the cells of these materials (filled cells stay, so what
    /// fills them shows).
    #[arg(help_heading = "Hiding", long, value_delimiter = ',')]
    hide_material: Vec<i64>,
    /// Render on the GPU (approximate at edges).
    #[arg(long)]
    gpu: bool,
    /// Show void cells too (hidden by default).
    #[arg(help_heading = "Hiding", long)]
    show_void: bool,
    /// What the colours show.
    #[arg(long, value_enum, default_value = "cell")]
    colour: Colour,
    #[command(flatten)]
    looks: LookArgs,
    #[command(flatten)]
    fields: FieldArgs,
    /// The PNG image to write.
    #[arg(short, long)]
    output: PathBuf,
}

impl RenderArgs {
    pub(crate) fn run(self, open: &Open) -> Result<u8, String> {
        let RenderArgs {
            model,
            eye,
            target,
            from,
            region: r,
            size,
            fov,
            clip,
            wedge,
            keep_box,
            remove_box,
            ortho,
            up,
            hide_rule,
            isolate,
            problems,
            level,
            ghost,
            see_through_gaps,
            hide,
            hide_universe,
            hide_material,
            gpu,
            show_void,
            colour,
            looks,
            fields,
            output,
        } = self;
        let m = load(&model, open)?;
        let looks = looks.looks(&m)?;
        let field = fields.field(&m, &model)?;
        let by: ColourBy = colour.into();
        let paint = Paint::new(&m, &looks, by).with_field(field.as_ref());
        let (w, h) = size
            .split_once(['x', 'X'])
            .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
            .filter(|&(w, h): &(usize, usize)| w > 0 && h > 0)
            .ok_or_else(|| format!("--size: expected WxH, got `{size}`"))?;
        let isolate = isolate
            .as_deref()
            .map(|r| rule(r, "--isolate"))
            .transpose()?;
        let hide_rule: Vec<Query> = hide_rule
            .iter()
            .map(|r| rule(r, "--hide-rule"))
            .collect::<Result<_, _>>()?;
        let ghost: Vec<Query> = ghost
            .iter()
            .map(|r| rule(r, "--ghost"))
            .collect::<Result<_, _>>()?;
        // One index for every rule (none without rules).
        let rules: Vec<&Query> = isolate.iter().chain(&hide_rule).chain(&ghost).collect();
        let x = (!rules.is_empty())
            .then(|| index(&m, &model, &rules))
            .transpose()?;
        // With --isolate, the view is framed on the cells isolated.
        let b = match (&isolate, &x, &r, &eye) {
            (Some(q), Some(x), None, None) => framing(&m, x, q)?,
            _ => region(&m, &r, Scope::Contents)?,
        };
        let hiding = Hiding {
            void: !show_void,
            cells: hide,
            universes: hide_universe,
            materials: hide_material,
            rules: hide_rule,
            isolate,
            problems: problems.into(),
            level,
            ghost,
            see_through: see_through_gaps,
        };
        let vis = match &x {
            Some(x) => hiding.visibility_with(&m, x),
            None => hiding.visibility(&m),
        }
        .map_err(err)?;
        let mut cam = match (eye, target) {
            (Some(e), Some(t)) => {
                let (e, t) = (vec3(&e, "--eye")?, vec3(&t, "--target")?);
                let span = (0..3).map(|i| b.hi[i] - b.lo[i]).fold(0.0, f64::max);
                let d = (0..3).map(|i| (t[i] - e[i]).powi(2)).sum::<f64>().sqrt();
                let dir = [0, 1, 2].map(|i| t[i] - e[i]);
                let up = match &up {
                    Some(u) => vec3(u, "--up")?,
                    None if dir[2].abs() > 0.99 * d => [0.0, 1.0, 0.0],
                    None => [0.0, 0.0, 1.0],
                };
                Camera {
                    eye: e,
                    target: t,
                    up,
                    fov_deg: fov,
                    width: w,
                    height: h,
                    reach: d + 2.0 * span,
                    cut: Cutaway::none(),
                    ortho: None,
                }
            }
            _ => {
                let f = match &from {
                    Some(f) => vec3(f, "--from")?,
                    // Where the cells isolated show most.
                    None if hiding.isolate.is_some() => {
                        let t = Instant::now();
                        let f = bounds::view_from(&m, &vis, &b, DEFAULT_FROM).map_err(err)?;
                        let n = DEFAULT_FROM.iter().map(|x| x * x).sum::<f64>().sqrt();
                        if (0..3).any(|k| (f[k] - DEFAULT_FROM[k] / n).abs() > 1e-9) {
                            eprintln!(
                                "from {:.2},{:.2},{:.2}: the cells isolated show more than \
                                 from {:?} (chosen in {:.2} s; --from to change)",
                                f[0],
                                f[1],
                                f[2],
                                DEFAULT_FROM,
                                t.elapsed().as_secs_f64()
                            );
                        }
                        f
                    }
                    None => DEFAULT_FROM,
                };
                Camera::fit(&b, f.map(|x| -x), w, h)
            }
        };
        for c in &clip {
            let v = numbers(c, 6, "--clip")?;
            let cut = Cutaway::plane(Clip {
                point: [v[0], v[1], v[2]],
                normal: [v[3], v[4], v[5]],
            });
            cam.cut = std::mem::take(&mut cam.cut).union(cut);
        }
        for w in &wedge {
            let v = numbers(w, 8, "--wedge")?;
            let axis = [v[3], v[4], v[5]];
            let zero = if axis[1].abs() + axis[2].abs() == 0.0 {
                [0.0, 1.0, 0.0]
            } else {
                [1.0, 0.0, 0.0]
            };
            let cut = Cutaway::wedge([v[0], v[1], v[2]], axis, zero, v[6], v[7]);
            cam.cut = std::mem::take(&mut cam.cut).union(cut);
        }
        let boxed = |s: &str, what: &str| -> Result<Aabb, String> {
            let v = numbers(s, 6, what)?;
            Ok(Aabb {
                lo: [v[0], v[1], v[2]],
                hi: [v[3], v[4], v[5]],
            })
        };
        if let Some(b) = &keep_box {
            let cut = Cutaway::keep_box(&boxed(b, "--box")?);
            cam.cut = std::mem::take(&mut cam.cut).union(cut);
        }
        for b in &remove_box {
            let cut = Cutaway::remove_box(&boxed(b, "--remove-box")?);
            cam.cut = std::mem::take(&mut cam.cut).union(cut);
        }
        cam.ortho = ortho;
        if gpu {
            let rgb = render_gpu(&paint, &cam, &vis)?;
            let (rgb, h) = with_bar(rgb, w, h, field.as_ref(), &fields);
            write_png(&output, w, h, &rgb)?;
            eprintln!("wrote {}", output.display());
            return Ok(0);
        }
        let t = Instant::now();
        let hits = m.render(&cam, &vis).map_err(err)?;
        let hit = hits.ids.iter().filter(|&&i| i != NO_HIT).count() as f64 / (w * h) as f64;
        eprintln!(
            "rendered {w}×{h} in {:.2} s ({:.0}% of rays hit)",
            t.elapsed().as_secs_f64(),
            100.0 * hit
        );
        if hit < 0.01 {
            eprintln!(
                "only {:.2}% of rays hit: the view frames {} .. {} (give --region, \
                 --isolate or --eye and --target)",
                100.0 * hit,
                fmt_point(b.lo),
                fmt_point(b.hi)
            );
        }
        let probes = Probe::from_hits(&m, &hits);
        let marked = |f: shading::Found| probes.iter().filter(|p| p.what == f).count();
        let (gaps, bad) = (marked(shading::Found::Gap), marked(shading::Found::Problem));
        if gaps + bad > 0 {
            eprintln!(
                "{gaps} px of gaps (grey checker), {bad} px of overlaps or other problems \
                 (magenta stripes){}; `llevant check --region {}` lists them",
                match (hiding.isolate.is_some(), problems) {
                    (true, Problems::Selection) =>
                        " of the cells isolated (--problems all: every one)",
                    _ => "",
                },
                box_arg(&b)
            );
        }
        let mut rgb = shading::render_image(&paint, &probes, &cam).map_err(err)?;
        ghost::overlay(&paint, &hits.ghosts, &cam, &mut rgb);
        let (rgb, h) = with_bar(rgb, w, h, field.as_ref(), &fields);
        write_png(&output, w, h, &rgb)?;
        eprintln!("wrote {}", output.display());
        Ok(0)
    }
}

#[cfg(feature = "gpu")]
fn render_gpu(paint: &Paint, cam: &Camera, vis: &Visibility) -> Result<Vec<u8>, String> {
    let m = paint.model;
    let gpu = llevant_gpu::Gpu::new().map_err(err)?;
    eprintln!(
        "GPU: {} ({:?}, {:?})",
        gpu.info.name, gpu.info.device_type, gpu.info.backend
    );
    let t = Instant::now();
    let gm = llevant_gpu::GpuModel::upload(&gpu, m).map_err(err)?;
    gm.set_visibility(&gpu, m, vis).map_err(err)?;
    eprintln!(
        "exported and uploaded {} MB in {:.2} s",
        gm.bytes >> 20,
        t.elapsed().as_secs_f64()
    );
    let t = Instant::now();
    let f = gm.render(&gpu, cam).map_err(err)?;
    eprintln!(
        "rendered on the GPU in {:.3} s (kernel {:.1} ms)",
        t.elapsed().as_secs_f64(),
        1e3 * f.gpu_seconds
    );
    let mut rgb = shading::render_image(paint, &f.probes(), cam).map_err(err)?;
    ghost::overlay(paint, &f.ghosts, cam, &mut rgb);
    Ok(rgb)
}

#[cfg(not(feature = "gpu"))]
fn render_gpu(_: &Paint, _: &Camera, _: &Visibility) -> Result<Vec<u8>, String> {
    Err("this build has no GPU support (feature `gpu`)".into())
}
