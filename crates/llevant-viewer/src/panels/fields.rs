//! The field drawn over the cells (`llevant_render::field`): numbers per
//! cell, per cell instance or per mesh voxel, read from a CSV file or an
//! MCNP tally file (meshtal, MCTAL or D1SUNED cuvmsh). Its source, colour scale and opacity are the scene's
//! (undoable, saved with the view); the data is the document's.

use llevant_render::scale::{tick_label, value_label};
use llevant_scene::field::{AxisSummary, Bin, FileSummary, TallyChoice, TallySummary, Values};
use llevant_scene::query::Query;

use crate::command::{Command, Edit};
use crate::document::Document;
use crate::selection::SelectMode;
use crate::state::{FieldSource, FieldState, ScaleState};
use crate::{icons, theme};

/// The fields panel (its state is the scene's and the document's).
#[derive(Default)]
pub struct FieldsPanel;

impl FieldsPanel {
    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
        ui.horizontal(|ui| {
            if ui
                .button(icons::with(icons::FOLDER_OPEN, "Open field…"))
                .on_hover_text(
                    "Values per cell (CSV) or an MCNP meshtal or MCTAL file; or drop one on the window",
                )
                .clicked()
            {
                out.push(Command::OpenFieldDialog);
            }
            if let Some(s) = doc.field_loading() {
                crate::logo::small_spinner(ui);
                ui.label(egui::RichText::new(format!("reading {}", s.describe())).weak());
            }
        });
        let Some(state) = &doc.scene.field else {
            crate::app::frame::hint(
                ui,
                "Colour the cells by numbers: a CSV file whose first column is cell, \
                 material, instance or path (12 < 100[1 0 0] < 5) and whose second holds \
                 the values; an MCNP meshtal file (FMESH tallies, on their mesh); or an \
                 MCTAL file (cell tallies, per cell or per instance of a repeated \
                 structure).",
            );
            return;
        };
        let data = doc.field_data(&state.source).cloned();
        ui.add_space(4.0);
        theme::section(ui, icons::FIELD, "Field", |ui| {
            ui.horizontal_wrapped(|ui| {
                let mut shown = state.shown;
                if ui
                    .checkbox(&mut shown, "")
                    .on_hover_text("Show it")
                    .changed()
                {
                    out.push(edit(state, |s| s.shown = shown));
                }
                ui.label(theme::data(
                    data.as_ref()
                        .map_or_else(|| state.source.describe(), |d| d.field.title()),
                ));
                if ui
                    .small_button(icons::REMOVE)
                    .on_hover_text("No field")
                    .clicked()
                {
                    out.push(Edit::Field(None).into());
                }
            });
            ui.label(egui::RichText::new(state.source.path.display().to_string()).weak())
                .on_hover_text("The file read");
            if let Some(d) = &data {
                let finite: Vec<f32> = d.field.numbers().filter(|x| x.is_finite()).collect();
                let (lo, hi) = finite
                    .iter()
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &x| {
                        (a.min(x), b.max(x))
                    });
                let what = match &d.field.values {
                    Values::Cells(_) => "per cell",
                    Values::Instances(_) => "per cell instance",
                    Values::Mesh(m) if m.by_cell.is_some() => "per voxel and cell",
                    Values::Mesh(_) => "per mesh voxel",
                };
                ui.label(
                    egui::RichText::new(if finite.is_empty() {
                        format!("{what}, no values")
                    } else {
                        format!(
                            "{} values {what}, {} to {}",
                            finite.len(),
                            value_label(lo as f64),
                            value_label(hi as f64)
                        )
                    })
                    .weak(),
                );
            }
            match doc.tally_summary(&state.source) {
                Some(summary) => tally(ui, doc, state, &summary, out),
                None => {
                    if let Some(e) = doc.field_error(&state.source) {
                        ui.label(egui::RichText::new(e).color(theme::palette(ui.ctx()).error));
                    }
                }
            }
        });
        // How it looks: a preview, and every setting on demand.
        ui.horizontal(|ui| {
            if let Some(f) = &doc.field {
                ramp_preview(ui, &f.look.scale, 140.0);
            }
            theme::settings_button(ui, icons::with(icons::COLOUR, "Style…"), |ui| {
                ui.set_min_width(300.0);
                style(ui, doc, state, out);
            })
            .on_hover_text("Colours, bands, scale, opacity and the legend");
        });
        opened(ui, doc, state, out);
    }
}

