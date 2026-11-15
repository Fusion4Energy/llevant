//! Results over the geometry: fields, MCTAL tallies and FMESH layouts.

use super::*;

/// A field read from a CSV file (per material) and drawn over both views:
/// its value in the inspector, its scale undone, and a view file naming it
/// draws it again when opened.
#[test]
fn fields_over_the_views() {
    let _one = serial();
    if !adapter_available() {
        eprintln!("skipped: no GPU adapter");
        return;
    }
    let dir = std::env::temp_dir().join(format!("llevant-viewer-field-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("heat.csv");
    std::fs::write(&csv, "material,heat [W/cm3]\n1,2\n2,300\n").unwrap();
    let build = |state: ViewState| {
        Harness::builder()
            .with_size(egui::vec2(1200.0, 700.0))
            .wgpu_setup(llevant_viewer::app::wgpu_options().wgpu_setup)
            .build_eframe(move |cc| ViewerApp::new(cc, model("universes.i"), state, None).unwrap())
    };
    let mut h = build(ViewState::new());
    step_until(&mut h, "the model to load", |a| {
        a.document().is_some() || a.error().is_some()
    });
    h.run_steps(2);
    // The panel, empty, then the field read in the background.
    panel(&mut h, Panel::Fields);
    h.run_steps(2);
    h.get_by_label(&icons::with(icons::FOLDER_OPEN, "Open field…"));
    let source = llevant_viewer::state::FieldSource::new(csv.clone());
    h.state_mut()
        .document_mut()
        .unwrap()
        .execute(llevant_viewer::command::Command::LoadField(source.clone()));
    step_until(&mut h, "the field", |a| {
        a.document().unwrap().field.is_some()
    });
    h.run_steps(3);
    let doc = h.state().document().unwrap();
    let f = doc.field.clone().unwrap();
    assert_eq!(
        (f.field.name.as_str(), f.field.unit.as_str()),
        ("heat", "W/cm3")
    );
    assert!(f.look.scale.log, "2 to 300 spans two decades");
    assert_eq!(doc.scene.field.as_ref().unwrap().source, source);
    // The point the inspector shows has the field's value of its material.
    let m = doc.model.clone();
    let cam = doc.orbit.camera(64, 48, None);
    let mut found = None;
    'rays: for j in 0..48 {
        for i in 0..64 {
            if let Some(p) = pick_ray(&m, &doc.vis, &cam, i, j) {
                if let Some(c) = p.cell() {
                    if m.cell(c).material == 2 {
                        found = Some(p);
                        break 'rays;
                    }
                }
            }
        }
    }
    let p = found.expect("a cell of material 2 in view");
    assert_eq!(f.field.value(&m, &p.label, p.point), Some(300.0));
    h.state_mut()
        .document_mut()
        .unwrap()
        .execute(llevant_viewer::command::Command::Focus(Some(p)));
    h.run_steps(2);
    h.get_by_label_contains("300 W/cm3");

    // A scale set by hand, undone.
    let scale = llevant_viewer::state::ScaleState {
        lo: 1.0,
        hi: 10.0,
        log: false,
    };
    h.state_mut()
        .document_mut()
        .unwrap()
        .execute(llevant_viewer::command::Edit::FieldScale(Some(scale)).into());
    settle(&mut h);
    let doc = h.state().document().unwrap();
    assert_eq!(doc.field.as_ref().unwrap().look.scale.hi, 10.0);
    shortcut(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
    settle(&mut h);
    let doc = h.state().document().unwrap();
    assert!(doc.scene.field.as_ref().unwrap().scale.is_none());
    assert!(doc.field.as_ref().unwrap().look.scale.log);
    let png = std::env::var_os("LLEVANT_VIEWER_TEST_PNG").map(PathBuf::from);
    if let Some(p) = &png {
        step_until(&mut h, "tracing", |a| !a.document().unwrap().view3d.busy());
        h.run_steps(3);
        save(&mut h, &p.with_extension("field.png"));
    }

    // Its look, from Style…: Turbo in 5 bands, the legend upright.
    panel(&mut h, Panel::Fields);
    h.get_by_label(&icons::with(icons::COLOUR, "Style…"))
        .click();
    h.run_steps(2);
    h.get_by_label_contains("turbo").click();
    h.run_steps(2);
    h.get_by_label("discrete").click();
    h.run_steps(2);
    h.get_by_label("vertical").click();
    h.run_steps(2);
    settle(&mut h);
    let doc = h.state().document().unwrap();
    let style = doc.scene.field.as_ref().unwrap().style.clone();
    assert_eq!(style.ramp, "turbo");
    assert_eq!(style.bands, 10);
    assert!(style.legend.vertical);
    let scale = doc.field.as_ref().unwrap().look.scale;
    assert_eq!(scale.ramp, llevant_render::scale::Ramp::Turbo);
    assert_eq!(scale.bands, 10);
    // Fitted ends rounded out: 2 to 300 shows 1 to 1000.
    assert_eq!((scale.lo, scale.hi), (1.0, 1000.0));

    // A threshold and a colour above the scale reach the look; saved
    // images carry the legend, as on screen (an opaque box in the corner).
    let mut style = style;
    // Ticks at the band edges: 3 bands over 1 to 1000, one a decade.
    style.bands = 3;
    style.legend.band_ticks = true;
    h.state_mut()
        .document_mut()
        .unwrap()
        .execute(llevant_viewer::command::Edit::FieldStyle(Box::new(style.clone())).into());
    settle(&mut h);
    {
        let doc = h.state().document().unwrap();
        let area = llevant_render::legend::Rect::new(0.0, 0.0, 800.0, 600.0);
        let l = llevant_viewer::overlay::legend(doc, area).unwrap();
        let words: Vec<&str> = l.texts.iter().skip(1).map(|t| t.text.as_str()).collect();
        assert_eq!(words[..4], ["1", "10", "100", "1000"]);
        let scale = doc.field.as_ref().unwrap().look.scale;
        assert_eq!(llevant_render::legend::edges_on_decades(&scale), Some(true));
        // The view as a `llevant render` command keeps the style.
        let cmd = llevant_viewer::export::cli_command(doc, 100, 80);
        for want in [
            " --ramp turbo",
            " --bands 3",
            " --ticks edges",
            " --scale 1,1000",
        ] {
            assert!(cmd.contains(want), "{want}: {cmd}");
        }
        if let Some(p) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
            std::fs::write(PathBuf::from(p).with_extension("cmd.txt"), &cmd).unwrap();
        }
    }
    style.above = Some([255, 0, 0]);
    h.state_mut()
        .document_mut()
        .unwrap()
        .execute(llevant_viewer::command::Edit::FieldStyle(Box::new(style)).into());
    h.state_mut()
        .document_mut()
        .unwrap()
        .execute(llevant_viewer::command::Edit::FieldThreshold([Some(10.0), None]).into());
    settle(&mut h);
    h.run_steps(3);
    step_until(&mut h, "the slice", |a| {
        let s = &a.document().unwrap().slice;
        s.progress() >= 1.0 && s.image_rgb().is_some()
    });
    let doc = h.state().document().unwrap();
    let look = doc.field.as_ref().unwrap().look;
    assert_eq!(
        (look.threshold, look.above),
        ([Some(10.0), None], Some([255, 0, 0]))
    );
    // Material 1 (2 W/cm3) is below the threshold: no value.
    assert_eq!(look.over([9; 3], Some(2.0)), look.no_value());
    let (w, hgt, plain) = doc.slice.image_rgb().unwrap();
    let mut rgb = plain.clone();
    llevant_viewer::export::with_legend(doc, &mut rgb, w, hgt);
    let changed = rgb
        .chunks(3)
        .zip(plain.chunks(3))
        .filter(|(a, b)| a != b)
        .count();
    assert!(changed > 1000, "{changed} pixels of legend");
    let ppp = h.ctx.pixels_per_point();
    let area = llevant_render::legend::Rect::new(0.0, 0.0, w as f32 / ppp, hgt as f32 / ppp);
    let l = llevant_viewer::overlay::legend(doc, area).unwrap();
    // Inside the box, under the title's line: its opaque background.
    let (x, y) = ((l.back.x0 + 3.0) * ppp, (l.back.y1 - 3.0) * ppp);
    let k = 3 * (y as usize * w + x as usize);
    let base = llevant_viewer::theme::palette(&h.ctx).base;
    assert_eq!(&rgb[k..k + 3], &[base.r(), base.g(), base.b()]);
    // The arrow above the scale, in its colour.
    assert_eq!(l.arrows.len(), 1);
    // The 3D outlines in a colour of their own (white here, over the image
    // as strongly as black darkens it).
    let light = |h: &Harness<'_, ViewerApp>| {
        let img = h.state().document().unwrap().view3d.read_image();
        img.chunks(4)
            .filter(|p| p[0].min(p[1]).min(p[2]) > 150)
            .count()
    };
    step_until(&mut h, "tracing", |a| !a.document().unwrap().view3d.busy());
    h.run_steps(2);
    let before = light(&h);
    let mut fx = h.state().document().unwrap().scene.effects;
    fx.outline_colour = Some([255, 255, 255]);
    h.state_mut()
        .document_mut()
        .unwrap()
        .execute(llevant_viewer::command::Edit::Effects(fx).into());
    settle(&mut h);
    step_until(&mut h, "tracing", |a| !a.document().unwrap().view3d.busy());
    h.run_steps(3);
    let after = light(&h);
    assert!(after > before + 100, "light pixels {before} -> {after}");
    if let Some(p) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        let p = PathBuf::from(p).with_extension("export.png");
        llevant_render::png::write_png(&p, w, hgt, &rgb).unwrap();
    }

    // The field along a ruler, under the views in Results.
    h.get_by_label(&icons::with(icons::RESULTS, "Results"))
        .click();
    h.run_steps(2);
    let (m, region) = {
        let d = h.state().document().unwrap();
        (d.model.clone(), d.region)
    };
    let c = region.center();
    let r = llevant_viewer::ruler::measure(
        &m,
        [region.lo[0], c[1], c[2]],
        [region.hi[0], c[1], c[2]],
        llevant_viewer::ruler::RulerMode::TwoPoints,
        None,
    );
    h.state_mut()
        .document_mut()
        .unwrap()
        .execute(llevant_viewer::command::Command::SetRuler(r));
    h.run_steps(3);
    h.get_by_label_contains("Along the ruler");
    let samples = {
        let d = h.state().document().unwrap();
        llevant_viewer::panels::profile::samples(d, d.ruler.as_ref().unwrap())
    };
    assert!(
        samples.iter().any(|(_, v)| v.is_some()),
        "values along the line"
    );
    if let Some(p) = &png {
        step_until(&mut h, "tracing", |a| !a.document().unwrap().view3d.busy());
        h.run_steps(3);
        save(&mut h, &p.with_extension("results.png"));
    }

    // The view file names the field; opened again, it is drawn again.
    let state = h
        .state()
        .document()
        .unwrap()
        .view_state(Default::default(), 33.0);
    let json = serde_json::to_string(&state).unwrap();
    assert!(json.contains("heat.csv"), "{json}");
    let again: ViewState = serde_json::from_str(&json).unwrap();
    drop(h);
    let mut h = build(again);
    step_until(&mut h, "the model and its field", |a| {
        a.document().is_some_and(|d| d.field.is_some())
    });
    // No field: the views go back to the cells' colours.
    h.state_mut()
        .document_mut()
        .unwrap()
        .execute(llevant_viewer::command::Edit::Field(None).into());
    settle(&mut h);
    assert!(h.state().document().unwrap().field.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// An MCTAL field (aci_tallies' probe: a ray through a lattice): read per
/// instance, the panel showing its tally; a tally needing a multiplier bin
/// stays the scene's, the panel saying why and offering its bins, and is
/// read once one is given; the file's summary from its index. With
/// `LLEVANT_VIEWER_TEST_PNG=<file>`, the window to `<file>.mctal.png`.
#[test]
fn mctal_fields() {
    let _one = serial();
    if !adapter_available() {
        eprintln!("skipped: no GPU adapter");
        return;
    }
    let fixtures =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tallies/mctal");
    let input = fixtures.join("probe.mcnp");
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 700.0))
        .wgpu_setup(llevant_viewer::app::wgpu_options().wgpu_setup)
        .build_eframe(move |cc| ViewerApp::new(cc, input.clone(), ViewState::new(), None).unwrap());
    step_until(&mut h, "the model to load", |a| {
        a.document().is_some() || a.error().is_some()
    });
    h.run_steps(2);
    panel(&mut h, Panel::Fields);
    h.run_steps(2);
    let mut source = llevant_viewer::state::FieldSource::new(fixtures.join("probe.m"));
    source.tally = Some(44);
    let load = |h: &mut Harness<'_, ViewerApp>, s: &llevant_viewer::state::FieldSource| {
        h.state_mut()
            .document_mut()
            .unwrap()
            .execute(llevant_viewer::command::Command::LoadField(s.clone()));
    };
    load(&mut h, &source);
    step_until(&mut h, "the field", |a| {
        let d = a.document().unwrap();
        d.field.is_some() || (d.field_loading().is_none() && !d.messages.is_empty())
    });
    h.run_steps(3);
    let doc = h.state().document().unwrap();
    let f = doc
        .field
        .clone()
        .unwrap_or_else(|| panic!("{:?}", last_said(doc)));
    assert!(matches!(
        f.field.values,
        llevant_scene::field::Values::Instances(_)
    ));
    assert_eq!(f.field.unit, "1/cm²");
    assert_eq!(
        doc.tally_format(&source),
        Some(llevant_scene::field::Format::Mctal)
    );
    // The tally menu; F44 has no other bins to choose.
    assert!(h.query_all_by_label("tally").count() >= 1);
    assert_eq!(h.query_all_by_label("multiplier").count(), 0);
    let summary = doc.tally_summary(&source).unwrap();
    assert_eq!(summary.default, Some(4));
    // Its cell bins with their values; a click selects a bin's cells, and
    // the tally's cells are isolated at once.
    assert_eq!(doc.field_bins(&source).map(|b| b.len()), Some(6));
    let c8 = doc.model.cell_index(8).unwrap();
    // (The panel is short: clicked through accesskit, out of view.)
    h.get_by_label("(8<7[1 0 0]<3)").click_accesskit();
    h.run_steps(2);
    let doc = h.state().document().unwrap();
    assert!(doc.selection.cells.contains(c8));
    assert!(!doc
        .selection
        .cells
        .contains(doc.model.cell_index(9).unwrap()));
    h.get_all_by_label(icons::ISOLATE)
        .next()
        .unwrap()
        .click_accesskit();
    h.run_steps(2);
    let doc = h.state().document().unwrap();
    assert_eq!(
        doc.scene.hiding.isolate,
        Some(llevant_scene::query::Query::Cells(vec![8, 9]))
    );
    step_until(&mut h, "tracing", |a| {
        let d = a.document().unwrap();
        !d.view3d.busy() && d.slice.progress() >= 1.0
    });
    h.run_steps(3);
    if let Some(path) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        save(&mut h, &PathBuf::from(path).with_extension("mctal.png"));
    }
    // Three multiplier bins and no Total: the scene names it all the same,
    // the panel saying why and offering the bins, until one is chosen.
    source.tally = Some(54);
    load(&mut h, &source);
    step_until(&mut h, "the failure", |a| {
        a.document().unwrap().field_loading().is_none()
    });
    h.run_steps(2);
    settle(&mut h);
    let doc = h.state().document().unwrap();
    let msg = last_said(doc);
    assert!(msg.contains("m bin"), "{msg}");
    assert_eq!(doc.scene.field.as_ref().unwrap().source, source);
    assert!(doc.field_error(&source).unwrap().contains("m bin"));
    assert!(doc.field.is_none());
    assert!(h.query_all_by_label("multiplier").count() >= 1);
    source.bins = "m=1".into();
    load(&mut h, &source);
    step_until(&mut h, "the field", |a| {
        let d = a.document().unwrap();
        d.field_loading().is_none() && d.scene.field.as_ref().is_some_and(|f| f.source == source)
    });
    settle(&mut h);
    let doc = h.state().document().unwrap();
    assert!(matches!(
        doc.field.as_ref().unwrap().field.values,
        llevant_scene::field::Values::Cells(_)
    ));
}

