//! NFC of each string in a JSON list, written out as a JSON list (for checking against Python's unicodedata).
use model::json::{self, Json};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let list = json::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
    let out: Vec<Json> = list.array().unwrap().iter().map(|s| Json::Str(model::nfc::nfc(s.str().unwrap()).into_owned())).collect();
    println!("{}", json::compact(&Json::Array(out)));
}
