#[test]
fn local_main_viewport_input_is_authorized() {
    let mut context: tauri::Context<tauri::Wry> = tauri::generate_context!();
    let authority = context.runtime_authority_mut();
    assert!(authority
        .resolve_access("viewport_input", "main", "main", &tauri::ipc::Origin::Local)
        .is_some());
    assert!(authority
        .resolve_access(
            "viewport_input",
            "other",
            "other",
            &tauri::ipc::Origin::Local
        )
        .is_none());
    assert!(authority
        .resolve_access(
            "viewport_input",
            "main",
            "main",
            &tauri::ipc::Origin::Remote {
                url: "https://example.com".parse().unwrap()
            },
        )
        .is_none());
}