/// FMESH in Tally: a new box called up from the palette, dragged in the
/// slice (rounded, as deep as the model), its bins typed in the FMESH panel
/// as on a card, undone; a click selects it, Delete removes it; the view
/// file keeps it.
#[test]
fn fmesh_boxes_from_the_slice() {
    let _one = serial();
    if !adapter_available() {
        eprintln!("skipped: no GPU adapter");
        return;
    }
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .wgpu_setup(llevant_viewer::app::wgpu_options().wgpu_setup)
        .build_eframe(|cc| {
            ViewerApp::new(cc, model("universes.i"), ViewState::new(), None).unwrap()
        });
    step_until(&mut h, "the model to load", |a| {
        a.document().is_some() || a.error().is_some()
    });
    step_until(&mut h, "the input's cards", |a| {
        a.document().is_some_and(|d| d.input_meshes.is_some())
    });

    shortcut(&mut h, egui::Key::K, egui::Modifiers::COMMAND);
    type_into(&mut h, "palette text", "fmesh new box");
    h.run_steps(2);
    shortcut(&mut h, egui::Key::Enter, egui::Modifiers::NONE);
    h.run_steps(2);
    let doc = h.state().document().unwrap();
    assert_eq!(doc.editing.drawing_mesh, Some(false), "a box, armed");
    h.get_by_label_contains("Drag in the slice: a new box");

    // A box dragged in the slice.
    let rect = h.get_by_label("slice view").rect();
    let (from, to) = (
        rect.center() + egui::vec2(-80.0, 50.0),
        rect.center() + egui::vec2(60.0, -40.0),
    );
    drag_between(&mut h, from, to);
    let doc = h.state().document().unwrap();
    assert_eq!(doc.scene.meshes.len(), 1, "{:?}", last_said(doc));
    assert_eq!(doc.editing.mesh_selected, Some(0));
    assert_eq!(doc.editing.drawing_mesh, None, "one mesh per + Box");
    let m = doc.scene.meshes[0].clone();
    assert_eq!(m.id, 14);
    let plane = doc.slice.plane();
    let f = plane.frame;
    let (a, _) = f.coords(plane.from_screen(rect, from));
    let (b, _) = f.coords(plane.from_screen(rect, to));
    let n = llevant_viewer::cuts::along_axis(f.n).unwrap();
    let (u, v) = (
        llevant_viewer::cuts::along_axis(f.u).unwrap(),
        llevant_viewer::cuts::along_axis(f.v).unwrap(),
    );
    let ax = m.axes();
    let step = 10.0 * plane.cm_per_point(rect);
    for (k, lo, hi) in [
        (u, a[0].min(b[0]), a[0].max(b[0])),
        (v, a[1].min(b[1]), a[1].max(b[1])),
    ] {
        assert!((ax[k][0] - lo).abs() <= step, "{k}: {:?} {lo}", ax[k]);
        assert!(
            (ax[k].last().unwrap() - hi).abs() <= step,
            "{k}: {:?} {hi}",
            ax[k]
        );
    }
    assert!(ax[n][0] <= doc.region.lo[n] && *ax[n].last().unwrap() >= doc.region.hi[n]);
    let text = m.text().unwrap();
    assert!(text.starts_with("FMESH14:n GEOM=XYZ ORIGIN"), "{text}");
    // The inspector shows it: its checks and the cells it covers.
    h.run_steps(2);
    h.get_by_label("FMESH 14:n");
    h.get_by_label("COVERS");
    let covers = llevant_viewer::panels::inspector::covered(h.state().document().unwrap(), &m);
    assert!(!covers.is_empty(), "the box covers cells");
    if let Some(path) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        h.run_steps(3);
        save(&mut h, &PathBuf::from(path).with_extension("tally.png"));
    }

    // Bins typed as on the card: 3 coarse bins of 2 fine ones along x.
    let lo = ax[u][0];
    let hi = *ax[u].last().unwrap();
    let mid = 0.5 * (lo + hi);
    let coarse = format!(
        "{} {}",
        llevant_viewer::mesh::number(mid),
        llevant_viewer::mesh::number(hi)
    );
    let field = ["mesh i", "mesh j", "mesh k"][u];
    let ints = ["ints i", "ints j", "ints k"][u];
    for (name, text) in [(field, coarse.as_str()), (ints, "2 3")] {
        // (In the Tally workspace's panel: scrolled to, animated.)
        h.get_by_label(name).scroll_to_me();
        h.run_steps(30);
        h.get_by_label(name).click();
        h.run_steps(2);
        shortcut(&mut h, egui::Key::A, egui::Modifiers::COMMAND);
        h.get_by_label(name).type_text(text);
        h.run_steps(2);
        shortcut(&mut h, egui::Key::Enter, egui::Modifiers::NONE);
    }
    let m2 = h.state().document().unwrap().scene.meshes[0].clone();
    assert_eq!(
        m2.axes()[u],
        &aci_tallies_geom::mcnp_input::fine_bounds(lo, &[mid, hi], &[2, 3])
    );
    // Its cards read back as the same mesh.
    let input = format!(
        "t\n1 0 -1 imp:n=1\n2 0 1 imp:n=0\n\n1 so 1\n\nmode n\n{}",
        m2.text().unwrap()
    );
    let card = aci_tallies_geom::mcnp_input::read_fmeshes(&input)
        .pop()
        .unwrap()
        .unwrap();
    assert_eq!(card.geometry().unwrap(), m2.geometry().unwrap());
    // Undone: back as it was dragged.
    h.get_by_label(&icons::with(icons::UNDO, "Undo")).click();
    h.run_steps(2);
    assert_eq!(h.state().document().unwrap().scene.meshes[0], m);
    h.get_by_label(&icons::with(icons::REDO, "Redo")).click();
    h.run_steps(2);

    // Clicks select it and nothing; Delete removes it.
    let outside = rect.min + egui::vec2(15.0, 15.0);
    click(&mut h, outside);
    assert_eq!(h.state().document().unwrap().editing.mesh_selected, None);
    let inside = from + (to - from) * 0.5;
    click(&mut h, inside);
    assert_eq!(h.state().document().unwrap().editing.mesh_selected, Some(0));
    if let Some(path) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        h.event(egui::Event::PointerMoved(inside));
        h.run_steps(3);
        save(&mut h, &PathBuf::from(path).with_extension("fmesh.png"));
    }
    // The view file keeps it.
    let state = h.state().view_state();
    let json = serde_json::to_string(&state).unwrap();
    let back: ViewState = serde_json::from_str(&json).unwrap();
    assert_eq!(
        back.scene.meshes,
        h.state().document().unwrap().scene.meshes
    );

    h.event(egui::Event::PointerMoved(inside));
    shortcut(&mut h, egui::Key::Delete, egui::Modifiers::NONE);
    assert!(h.state().document().unwrap().scene.meshes.is_empty());
}

