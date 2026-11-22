//! What the GPU the tests paint on can do (ADR-0016, ADR-0019, ADR-0023).
//! Compute is required: the renderer takes no adapter without it. Dual-source
//! blending is what LCD text needs; without it subpixel text falls back to
//! grey and the LCD path goes untested. CI runs on Mesa's lavapipe with
//! `GUI_REQUIRE_DUAL_SOURCE=1`, so a lavapipe that loses either fails the
//! run instead of quietly testing less.

use gui::gpu::GpuRenderer;

#[test]
fn the_adapter_runs_compute_and_blends_dual_source_where_required() {
    // The backends the renderer takes (no OpenGL), as `gpu::instance` asks.
    let ours = wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::DX12;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().map_or(ours, |b| b & ours),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .expect("a GPU adapter (CI: Mesa's lavapipe)");
    let info = adapter.get_info();
    let downlevel = adapter.get_downlevel_capabilities();
    let compute = downlevel.flags.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS);
    let indirect = downlevel.flags.contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION);
    let dual = adapter.features().contains(wgpu::Features::DUAL_SOURCE_BLENDING);
    eprintln!(
        "adapter: {} ({:?}, {:?}, driver {} {}): compute {compute}, indirect {indirect}, dual-source blending {dual}",
        info.name, info.device_type, info.backend, info.driver, info.driver_info
    );
    assert!(compute && indirect, "{} cannot run compute shaders", info.name);

    let gpu = GpuRenderer::headless().expect("the renderer takes the adapter");
    assert_eq!(
        gpu.dual_source(),
        dual,
        "the renderer blends dual-source where the adapter can"
    );
    if std::env::var_os("GUI_REQUIRE_DUAL_SOURCE").is_some() {
        assert!(
            dual,
            "{} has no dual-source blending: LCD text is untested on it (ADR-0023: run the LCD goldens on a Windows runner)",
            info.name
        );
    }
}