/// The fields read in this session, to switch between (each drawn again at
/// once, its scale fitted, the look kept).
fn opened(ui: &mut egui::Ui, doc: &Document, state: &FieldState, out: &mut Vec<Command>) {
    let read = doc.fields_read();
    if read.len() < 2 {
        return;
    }
    theme::section(ui, icons::FOLDER_OPEN, "Opened", |ui| {
        for source in read.iter().rev() {
            let on = *source == state.source;
            let title = doc
                .field_data(source)
                .map_or_else(|| source.describe(), |d| d.field.title());
            if ui
                .selectable_label(on, theme::data(title))
                .on_hover_text(source.path.display().to_string())
                .clicked()
                && !on
            {
                out.push(edit(state, |s| {
                    s.source = source.clone();
                    s.scale = None;
                    s.shown = true;
                }));
            }
        }
    });
}

/// A strip of the scale's colours, `width` wide.
fn ramp_preview(ui: &mut egui::Ui, s: &llevant_render::scale::Scale, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 12.0), egui::Sense::hover());
    let n = 48;
    for k in 0..n {
        let [r, g, b] = s.colour(s.value((k as f64 + 0.5) / n as f64));
        let x0 = rect.left() + rect.width() * k as f32 / n as f32;
        let x1 = rect.left() + rect.width() * (k + 1) as f32 / n as f32;
        ui.painter().rect_filled(
            egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1, rect.bottom())),
            0.0,
            egui::Color32::from_rgb(r, g, b),
        );
    }
}