/// On a radial slice a box is placed by a TR along the plane; a cylinder is
/// dragged from its centre about the plane's normal; both read back from
/// their cards where they are drawn.
#[test]
fn fmesh_trs_and_cylinders() {
    let _one = serial();
    if !adapter_available() {
        eprintln!("skipped: no GPU adapter");
        return;
    }
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .wgpu_setup(llevant_viewer::app::wgpu_options().wgpu_setup)
        .build_eframe(|cc| {
            ViewerApp::new(cc, model("universes.i"), ViewState::new(), None).unwrap()
        });
    step_until(&mut h, "the model to load", |a| {
        a.document().is_some() || a.error().is_some()
    });
    // Tally's FMESH panel, + Box: the next drag in the slice draws one.
    panel(&mut h, Panel::Mesh);
    h.get_by_label("+ Box").click();
    h.run_steps(2);
    let doc = h.state_mut().document_mut().unwrap();
    doc.execute(llevant_viewer::command::Command::SliceRadial(30.0));
    h.run_steps(2);
    let rect = h.get_by_label("slice view").rect();
    let (from, to) = (
        rect.center() + egui::vec2(-60.0, 40.0),
        rect.center() + egui::vec2(50.0, -30.0),
    );
    drag_between(&mut h, from, to);
    let doc = h.state().document().unwrap();
    let m = doc.scene.meshes.first().expect("a mesh").clone();
    let t = m.tr.expect("placed by a TR");
    // Its local x is the plane's u: 30° from x.
    let plane = doc.slice.plane();
    assert!((t.rotation[0][0] - plane.frame.u[0]).abs() < 1e-12);
    assert!((30f64.to_radians().cos() - plane.frame.u[0]).abs() < 1e-9);
    let middle = plane.from_screen(rect, from + (to - from) * 0.5);
    let text = m.text().unwrap();
    assert!(
        text.contains(&format!("TR={}", t.number)) && text.contains(&format!("TR{} ", t.number))
    );
    let read = |text: &str| {
        let input = format!("t\n1 0 -1 imp:n=1\n2 0 1 imp:n=0\n\n1 so 1\n\nmode n\n{text}");
        aci_tallies_geom::mcnp_input::read_fmeshes(&input)
            .pop()
            .unwrap()
            .unwrap()
    };
    let g = read(&text).geometry().unwrap();
    assert!(g.locate(middle).is_some());
    assert!(g
        .locate(plane.from_screen(rect, from + (from - to) * 0.2))
        .is_none());

    // A cylinder about the slice's normal.
    let doc = h.state_mut().document_mut().unwrap();
    doc.execute(llevant_viewer::command::Command::DrawMesh(Some(true)));
    doc.execute(llevant_viewer::command::Command::SliceAt(
        llevant_viewer::state::AxisName::Z,
        0.0,
    ));
    h.run_steps(2);
    let centre = rect.center() + egui::vec2(0.0, 120.0);
    drag_between(&mut h, centre, centre + egui::vec2(40.0, 0.0));
    let doc = h.state().document().unwrap();
    assert_eq!(doc.scene.meshes.len(), 2);
    let c = doc.scene.meshes[1].clone();
    assert_ne!(c.id, m.id);
    assert!(c.is_cylinder() && c.tr.is_none());
    let llevant_viewer::mesh::MeshShape::Cyl { axs, .. } = c.shape else {
        unreachable!()
    };
    assert_eq!(axs, [0.0, 0.0, 1.0]);
    let plane = doc.slice.plane();
    let g = read(&c.text().unwrap()).geometry().unwrap();
    assert!(g
        .locate(plane.from_screen(rect, centre + egui::vec2(20.0, 0.0)))
        .is_some());
    assert!(g
        .locate(plane.from_screen(rect, centre + egui::vec2(0.0, 60.0)))
        .is_none());
    // Both, each with its own numbers, in one go.
    let both =
        llevant_viewer::mesh::cards_text(&doc.scene.meshes, doc.input_meshes.as_deref()).unwrap();
    assert_eq!(read(&both).id, c.id, "both read");
    if let Some(path) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        h.event(egui::Event::PointerMoved(centre));
        h.run_steps(3);
        save(&mut h, &PathBuf::from(path).with_extension("fmesh-cyl.png"));
    }
}
