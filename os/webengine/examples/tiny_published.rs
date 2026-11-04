//! Writes a small model with random weights as Qwen publishes its models (webengine::fixture: config.json,
//! tokenizer.json, the chat template, two safetensors files), for tests/test_web_engine.py: the page must make the
//! same model's file from it as the engine's own program does.
//!
//!     cargo run -p webengine --example tiny_published -- FOLDER
fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("the folder to write it in"));
    std::fs::create_dir_all(&dir).expect("making the folder");
    for (name, bytes) in webengine::fixture::files() {
        std::fs::write(dir.join(&name), bytes).expect("writing it");
    }
}
