/// The system heap on Windows does not scale to large parallel batches
/// (docs/decisions/0009-mimalloc.md).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> std::process::ExitCode {
    llevant_cli::run_cli(std::env::args_os()).into()
}
