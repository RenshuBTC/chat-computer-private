//! Writes a small model with random weights (the one the engine's own tests use), for Chat Computer's tests that
//! start the engine's program: tests/data/tiny-model.ccm is this, with experts.
//!
//!     cargo run --features std --example tiny_model -- OUT.ccm [dense]
fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().expect("where to write it");
    let dense = args.next().as_deref() == Some("dense");
    std::fs::write(&out, model::testing::tiny(!dense)).expect("writing it");
}