/// Every setting of the field's look: colour map, bands, scale, opacity,
/// legend.
fn style(ui: &mut egui::Ui, doc: &Document, state: &FieldState, out: &mut Vec<Command>) {
    use crate::state::{Corner, LegendSize, NumberFormat, TextSize};
    use llevant_render::scale::{Ramp, Scale};
    let st = &state.style;
    let set = |out: &mut Vec<Command>, f: &dyn Fn(&mut crate::state::FieldStyle)| {
        let mut n = st.clone();
        f(&mut n);
        if n != *st {
            out.push(Edit::FieldStyle(Box::new(n)).into());
        }
    };
    let heading = |ui: &mut egui::Ui, t: &str| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(t).strong());
    };
    heading(ui, "Colours");
    let now = st.ramp();
    for r in Ramp::ALL {
        ui.horizontal(|ui| {
            let preview = Scale {
                ramp: r,
                bands: st.bands,
                ..Scale::linear(0.0, 1.0)
            };
            if ui
                .selectable_label(r == now, format!("{:8}", r.name()))
                .clicked()
            {
                set(out, &|s| s.ramp = r.name().to_string());
            }
            ramp_preview(ui, &preview, 150.0);
        });
    }
    heading(ui, "Bands");
    ui.horizontal(|ui| {
        if ui.selectable_label(st.bands == 0, "continuous").clicked() {
            set(out, &|s| s.bands = 0);
        }
        let mut n = if st.bands == 0 { 10 } else { st.bands };
        let r = ui.add(egui::DragValue::new(&mut n).range(2..=32).suffix(" bands"));
        if r.changed() || (r.clicked() && st.bands == 0) {
            set(out, &|s| s.bands = n);
        }
        if ui
            .selectable_label(st.bands > 0, "discrete")
            .on_hover_text("One colour per band (on a log scale, bands of equal decades)")
            .clicked()
            && st.bands == 0
        {
            set(out, &|s| s.bands = 10);
        }
    });
    heading(ui, "Scale");
    scale(ui, doc, state, out);
    if state.scale.is_none() {
        let mut round = !st.exact_ends;
        if ui
            .checkbox(&mut round, "round ends")
            .on_hover_text(
                "The fitted scale's ends moved out to round numbers (decades on a log \
                 scale): ticks at its ends read 1e3, not 1.274e3",
            )
            .changed()
        {
            set(out, &|s| s.exact_ends = !round);
        }
    }
    let shown = doc.field.as_ref().map(|f| f.look.scale);
    let (lo, hi) = shown.map_or((0.0, 1.0), |s| (s.lo, s.hi));
    if let Some(c) = optional_colour(
        ui,
        "below the scale",
        "Values below the scale (and zeros on a log scale) in this colour, not the low \
         end's: a triangle at the bar's low end shows it",
        st.below,
        [255, 255, 255],
    ) {
        set(out, &|s| s.below = c);
    }
    if let Some(c) = optional_colour(
        ui,
        "above the scale",
        "Values above the scale in this colour, not the high end's (above a limit, say): \
         a triangle at the bar's high end shows it",
        st.above,
        [230, 30, 30],
    ) {
        set(out, &|s| s.above = c);
    }
    threshold(ui, state, (lo, hi), out);
    let mut opacity = state.opacity;
    if ui
        .add(egui::Slider::new(&mut opacity, 0.0..=1.0).text("opacity"))
        .on_hover_text("How much the field's colours cover the cells'")
        .changed()
    {
        out.push(Edit::FieldOpacity(opacity).into());
    }
    let mut under = state.geometry_under;
    if ui
        .checkbox(&mut under, "cell colours under it")
        .on_hover_text(
            "Off, the cells are grey under the field, and grey where it has no value, so \
             that only the scale's colours read as values. On, the geometry's colours \
             show through (with opacity under 1) and where there is no value",
        )
        .changed()
    {
        out.push(Edit::FieldGeometryUnder(under).into());
    }
    heading(ui, "Legend");
    let l = &st.legend;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("place").weak());
        for (c, name) in [
            (Corner::TopLeft, "↖"),
            (Corner::TopRight, "↗"),
            (Corner::BottomLeft, "↙"),
            (Corner::BottomRight, "↘"),
        ] {
            if ui.selectable_label(l.corner == c, name).clicked() {
                set(out, &|s| s.legend.corner = c);
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.selectable_label(!l.vertical, "horizontal").clicked() {
            set(out, &|s| s.legend.vertical = false);
        }
        if ui.selectable_label(l.vertical, "vertical").clicked() {
            set(out, &|s| s.legend.vertical = true);
        }
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("bar").weak());
        for (z, name) in [
            (LegendSize::Small, "S"),
            (LegendSize::Medium, "M"),
            (LegendSize::Large, "L"),
        ] {
            let on = l.size == z && l.length.is_none();
            if ui.selectable_label(on, name).clicked() {
                set(out, &|s| {
                    s.legend.size = z;
                    s.legend.length = None;
                });
            }
        }
        let mut length = l.length();
        let r = ui
            .add(
                egui::DragValue::new(&mut length)
                    .range(40.0..=2000.0)
                    .speed(2.0)
                    .suffix(" pt"),
            )
            .on_hover_text("The bar's length (it grows if its end labels need more)");
        theme::name(&r, "legend length");
        if r.changed() {
            set(out, &|s| s.legend.length = Some(length));
        }
        let mut thick = l.thickness();
        let r = ui
            .add(
                egui::DragValue::new(&mut thick)
                    .range(4.0..=60.0)
                    .speed(0.5)
                    .prefix("× ")
                    .suffix(" pt"),
            )
            .on_hover_text("The bar's thickness");
        theme::name(&r, "legend thickness");
        if r.changed() {
            set(out, &|s| s.legend.thickness = Some(thick));
        }
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("text").weak());
        for (z, name) in [
            (TextSize::Small, "S"),
            (TextSize::Medium, "M"),
            (TextSize::Large, "L"),
        ] {
            if ui.selectable_label(l.text == z, name).clicked() {
                set(out, &|s| s.legend.text = z);
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("numbers").weak());
        for (f, name) in [
            (NumberFormat::Short, "1e12"),
            (NumberFormat::Powers, "10¹²"),
            (NumberFormat::Decimal, "0.001"),
        ] {
            if ui
                .selectable_label(l.numbers == f, name)
                .on_hover_text("Every number of the bar in the same notation")
                .clicked()
            {
                set(out, &|s| s.legend.numbers = f);
            }
        }
        let mut d = l.digits;
        let r = ui
            .add(
                egui::DragValue::new(&mut d)
                    .range(0..=6)
                    .custom_formatter(|v, _| {
                        if v < 0.5 {
                            "auto digits".into()
                        } else {
                            format!("{v:.0} digits")
                        }
                    }),
            )
            .on_hover_text("Significant digits at most (auto: as few as write each number)");
        theme::name(&r, "legend digits");
        if r.changed() {
            set(out, &|s| s.legend.digits = d);
        }
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("ticks").weak());
        let edges = l.band_ticks;
        let auto = l.tick_count < 2 && !edges;
        if ui
            .selectable_label(auto && !l.many_ticks, "few")
            .on_hover_text("The ends and a few round values; labels that would overlap left out")
            .clicked()
        {
            set(out, &|s| {
                s.legend.many_ticks = false;
                s.legend.tick_count = 0;
                s.legend.band_ticks = false;
            });
        }
        if ui
            .selectable_label(auto && l.many_ticks, "many")
            .on_hover_text("Every decade of a log scale; labels that would overlap left out")
            .clicked()
        {
            set(out, &|s| {
                s.legend.many_ticks = true;
                s.legend.tick_count = 0;
                s.legend.band_ticks = false;
            });
        }
        let counted = l.tick_count >= 2 && !edges;
        let mut n = if counted { l.tick_count } else { 5 };
        let r = ui
            .add(egui::DragValue::new(&mut n).range(2..=40).suffix(" ticks"))
            .on_hover_text(
                "Exactly this many, ends included, evenly spaced (in decades on a log \
                 scale), every one labelled: the bar grows, or the labels take two rows, \
                 to fit them",
            );
        theme::name(&r, "legend ticks");
        if r.changed() || (r.clicked() && !counted) {
            set(out, &|s| {
                s.legend.tick_count = n;
                s.legend.band_ticks = false;
            });
        }
        let r = ui
            .add_enabled(st.bands >= 2, egui::Button::selectable(edges, "band edges"))
            .on_hover_text("A tick exactly at each edge between bands, every one labelled")
            .on_disabled_hover_text("With discrete bands");
        if r.clicked() {
            set(out, &|s| s.legend.band_ticks = !edges);
        }
    });
    // Band edges on decades need decade ends and whole decades a band.
    if l.band_ticks {
        if let Some(scale) = doc.field.as_ref().map(|f| f.look.scale) {
            if llevant_render::legend::edges_on_decades(&scale) == Some(false) {
                let decades = (scale.hi / scale.lo).log10();
                ui.label(
                    egui::RichText::new(format!(
                        "{decades:.3} decades in {} bands: the edges are not on decades \
                         (round ends, and bands that divide the decades, put them there)",
                        scale.bands
                    ))
                    .weak(),
                );
            }
        }
    }
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("title").weak());
        let mut title = l.title.clone().unwrap_or_default();
        let hint = doc
            .field
            .as_ref()
            .map_or(String::new(), |f| f.field.title());
        let r = ui.add(egui::TextEdit::singleline(&mut title).hint_text(hint));
        theme::name(&r, "legend title");
        if r.changed() {
            let t = (!title.trim().is_empty()).then(|| title.clone());
            set(out, &|s| s.legend.title = t.clone());
        }
    });
}

