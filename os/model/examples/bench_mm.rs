//! How fast a weight matrix times many tokens' vectors goes, per kind of arithmetic.
use model::net::{Matrix, Pool, Serial};
use model::quant::{self, Dtype, Prepared};
use model::threads::Threads;
use std::time::Instant;

fn main() {
    let (rows, cols, n) = (3584usize, 1024usize, 64usize);
    let mut seed = 1u64;
    let mut rnd = || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; (seed % 2000) as f32 / 1000.0 - 1.0 };
    let w: Vec<f32> = (0..rows * cols).map(|_| rnd()).collect();
    let xs: Vec<Prepared> = (0..n).map(|_| Prepared::new(&(0..cols).map(|_| rnd()).collect::<Vec<_>>())).collect();
    let threads = Threads::for_this_computer();
    for (dtype, name) in [(Dtype::Q8, "q8"), (Dtype::Q4, "q4")] {
        let mut data = Vec::new();
        if dtype == Dtype::Q8 { quant::quantize_q8(&w, &mut data) } else { quant::quantize_q4(&w, &mut data) }
        let m = Matrix { dtype, rows, cols, data: &data };
        for (pool, pname) in [(&Serial as &dyn Pool, "1 thread"), (&threads as &dyn Pool, "all threads")] {
            for &nn in &[1usize, 4, 16, 64] {
                let mut out = vec![0.0f32; rows * nn];
                let reps = if nn == 1 { 40 } else { 8 };
                let t = Instant::now();
                for _ in 0..reps { m.times_into(&xs[..nn], &mut out, pool); }
                let s = t.elapsed().as_secs_f64() / reps as f64;
                println!("{name} {pname} {:?}: {nn} tokens: {:.2} ms, {:.1} GMAC/s", model::cpu::level(), s * 1e3, (rows * cols * nn) as f64 / s / 1e9);
            }
        }
    }
}
