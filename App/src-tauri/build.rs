fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(
                tauri_build::WindowsAttributes::new().window_icon_path("../../Assets/app.ico"),
            )
            .app_manifest(tauri_build::AppManifest::new().commands(&[
                "bootstrap",
                "choose_file",
                "load_preview",
                "cancel_load",
                "register_associations",
                "unregister_associations",
                "show_licenses",
            ])),
    )
    .expect("Unable to build Litematica Preview resources");
}
