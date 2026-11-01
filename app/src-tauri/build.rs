mod app_commands;

fn main() {
    println!("cargo:rerun-if-changed=app_commands.rs");
    println!("cargo:rerun-if-changed=windows.manifest");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(app_commands::COMMANDS))
            .windows_attributes(
                tauri_build::WindowsAttributes::new()
                    .app_manifest(include_str!("windows.manifest")),
            ),
    )
    .expect("failed to build Tauri application manifest");
}
