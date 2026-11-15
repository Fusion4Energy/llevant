//! What leaves the viewer: the views as PNG images next to the input (with
//! the legend drawn in, as on screen), and the 3D view as the `llevant
//! render` command that draws it again.

use llevant_render::png::write_png;

use crate::document::Document;
use crate::state::{Colouring, CutState};

/// `<input><suffix>`, or why there is none.
fn beside(doc: &Document, suffix: &str) -> Result<std::path::PathBuf, String> {
    let input = doc.input.as_ref().ok_or("no input file to write next to")?;
    let mut p = input.clone().into_os_string();
    p.push(suffix);
    Ok(p.into())
}

/// The 3D view as composed now, to `<input>.3d.png`: what was written.
pub fn save_3d(doc: &Document) -> Result<String, String> {
    let go = || -> Result<String, String> {
        let path = beside(doc, ".3d.png")?;
        let (w, h) = doc.view3d.size();
        let rgba = doc.view3d.read_image();
        let mut rgb: Vec<u8> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        with_legend(doc, &mut rgb, w as usize, h as usize);
        write_png(&path, w as usize, h as usize, &rgb)?;
        Ok(format!("wrote {}", path.display()))
    };
    go()
}

/// The slice as traced (once complete), to `<input>.slice.png`.
pub fn save_slice(doc: &Document) -> Result<String, String> {
    let go = || -> Result<String, String> {
        let path = beside(doc, ".slice.png")?;
        let (w, h, mut rgb) = doc
            .slice
            .image_rgb()
            .ok_or("the slice is still being traced")?;
        with_legend(doc, &mut rgb, w, h);
        write_png(&path, w, h, &rgb)?;
        Ok(format!("wrote {}", path.display()))
    };
    go()
}

/// The legend shown over the views (if any) drawn into `rgb`, `w × h`
/// pixels of a view: laid out as on screen at the screen's pixels per
/// point, in the theme's colours.
pub fn with_legend(doc: &Document, rgb: &mut [u8], w: usize, h: usize) {
    use llevant_render::legend;
    let ppp = doc.ctx.pixels_per_point().max(0.5);
    let area = legend::Rect::new(0.0, 0.0, w as f32 / ppp, h as f32 / ppp);
    let Some(layout) = crate::overlay::legend(doc, area) else {
        return;
    };
    let v = &doc.ctx.global_style().visuals;
    let rgb_of = |c: egui::Color32| [c.r(), c.g(), c.b()];
    let colours = legend::Colours {
        back: rgb_of(crate::theme::palette(&doc.ctx).base),
        border: rgb_of(v.widgets.noninteractive.bg_stroke.color),
        ink: rgb_of(v.text_color()),
    };
    let mut canvas = legend::Canvas {
        rgb,
        width: w,
        height: h,
    };
    legend::rasterize(&mut canvas, &layout, ppp, colours);
}