/// The tally file's run, its tallies to choose from (those that can't be
/// drawn greyed, saying why), the bins along each axis of the one chosen
/// that has several, results or errors, and the run's input; each change
/// reads the field again. Why the choice could not be read, if it
/// couldn't.
fn tally(
    ui: &mut egui::Ui,
    doc: &Document,
    state: &FieldState,
    summary: &FileSummary,
    out: &mut Vec<Command>,
) {
    let s = &state.source;
    let p = theme::palette(ui.ctx());
    let mut run = vec![summary.code.clone()];
    if let Some(h) = summary.histories {
        run.push(format!("{} histories", tick_label(h)));
    }
    ui.label(egui::RichText::new(run.join(" · ")).weak())
        .on_hover_text(&summary.title);
    if let Some(e) = doc.field_error(s) {
        ui.label(egui::RichText::new(e).color(p.error));
    }
    let choice = s.choice().unwrap_or_default();
    let mut next = choice;
    let shown = choice.tally.or(summary.default);
    let current = shown.and_then(|id| summary.tally(id));

    // The tally: number, kind and comment, as a menu.
    let line = |t: &llevant_scene::field::TallySummary| {
        let mut l = format!("{}  {}", t.id, t.kind);
        if let Some(c) = &t.comment {
            l += &format!("  {c}");
        }
        l
    };
    let picked = egui::ComboBox::from_id_salt("tally")
        .selected_text(current.map_or_else(|| "none".to_string(), line))
        .width(ui.available_width().min(320.0))
        .truncate()
        .show_ui(ui, |ui| {
            let mut picked = None;
            for t in &summary.tallies {
                let r = ui.add_enabled(
                    t.not_drawn.is_none(),
                    egui::Button::selectable(Some(t.id) == shown, line(t)),
                );
                let r = match &t.not_drawn {
                    Some(why) => r.on_disabled_hover_text(why),
                    None if !t.notes.is_empty() => r.on_hover_text(t.notes.join("\n")),
                    None => r,
                };
                if r.clicked() {
                    picked = Some(t);
                }
            }
            for (id, why) in &summary.failed {
                ui.add_enabled(
                    false,
                    egui::Button::selectable(false, format!("{id}  not read")),
                )
                .on_disabled_hover_text(why);
            }
            picked
        });
    theme::name(&picked.response, "tally");
    picked.response.on_hover_text("The tally drawn");
    if let Some(t) = picked.inner.flatten() {
        if Some(t.id) != choice.tally {
            // Its Totals, or its first bins where it has none.
            next = TallyChoice {
                error: choice.error,
                ..t.choice()
            };
        }
    }

    if let Some(t) = current {
        // The list of cell bins below counts them.
        if t.bins.is_empty() {
            ui.label(
                egui::RichText::new(format!(
                    "{} {}",
                    t.cells,
                    if matches!(
                        summary.format,
                        llevant_scene::field::Format::Meshtal
                            | llevant_scene::field::Format::Cuvmsh
                    ) {
                        "voxels"
                    } else {
                        "cell bins"
                    }
                ))
                .weak(),
            );
        }
        for n in &t.notes {
            ui.label(egui::RichText::new(n).color(p.warning));
        }
        if !t.warnings.is_empty() {
            egui::CollapsingHeader::new(format!("{} warnings", t.warnings.len()))
                .id_salt("tally warnings")
                .show(ui, |ui| {
                    for w in &t.warnings {
                        ui.label(egui::RichText::new(w).weak());
                    }
                });
        }
        // A menu per axis with several bins.
        egui::Grid::new("tally bins").num_columns(2).show(ui, |ui| {
            for a in &t.axes {
                ui.label(egui::RichText::new(a.name).weak());
                let mut bin = bin_of(&next, a.dim);
                if axis_menu(ui, a, &mut bin) {
                    next.set_bin(&a.dim.to_string(), bin).expect("a dimension");
                }
                ui.end_row();
            }
        });
    }
    ui.checkbox(&mut next.error, "relative errors")
        .on_hover_text("The results' relative errors instead of the results");

    // The run's input, for what the file leaves out.
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("run input").weak());
        let name = s
            .input
            .as_ref()
            .and_then(|i| i.file_name())
            .map_or_else(|| "the model".to_string(), |n| n.to_string_lossy().into());
        let wants = current.is_some_and(|t| t.wants_input);
        let text = egui::RichText::new(name);
        ui.label(if wants {
            text.color(p.warning)
        } else {
            theme::data(text.text())
        })
        .on_hover_text(
            "The MCNP input of the run: its F cards say which cells each MCTAL bin is, its \
                 FMESH cards where each mesh is exactly",
        );
        if ui
            .small_button(icons::FOLDER_OPEN)
            .on_hover_text("Choose the run's input")
            .clicked()
        {
            out.push(Command::OpenRunInputDialog);
        }
        if s.input.is_some()
            && ui
                .small_button(icons::REMOVE)
                .on_hover_text("The model's input")
                .clicked()
        {
            out.push(Command::LoadField(FieldSource {
                input: None,
                ..s.clone()
            }));
        }
    });

    if let Some(t) = current.filter(|t| !t.bins.is_empty()) {
        cell_bins(ui, doc, state, t, out);
    }

    if next != choice {
        out.push(Command::LoadField(s.with_choice(&next)));
    }
}

