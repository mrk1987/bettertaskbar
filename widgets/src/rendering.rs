//! Small UI surfaces should not allocate game-sized DX12 heaps per process.
use std::sync::Arc;

pub fn configure(options: &mut eframe::NativeOptions) {
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        let descriptor = Arc::clone(&setup.device_descriptor);
        setup.device_descriptor = Arc::new(move |adapter| {
            let mut descriptor = descriptor(adapter);
            descriptor.memory_hints = eframe::wgpu::MemoryHints::MemoryUsage;
            // Wgpu defaults to one million slots. Themes have at most 64
            // elements; 4096 leaves ample room for dashboard images and fonts.
            descriptor.required_limits.max_non_sampler_bindings = 4096;
            descriptor
        });
    }
}
