mod harness;

use std::path::Path;

use harness::{Kind, Scenario, bundle, camera};

fn save_png(path: &Path, size: [u32; 2], pixels: &[craft_plugin_host::Pixel]) {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let bytes: Vec<u8> = pixels.iter().flat_map(|p| [byte(p[0]), byte(p[1]), byte(p[2]), 255]).collect();
    image::save_buffer(path, &bytes, size[0], size[1], image::ColorType::Rgba8).expect("png saves");
}

fn bench(kind: Kind, label: &str) {
    let scenario = Scenario::new([1920, 1080])
        .param("Emitter/rate", 10000.0)
        .param("Emitter/spread", 40.0)
        .param("Emitter/speed", 700.0)
        .param("Physics/turbulence", 200.0)
        .param("Particle/size", 5.0);
    let bundle = bundle(kind);
    let loaded = scenario.load(&bundle);
    let cold = loaded.timed(90).1;
    println!("{label}: cold frame 90 (catch-up from frame 0): layout {:.1} ms, render {:.1} ms", cold[0], cold[1]);
    let loaded = scenario.load(&bundle);
    (0..80).for_each(|f| {
        loaded.timed(f);
    });
    let times: Vec<_> = (80..100).map(|f| loaded.timed(f).1).collect();
    let mean = |pick: fn(&[f64; 2]) -> f64| times.iter().map(pick).sum::<f64>() / times.len() as f64;
    println!(
        "{label}: 1920x1080, ~20k live particles, frames 80..100 in order: layout {:.1} ms + render {:.1} ms = {:.1} ms per frame",
        mean(|t| t[0]),
        mean(|t| t[1]),
        mean(|t| t[0] + t[1])
    );
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let size = [1280u32, 720];
    let burst = Scenario::new(size)
        .param("Emitter/type", 2.0)
        .list("Emitter/position", &[640.0, 360.0, 0.0])
        .param("Emitter/size_x", 40.0)
        .param("Emitter/size_y", 40.0)
        .param("Emitter/size_z", 40.0)
        .param("Emitter/emission", 1.0)
        .param("Emitter/rate", 20000.0)
        .param("Emitter/spread", 100.0)
        .param("Emitter/speed", 500.0)
        .param("Emitter/speed_random", 60.0)
        .param("Physics/gravity", 60.0)
        .param("Physics/drag", 1.2)
        .param("Particle/life", 1.5)
        .param("Particle/size", 7.0)
        .param("Particle/shape", 1.0)
        .list("Particle/colour_birth", &[0.8, 0.9, 1.0, 1.0])
        .list("Particle/colour_middle", &[0.2, 0.5, 1.0, 1.0])
        .list("Particle/colour_death", &[0.4, 0.05, 0.6, 1.0])
        .scene(camera([1280.0, 720.0], 0.7, 700.0));
    let pixels = burst.load(&bundle(Kind::Wasm)).frame(24);
    let path = Path::new(&out).join("ember-burst-camera.png");
    save_png(&path, size, &pixels);
    println!("wrote {}", path.display());
    bench(Kind::Native, "native");
    bench(Kind::Wasm, "sandboxed");
}