/// Height of a row of the list of cell bins.
const ROW: f32 = 20.0;

/// The cell bins of tally `t`: each as MCNP names it, with its value and
/// relative error once read; a click selects its cells (Ctrl adds, Shift
/// removes; double click frames them), a button isolates them. Above,
/// the cells of every bin selected or isolated at once.
fn cell_bins(
    ui: &mut egui::Ui,
    doc: &Document,
    state: &FieldState,
    t: &TallySummary,
    out: &mut Vec<Command>,
) {
    let values = doc
        .field_bins(&state.source)
        .filter(|v| v.len() == t.bins.len());
    let mut all: Vec<i64> = t.bins.iter().flat_map(|b| b.cells.clone()).collect();
    all.sort_unstable();
    all.dedup();
    let m = &doc.model;
    let selected = |cells: &[i64]| {
        !cells.is_empty()
            && cells.iter().all(|&c| {
                m.cell_index(c)
                    .is_some_and(|i| doc.selection.cells.contains(i))
            })
    };
    egui::CollapsingHeader::new(format!("{} cell bins", t.bins.len()))
        .id_salt("tally cell bins")
        .default_open(true)
        .show(ui, |ui| {
            if !all.is_empty() {
                let q = Query::Cells(all.clone());
                ui.horizontal_wrapped(|ui| {
                    ui.label(theme::data(format!(
                        "{} {}",
                        all.len(),
                        if all.len() == 1 { "cell" } else { "cells" }
                    )));
                    if ui
                        .small_button("select all")
                        .on_hover_text("Select the cells of every bin")
                        .clicked()
                    {
                        out.push(Command::Select(q.clone(), SelectMode::Replace));
                    }
                    if ui
                        .small_button(icons::ISOLATE)
                        .on_hover_text("Isolate the tally's cells (with all they hold)")
                        .clicked()
                    {
                        out.push(Edit::Isolate(Some(q.clone())).into());
                    }
                    if doc.scene.hiding.isolate.is_some()
                        && ui
                            .small_button(icons::SHOW)
                            .on_hover_text("Stop isolating")
                            .clicked()
                    {
                        out.push(Edit::Isolate(None).into());
                    }
                });
            }
            if t.bins.iter().any(|b| !b.whole_cells && !b.cells.is_empty()) {
                ui.label(
                    egui::RichText::new(
                        "Some bins score only some instances of their cells (chains): \
                         isolating them shows every instance",
                    )
                    .weak(),
                );
            }
            egui::ScrollArea::vertical()
                .id_salt("tally cell bins")
                .max_height(12.0 * ROW)
                .auto_shrink([false, true])
                .show_rows(ui, ROW, t.bins.len(), |ui, range| {
                    for k in range {
                        let b = &t.bins[k];
                        ui.horizontal(|ui| {
                            let r = ui.add_enabled(
                                !b.cells.is_empty(),
                                egui::Button::selectable(
                                    selected(&b.cells),
                                    theme::data(b.label.clone()),
                                ),
                            );
                            let r = if b.total {
                                r.on_disabled_hover_text("The T bin: every other bin together")
                            } else if b.cells.is_empty() {
                                r.on_disabled_hover_text(
                                    "Only the run's input (its F card) says which cells this \
                                     bin is",
                                )
                            } else {
                                let cells: Vec<String> =
                                    b.cells.iter().map(|c| c.to_string()).collect();
                                r.on_hover_text(format!(
                                    "cell {}{}: click to select, double click to frame",
                                    cells.join(" "),
                                    if b.whole_cells {
                                        ""
                                    } else {
                                        " (some of its instances)"
                                    }
                                ))
                            };
                            let q = || Query::Cells(b.cells.clone());
                            if r.clicked() {
                                let mode = SelectMode::from_modifiers(ui.input(|i| i.modifiers));
                                out.push(Command::Select(q(), mode));
                            }
                            if r.double_clicked() {
                                out.push(Command::Select(q(), SelectMode::Replace));
                                out.push(Command::FrameSelection);
                            }
                            if !b.cells.is_empty()
                                && ui
                                    .small_button(icons::ISOLATE)
                                    .on_hover_text("Isolate this bin's cells")
                                    .clicked()
                            {
                                out.push(Edit::Isolate(Some(q())).into());
                            }
                            if let Some(&(v, e)) = values.map(|v| &v[k]) {
                                ui.label(egui::RichText::new(bin_value(v, e)).weak());
                            }
                        });
                    }
                });
        });
}

