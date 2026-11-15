//! The 3D view and the slice in an eframe app of one's own:
//! `cargo run --release -p llevant-viewer --example embed -- model.mcnp`.
//! Left: 3D (drag to orbit, scroll to zoom), with an effect of this app's
//! own; right: the slice. `ViewerApp` (`app/`) is the full version.

use std::sync::Arc;

use llevant::{Model, Options, Scope, Visibility};
use llevant_gpu::{Gpu, GpuModel};
use llevant_render::colours::ColourBy;
use llevant_scene::materials::Looks;
use llevant_viewer::camera::{visible_extent, Orbit};
use llevant_viewer::slice::SliceView;
use llevant_viewer::state::SliceState;
use llevant_viewer::view3d::backend::{beside_window, GpuBackend, TIMESTAMPS};
use llevant_viewer::view3d::passes::WgslPass;
use llevant_viewer::view3d::View3d;

struct App {
    rs: egui_wgpu::RenderState,
    view: View3d,
    orbit: Orbit,
    slice: SliceView,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, path: &str) -> Result<App, String> {
        let err = |e: llevant::Error| e.to_string();
        let rs = cc
            .wgpu_render_state
            .clone()
            .ok_or("needs the wgpu renderer")?;
        // The window's device, for the views' images.
        let window_gpu = Arc::new(Gpu::from_parts(
            rs.device.clone(),
            rs.queue.clone(),
            rs.adapter.get_info(),
        ));
        let model = Arc::new(Model::load(path, &Options::default(), None).map_err(err)?);
        // What materials look like, for realistic colours (none overridden).
        let looks = Arc::new(Looks::new(&model, &[]).map_err(err)?);
        let vis = Visibility::solid(&model);
        let region = model.extent(Scope::Contents, 64);
        let region = visible_extent(&model, &vis, &region, 48).unwrap_or(region);

        // The GPU renderer on a device of its own, not the window's.
        let trace_gpu = Arc::new(Gpu::from_adapter(&rs.adapter, TIMESTAMPS).map_err(err)?);
        let mut gm = GpuModel::upload(&trace_gpu, &model).map_err(err)?;
        beside_window(&mut gm);
        let gm = Arc::new(gm);
        gm.set_visibility(&trace_gpu, &model, &vis).map_err(err)?;
        let backend = GpuBackend::new(trace_gpu, gm, model.clone());
        let mut view = View3d::new(
            window_gpu,
            model.clone(),
            looks.clone(),
            Box::new(backend),
            &vis,
            0.033,
            ColourBy::Cell,
        );

        // An effect of this app's own: fade far hits towards blue.
        let gpu = view.gpu().clone();
        view.passes().push(
            &gpu,
            Box::new(WgslPass {
                name: "depth tint".into(),
                source: r#"
fn effect(p: vec2<i32>) -> vec4<f32> {
    let c = source(p);
    if !hit_any(p) {
        return c;
    }
    let d = clamp(hit_t(p) / F.eye.w, 0.0, 1.0);
    return vec4<f32>(mix(c.rgb, F.params[0].xyz, d * 0.5), c.a);
}
"#
                .into(),
                params: [0.2, 0.3, 0.8, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            }),
            true,
        )?;

        let slice = SliceView::new(model, looks, region, &SliceState::default(), ColourBy::Cell);
        Ok(App {
            rs,
            view,
            orbit: Orbit::fit(&region, 35.0, 25.0),
            slice,
        })
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::no_frame().show(ui, |ui| {
            ui.columns(2, |cols| {
                // 3D: a rectangle of the ui, in physical pixels for the camera.
                let ui = &mut cols[0];
                let size = ui.available_size();
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::drag());
                let d = response.drag_delta();
                self.orbit.orbit(d.x as f64, d.y as f64);
                if response.hovered() {
                    self.orbit
                        .zoom(ui.input(|i| i.smooth_scroll_delta.y) as f64);
                }
                let ppp = ui.ctx().pixels_per_point();
                let (w, h) = (
                    ((rect.width() * ppp) as usize).max(8),
                    ((rect.height() * ppp) as usize).max(8),
                );
                if let Some(tex) = self.view.frame(&self.rs, self.orbit.camera(w, h, None)) {
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    ui.painter().image(tex, rect, uv, egui::Color32::WHITE);
                }
                if self.view.busy() {
                    ui.ctx().request_repaint();
                }

                // The slice: a widget that takes the rest of its ui.
                let events = self.slice.ui(&mut cols[1]);
                if let Some(p) = events.clicked {
                    println!("clicked ({:.2}, {:.2}, {:.2})", p[0], p[1], p[2]);
                }
            });
        });
    }
}

fn main() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("usage: embed <model>")?;
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        // The adapter's own limits (big models need large buffers), Vulkan
        // first.
        wgpu_options: llevant_viewer::app::wgpu_options(),
        ..Default::default()
    };
    eframe::run_native(
        "llevant embed",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, &path)?))),
    )
    .map_err(|e| e.to_string())
}
