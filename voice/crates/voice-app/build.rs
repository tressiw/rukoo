// Every command voice/ui/API.md lists. Naming them gives each an `allow-<command>` permission, so the
// capabilities can give the pill only the pill_* commands and Settings only the settings_* ones.
const COMMANDS: &[&str] = &[
    "pill_snapshot",
    "pill_ack",
    "pill_layout",
    "settings_state",
    "settings_set",
    "settings_save_provider",
    "settings_remove_key",
    "settings_list_models",
    "settings_model",
    "settings_choose_folder",
    "settings_open_folder",
    "settings_open_link",
    "settings_open_permission",
    "settings_clean_preview",
    "settings_check_updates",
    "settings_new_token",
    "settings_import_ownkey",
    "settings_sync_test",
    "settings_sync_save_secret",
    "settings_sync_key",
    "settings_sync_key_copy",
    "settings_sync_restore",
    "settings_protect",
    "settings_close",
];

/// The resource id of the tray icon for a recording meeting. Load it at the tray's size with
/// `tauri::image::Image::from_icon_resource(32513, cx, cy)`, cx and cy from SM_CXSMICON and SM_CYSMICON, so
/// Windows picks the matching size from the .ico instead of shrinking a large one. It is a number above the app
/// icon's 32512 on purpose: Explorer and shortcuts show an exe's first icon, and named resources and lower numbers
/// come before the app icon.
const TRAY_RECORDING: u16 = 32513;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let tray_recording = std::path::Path::new(&manifest_dir).join("icons/tray-recording.ico");
    println!("cargo:rerun-if-changed=icons/tray-recording.ico");
    // An .rc string doubles its backslashes and quotes.
    let path = tray_recording
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\"\"");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS))
            .windows_attributes(
                tauri_build::WindowsAttributes::new()
                    .append_rc_content(format!("{TRAY_RECORDING} ICON \"{path}\""))
                    // voice/.cargo/config.toml links the whole C runtime statically (+crt-static), as for the
                    // recorder and the local worker. Tauri's own option would put the UCRT back as a DLL import.
                    .static_vc_runtime(false),
            ),
    )
    .expect("tauri-build failed");
}