/// A bin's result and relative error: `1.235e-5 ± 0.6 %`; nothing scored
/// says so.
fn bin_value(v: f64, e: f64) -> String {
    if v == 0.0 {
        "not scored".into()
    } else {
        format!("{} ± {:.2} %", value_label(v), 100.0 * e)
    }
}

/// The bin of `c` along dimension `dim`.
fn bin_of(c: &TallyChoice, dim: char) -> Bin {
    match dim {
        'e' => c.energy,
        't' => c.time,
        d => llevant_scene::field::OTHER_DIMS
            .iter()
            .position(|&x| x == d)
            .map_or(Bin::Total, |k| c.other[k]),
    }
}

/// A menu of the bins of `a`: its Total, or the sum of its bins when it
/// has none, then each bin; true if `bin` changed.
fn axis_menu(ui: &mut egui::Ui, a: &AxisSummary, bin: &mut Bin) -> bool {
    let before = *bin;
    let r = egui::ComboBox::from_id_salt(("bin", a.dim))
        .selected_text(a.label(*bin))
        .show_ui(ui, |ui| {
            if a.total {
                ui.selectable_value(bin, Bin::Total, "Total");
            } else {
                ui.selectable_value(bin, Bin::Sum, a.label(Bin::Sum))
                    .on_hover_text("The bins added up, errors as if independent");
            }
            for k in 0..a.labels.len() {
                ui.selectable_value(bin, Bin::Index(k), &a.labels[k]);
            }
        });
    theme::name(&r.response, a.name);
    // The Total and a sum with one are the same bin.
    let same = |b: Bin| {
        if a.total && b == Bin::Sum {
            Bin::Total
        } else {
            b
        }
    };
    same(*bin) != same(before)
}

