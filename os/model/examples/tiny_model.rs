//! Writes a small model with random weights (the one the engine's own tests use), for Chat Computer's tests that
//! start the engine's program: tests/data/tiny-model.ccm is this, with experts; tests/data/tiny-seeing-model.ccm,
//! with experts and a small vision part (for the own system's check that its model sees pictures).
//!
//!     cargo run --features std --example tiny_model -- OUT.ccm [dense] [seeing]
fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().expect("where to write it");
    let rest: Vec<String> = args.collect();
    let experts = !rest.iter().any(|a| a == "dense");
    let file = if rest.iter().any(|a| a == "seeing") { model::testing::tiny_seeing(experts) } else { model::testing::tiny(experts) };
    std::fs::write(&out, file).expect("writing it");
}
