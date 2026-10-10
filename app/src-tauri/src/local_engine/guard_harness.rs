//! Synthetic test-only entry. Production dispatch lives before Tauri startup.
#[path = "guard_core.rs"]
mod guard_core;
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    std::process::exit(guard_core::dispatch(&args).unwrap_or(125));
}
