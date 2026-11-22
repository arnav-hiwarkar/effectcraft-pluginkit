use std::time::Instant;

use effectcraft_engine::render::{Accelerator, FxStep};
use retro_check::engine::{ctx, gpu, params, picture, spec};
use retro_check::{CRT, NEON};

fn main() {
    let size = [1920, 1080];
    let rounds: usize = std::env::args().nth(1).and_then(|r| r.parse().ok()).unwrap_or(10);
    let Some(gpu) = gpu() else {
        println!("gpu: no device");
        return;
    };
    let input = picture(size);
    for (name, id) in [("crt", CRT), ("neon", NEON)] {
        let spec = spec(id);
        let params = params(spec, size, &[]);
        let mut best = f64::MAX;
        for round in 0..=rounds {
            let start = Instant::now();
            gpu.effects(&[FxStep { spec, ctx: ctx(&params, size, round as f64 / 30.0) }], &input, None).expect("the GPU ran the plan");
            if round > 0 {
                best = best.min(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        println!("{name} gpu: best of {rounds}: {best:.2} ms (upload, passes and readback)");
    }
}
