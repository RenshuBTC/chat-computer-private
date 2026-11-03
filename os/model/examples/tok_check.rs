//! Each string in a JSON list as tokens (no special tokens), written out as a JSON list of lists, for checking
//! against the models' own tokenizer.
use model::json::{self, Json};
use model::{File, load_tokenizer};
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bytes = std::fs::read(&args[0]).unwrap();
    let file = File::open(&bytes).unwrap();
    let tok = load_tokenizer(&file).unwrap();
    let list = json::parse(&std::fs::read_to_string(&args[1]).unwrap()).unwrap();
    let mut out = Vec::new();
    for s in list.array().unwrap() {
        let mut ids = Vec::new();
        tok.encode(s.str().unwrap(), &mut ids);
        out.push(Json::Array(ids.into_iter().map(|i| Json::Number(i as f64)).collect()));
    }
    println!("{}", json::compact(&Json::Array(out)));
}
