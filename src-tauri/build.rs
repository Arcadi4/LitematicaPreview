const EXTENSIONS_SOURCE: &str = "src/formats.rs";

fn extensions() -> Vec<String> {
    let source = std::fs::read_to_string(EXTENSIONS_SOURCE)
        .unwrap_or_else(|error| panic!("Unable to read {EXTENSIONS_SOURCE}: {error}"));
    let body = source
        .split_once("pub const EXTENSIONS: &[&str] = &[")
        .and_then(|(_, rest)| rest.split_once("];"))
        .map(|(body, _)| body)
        .unwrap_or_else(|| {
            panic!("{EXTENSIONS_SOURCE} must define `EXTENSIONS` as a string slice")
        });
    body.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            entry
                .strip_prefix('"')
                .and_then(|entry| entry.strip_suffix('"'))
                .unwrap_or_else(|| panic!("`EXTENSIONS` entry is not a string literal: {entry}"))
                .to_string()
        })
        .collect()
}

fn main() {
    println!("cargo:rerun-if-changed={EXTENSIONS_SOURCE}");
    let mut include =
        String::from("; Generated from src/formats.rs.\n!macro LP_FOREACH_EXTENSION APPLY\n");
    for extension in extensions() {
        include.push_str(&format!(
            "  !insertmacro ${{APPLY}} \"{}\" \"{}\"\n",
            &extension[1..],
            extension
        ));
    }
    include.push_str("!macroend\n");
    std::fs::create_dir_all("gen").expect("Unable to create installer metadata directory");
    std::fs::write("gen/installer-extensions.nsh", include)
        .expect("Unable to generate installer extension choices");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(
                tauri_build::WindowsAttributes::new().window_icon_path("../Assets/app.ico"),
            )
            .app_manifest(tauri_build::AppManifest::new().commands(&[
                "bootstrap",
                "choose_file",
                "start_preview",
                "preview_memory",
                "cancel_load",
                "copy_preview_image",
                "export_preview_image",
                "viewport_input",
                "register_associations",
                "unregister_associations",
                "show_licenses",
                "import_texture_packs",
                "set_texture_pack_enabled",
                "set_texture_pack_order",
                "remove_texture_pack",
            ])),
    )
    .expect("Unable to build Litematica Preview resources");
}
