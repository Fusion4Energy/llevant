//! The adapter llevant-gpu would use, and its limits.
//!
//! cargo run --release -p llevant-gpu --example gpu_info

pub fn run(_args: Vec<String>) {
    match llevant_gpu::Gpu::new() {
        Ok(gpu) => {
            println!("{:#?}", gpu.info);
            println!("{:#?}", gpu.device.limits());
        }
        Err(e) => println!("{e}"),
    }
}