/// The colour scale: fitted to the values or set, its ends, logarithmic or
/// not, and the ramp.
fn scale(ui: &mut egui::Ui, doc: &Document, state: &FieldState, out: &mut Vec<Command>) {
    let Some(f) = &doc.field else {
        if !state.shown {
            ui.label(egui::RichText::new("hidden").weak());
        }
        return;
    };
    let s = f.look.scale;
    let mut set = state.scale.is_some();
    ui.horizontal(|ui| {
        if ui.selectable_label(!set, "fitted").clicked() && set {
            out.push(Edit::FieldScale(None).into());
            set = false;
        }
        if ui.selectable_label(set, "set").clicked() && !set {
            out.push(
                Edit::FieldScale(Some(ScaleState {
                    lo: s.lo,
                    hi: s.hi,
                    log: s.log,
                }))
                .into(),
            );
        }
        let mut log = s.log;
        if ui.checkbox(&mut log, "log").changed() {
            let (lo, hi) = if log && !(s.lo > 0.0) {
                (s.hi.abs().max(1e-30) * 1e-3, s.hi.abs().max(1e-30))
            } else {
                (s.lo, s.hi)
            };
            out.push(Edit::FieldScale(Some(ScaleState { lo, hi, log })).into());
        }
    });
    ui.horizontal(|ui| {
        let (mut lo, mut hi) = (s.lo, s.hi);
        let speed = |x: f64| (x.abs() * 0.01).max(1e-12);
        let (lo_speed, hi_speed) = (speed(lo), speed(hi));
        let a = ui.add(
            egui::DragValue::new(&mut lo)
                .speed(lo_speed)
                .custom_formatter(|v, _| tick_label(v)),
        );
        ui.label("to");
        let b = ui.add(
            egui::DragValue::new(&mut hi)
                .speed(hi_speed)
                .custom_formatter(|v, _| tick_label(v)),
        );
        if (a.changed() || b.changed()) && lo < hi && (!s.log || lo > 0.0) {
            out.push(Edit::FieldScale(Some(ScaleState { lo, hi, log: s.log })).into());
        }
    });
    // The ramp, with the scale's ticks.
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().min(240.0), 26.0),
        egui::Sense::hover(),
    );
    let bar = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 10.0));
    let n = 60;
    for k in 0..n {
        let [r, g, b] = s.colour(s.value(k as f64 / (n - 1) as f64));
        let x0 = bar.left() + bar.width() * k as f32 / n as f32;
        let x1 = bar.left() + bar.width() * (k + 1) as f32 / n as f32;
        ui.painter().rect_filled(
            egui::Rect::from_min_max(egui::pos2(x0, bar.top()), egui::pos2(x1, bar.bottom())),
            0.0,
            egui::Color32::from_rgb(r, g, b),
        );
    }
    let ink = ui.visuals().text_color();
    for (v, label) in s.ticks() {
        let x = bar.left() + bar.width() * s.fraction(v) as f32;
        ui.painter().text(
            egui::pos2(
                x.clamp(bar.left() + 8.0, bar.right() - 8.0),
                bar.bottom() + 2.0,
            ),
            egui::Align2::CENTER_TOP,
            label,
            egui::FontId::monospace(10.0),
            ink,
        );
    }
}

