// The app's own commands, each a permission of its own (allow-show-site...), given in capabilities/ only to the
// app's own page.
fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
        "show_site",
        "place_site",
        "hide_site",
        "unhide_site",
        "close_site",
        "site_media",
        "search_files",
        "open_file",
        "about",
    ])))
    .expect("tauri-build couldn't run");
}