/// `llevant render` with the 3D view's camera, cuts, hiding and colours,
/// at `w × h` pixels.
pub fn cli_command(doc: &Document, w: usize, h: usize) -> String {
    let cam = doc.orbit.camera(w, h, None);
    let v = |x: [f64; 3]| format!("{},{},{}", x[0], x[1], x[2]);
    let quote = |s: &str| format!("\"{}\"", s.replace('"', "\\\""));
    // A rule: its label texts may hold what a POSIX shell reads inside
    // double quotes (paths are left alone, for Windows).
    let rule = |s: &str| {
        let mut out = String::from("\"");
        for c in s.chars() {
            if matches!(c, '"' | '\\' | '$' | '`') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out
    };
    let input = doc
        .input
        .as_ref()
        .map_or("model.i".to_string(), |p| p.display().to_string());
    let mut a: Vec<String> = vec!["llevant".into(), "render".into(), quote(&input)];
    let mut push = |k: &str, val: String| {
        a.push(k.to_string());
        a.push(val);
    };
    if let Some(u) = doc.universe {
        push("--universe", u.to_string());
    }
    push("--eye", v(cam.eye));
    push("--target", v(cam.target));
    push("--up", v(cam.up));
    push("--fov", format!("{}", cam.fov_deg));
    if let Some(oh) = cam.ortho {
        push("--ortho", format!("{oh}"));
    }
    push("--size", format!("{w}x{h}"));
    let s = &doc.scene;
    let colour = match s.colour_by {
        Colouring::Cell => "cell",
        Colouring::Material => "material",
        Colouring::Universe => "universe",
        Colouring::Realistic => "realistic",
        Colouring::Density => "density",
    };
    push("--colour", colour.into());
    if let Some(c) = s.clip {
        push("--clip", format!("{},{}", v(c.point), v(c.normal)));
    }
    for (_, c) in s.cuts_on() {
        match *c {
            CutState::Plane { point, normal } => {
                push("--clip", format!("{},{}", v(point), v(normal)));
            }
            CutState::Wedge {
                point,
                axis,
                from,
                to,
                ..
            } => push("--wedge", format!("{},{},{from},{to}", v(point), v(axis))),
            CutState::Box { lo, hi, keep } => {
                let k = if keep { "--box" } else { "--remove-box" };
                push(k, format!("{},{}", v(lo), v(hi)));
            }
        }
    }
    let h_ = &s.hiding;
    let list = |v: &[i64]| {
        v.iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    if !h_.void {
        a.push("--show-void".into());
    }
    if h_.see_through {
        a.push("--see-through-gaps".into());
    }
    let mut push = |k: &str, val: String| {
        a.push(k.to_string());
        a.push(val);
    };
    if !h_.cells.is_empty() {
        push("--hide", list(&h_.cells));
    }
    if !h_.universes.is_empty() {
        push("--hide-universe", list(&h_.universes));
    }
    if !h_.materials.is_empty() {
        push("--hide-material", list(&h_.materials));
    }
    for r in &h_.rules {
        push("--hide-rule", rule(&r.to_string()));
    }
    if let Some(r) = &h_.isolate {
        push("--isolate", rule(&r.to_string()));
        if h_.problems == llevant_scene::hiding::ProblemScope::All {
            push("--problems", "all".into());
        }
    }
    if let (Some(f), Some(shown)) = (&s.field, &doc.field) {
        let src = &f.source;
        push("--field", quote(&src.path.display().to_string()));
        if let Some(t) = src.tally {
            push("--tally", t.to_string());
        }
        if src.energy != "total" {
            push("--energy", src.energy.clone());
        }
        if src.time != "total" {
            push("--time", src.time.clone());
        }
        for b in src.bins.split([' ', ',']).filter(|b| !b.is_empty()) {
            push("--bin", b.to_string());
        }
        if let Some(i) = &src.input {
            push("--run-input", quote(&i.display().to_string()));
        }
        if src.error {
            a.push("--error".into());
        }
        let sc = shown.look.scale;
        a.push(if sc.log { "--log" } else { "--linear" }.into());
        a.push("--scale".into());
        a.push(format!("{},{}", sc.lo, sc.hi));
        if f.opacity < 1.0 {
            a.push("--opacity".into());
            a.push(format!("{}", f.opacity));
        }
        if f.geometry_under {
            a.push("--cell-colours-under".into());
        }
        let st = &f.style;
        if st.ramp() != llevant_render::scale::Ramp::default() {
            a.push("--ramp".into());
            a.push(st.ramp().name().into());
        }
        if st.bands >= 2 {
            a.push("--bands".into());
            a.push(format!("{}", st.bands.min(64)));
        }
        let l = &st.legend;
        let ticks = match l.ticks() {
            llevant_render::legend::Ticks::Auto => None,
            llevant_render::legend::Ticks::Many => Some("many".to_string()),
            llevant_render::legend::Ticks::Count(n) => Some(n.to_string()),
            // Edges without bands are the automatic ticks.
            llevant_render::legend::Ticks::Edges => (st.bands >= 2).then(|| "edges".into()),
        };
        if let Some(t) = ticks {
            a.push("--ticks".into());
            a.push(t);
        }
        match l.numbers {
            crate::state::NumberFormat::Short => {}
            crate::state::NumberFormat::Powers => a.extend(["--numbers".into(), "powers".into()]),
            crate::state::NumberFormat::Decimal => a.extend(["--numbers".into(), "decimal".into()]),
        }
        if l.digits > 0 {
            a.push("--digits".into());
            a.push(l.digits.min(6).to_string());
        }
        if let Some(t) = &l.title {
            a.push("--legend-title".into());
            a.push(quote(t));
        }
        let bound = |b: Option<f64>| b.map_or(String::new(), |x| format!("{x}"));
        if f.threshold.iter().any(Option::is_some) {
            a.push("--threshold".into());
            a.push(format!(
                "{},{}",
                bound(f.threshold[0]),
                bound(f.threshold[1])
            ));
        }
        let hex = |[r, g, b]: [u8; 3]| format!("'#{r:02x}{g:02x}{b:02x}'");
        if let Some(c) = f.style.below {
            a.push("--below-colour".into());
            a.push(hex(c));
        }
        if let Some(c) = f.style.above {
            a.push("--above-colour".into());
            a.push(hex(c));
        }
    }
    a.push("-o".into());
    a.push("view.png".into());
    a.join(" ")
}
