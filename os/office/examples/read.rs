//! Reads Office files and prints their words, as Chat Computer's model is given them:
//! `cargo run -p office --example read -- letter.doc budget.xlsx`.

fn main() {
    for path in std::env::args().skip(1) {
        let data = std::fs::read(&path).unwrap();
        match office::read(&data) {
            Ok((kind, text)) => println!("=== {path}: {kind:?}\n{text}\n"),
            Err(e) => println!("=== {path}: error {e:?}\n"),
        }
    }
}
