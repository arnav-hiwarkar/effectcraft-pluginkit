use std::path::Path;
use std::time::Instant;

use craft_plugin::{Pixel, RenderCx, Value, native_target};
use craft_plugin_host::{Bundle, Host, Limits, Plugin};
use serde_json::json;

struct NoHost;
impl Host for NoHost {}

fn load(dir: &Path, entry: serde_json::Value, built: &Path, file: &str) -> Plugin {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::copy(built, dir.join(file)).unwrap();
    let manifest = json!({"id": "bench.probe", "name": "Probe", "version": "1", "entry": entry});
    std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
    let bundle = Bundle::open(dir).unwrap();
    Plugin::load(&bundle, Default::default(), dir.join("data"), Limits::default()).unwrap()
}

fn time(label: &str, mut run: impl FnMut()) {
    run();
    let rounds = 20;
    let start = Instant::now();
    (0..rounds).for_each(|_| run());
    println!("{label:<34} {:>7.2} ms", start.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds));
}

fn main() {
    let probe = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/probe/target");
    let scratch = std::env::temp_dir().join("craft-plugin-bench");
    let native_file = libloading::library_filename("probe").into_string().unwrap();
    let wasm = load(&scratch.join("wasm"), json!({"wasm": "probe.wasm"}), &probe.join("wasm32-unknown-unknown/release/probe.wasm"), "probe.wasm");
    let native = load(&scratch.join("native"), json!({"native": {native_target(): native_file}}), &probe.join("release").join(&native_file), &native_file);
    let (width, height) = (1920u32, 1080u32);
    let mut cx = RenderCx { size: [width, height], scale: 1.0, ..Default::default() };
    cx.params.insert("amount".into(), Value::Number(1.0));
    let mut pixels: Vec<Pixel> = vec![[0.25, 0.5, 0.75, 1.0]; (width * height) as usize];
    println!("invert, {width}x{height} f32 RGBA, {} threads", rayon::current_num_threads());
    time("native plugin, in place", || native.render("invert", &cx, Some(0), &mut pixels, || NoHost).unwrap());
    time("wasm plugin, one instance", || wasm.render("invert", &cx, None, &mut pixels, || NoHost).unwrap());
    time("wasm plugin, parallel bands", || wasm.render("invert", &cx, Some(0), &mut pixels, || NoHost).unwrap());
    time("host loop, one thread", || {
        for pixel in pixels.iter_mut() {
            for channel in &mut pixel[..3] {
                *channel += 1.0 - 2.0 * *channel;
            }
        }
    });
}
