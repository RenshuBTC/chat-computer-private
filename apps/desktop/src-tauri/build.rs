// The app's own commands, each a permission of its own (allow-show-site...), given in capabilities/ only to the
// app's own page (the chatgpt_ ones: signing in with ChatGPT and asking it, src/chatgpt.rs); but messages_found,
// given only to the connected services' own views, on their own pages.
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
        "connect_service",
        "forget_service",
        "open_service_item",
        "model_status",
        "chatgpt_status",
        "chatgpt_sign_in",
        "chatgpt_sign_out",
        "chatgpt_ask",
        "way_status",
        "choose_way",
        "show_folder",
        "messages_found",
    ])))
    .expect("tauri-build couldn't run");
}
