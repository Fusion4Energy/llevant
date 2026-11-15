//! The window's GPU device.

use std::sync::Arc;

/// Set when the window's GPU device is lost (egui cannot draw on).
pub(crate) static WINDOW_LOST: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Open the device eframe should use: the adapter's own limits (large
/// storage buffers), Vulkan before other backends where both exist (the
/// D3D12 shader compiler, FXC, sometimes fails on the renderer's loops).
pub fn wgpu_options() -> egui_wgpu::WgpuConfiguration {
    use egui_wgpu::{WgpuSetup, WgpuSetupCreateNew};
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.device_descriptor = Arc::new(|adapter| wgpu::DeviceDescriptor {
        label: Some("llevant viewer"),
        required_limits: adapter.limits(),
        // GPU timestamps time the tracing alone (not the frames queued
        // with it) when the adapter has them.
        required_features: adapter.features() & crate::view3d::backend::TIMESTAMPS,
        ..Default::default()
    });
    setup.native_adapter_selector = Some(Arc::new(|adapters, surface| {
        let rank = |a: &wgpu::Adapter| llevant_gpu::adapter_rank(&a.get_info());
        let mut ok: Vec<&wgpu::Adapter> = adapters
            .iter()
            .filter(|a| surface.is_none_or(|s| a.is_surface_supported(s)))
            .collect();
        ok.sort_by_key(|a| rank(a));
        ok.first()
            .map(|a| (*a).clone())
            .ok_or_else(|| "no GPU adapter can show the window".to_string())
    }));
    egui_wgpu::WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..Default::default()
    }
}