/// The edit setting the field to `state` changed by `f`.
fn edit(state: &FieldState, f: impl FnOnce(&mut FieldState)) -> Command {
    let mut s = state.clone();
    f(&mut s);
    Edit::Field(Some(Box::new(s))).into()
}

/// A colour that may be set: a checkbox, and a colour button when on; the
/// new choice if it changed.
fn optional_colour(
    ui: &mut egui::Ui,
    label: &str,
    hint: &str,
    current: Option<[u8; 3]>,
    default: [u8; 3],
) -> Option<Option<[u8; 3]>> {
    let mut on = current.is_some();
    let mut c = current.unwrap_or(default);
    let mut changed = None;
    ui.horizontal(|ui| {
        if ui.checkbox(&mut on, label).on_hover_text(hint).changed() {
            changed = Some(on.then_some(c));
        }
        if on && egui::color_picker::color_edit_button_srgb(ui, &mut c).changed() {
            changed = Some(Some(c));
        }
    });
    changed
}

/// The threshold: only values from a bound to another drawn (each one
/// optional, starting at the scale's end).
fn threshold(ui: &mut egui::Ui, state: &FieldState, ends: (f64, f64), out: &mut Vec<Command>) {
    let mut t = state.threshold;
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("only values").weak())
            .on_hover_text("The others are drawn as no value");
        for (k, word, end) in [(0, "from", ends.0), (1, "to", ends.1)] {
            let mut on = t[k].is_some();
            if ui.checkbox(&mut on, word).changed() {
                t[k] = on.then_some(end);
                changed = true;
            }
            if let Some(v) = &mut t[k] {
                let speed = (v.abs() * 0.01).max(1e-12);
                let r = ui.add(
                    egui::DragValue::new(v)
                        .speed(speed)
                        .custom_formatter(|v, _| tick_label(v)),
                );
                theme::name(&r, &format!("threshold {word}"));
                changed |= r.changed();
            }
        }
    });
    if changed {
        out.push(Edit::FieldThreshold(t).into());
    }
}
