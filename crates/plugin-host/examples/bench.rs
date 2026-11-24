use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use craft_plugin::{Call, Layout, ParamKind, Pixel, RenderCx, Value, native_target};
use craft_plugin_host::{Bundle, Host, Limits, Plugin};
use serde_json::json;

struct NoHost;
impl Host for NoHost {}

struct Subject {
    name: &'static str,
    dir: PathBuf,
    library: &'static str,
    effect: &'static str,
}

fn built(subject: &Subject, wasm: bool) -> PathBuf {
    let release = subject.dir.join("target");
    if wasm {
        release.join("wasm32-unknown-unknown/release").join(format!("{}.wasm", subject.library))
    } else {
        release.join("release").join(libloading::library_filename(subject.library))
    }
}

fn load(scratch: &Path, subject: &Subject, wasm: bool) -> Plugin {
    let dir = scratch.join(subject.library).join(if wasm { "wasm" } else { "native" });
    std::fs::create_dir_all(&dir).unwrap();
    let from = built(subject, wasm);
    let file = from.file_name().unwrap().to_str().unwrap().to_string();
    let _ = std::fs::remove_file(dir.join(&file));
    std::fs::copy(&from, dir.join(&file)).unwrap_or_else(|e| panic!("{}: {e}", from.display()));
    let entry = if wasm { json!({"wasm": file}) } else { json!({"native": {native_target(): file}}) };
    let manifest = json!({"id": format!("bench.{}", subject.library.replace('_', "-")), "name": subject.name, "version": "1", "entry": entry});
    std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
    let bundle = Bundle::open(&dir).unwrap();
    Plugin::load(&bundle, Default::default(), dir.join("data"), Limits::default()).unwrap()
}

fn defaults(plugin: &Plugin, effect: &str, size: [u32; 2]) -> BTreeMap<String, Value> {
    let spec = plugin.contributions().effects.iter().find(|e| e.id == effect).unwrap();
    spec.params
        .iter()
        .filter_map(|p| {
            let value = match &p.kind {
                ParamKind::Slider { default, .. } | ParamKind::Percent { default, .. } | ParamKind::Angle { default } => Value::Number(*default),
                ParamKind::Checkbox { default } => Value::Bool(*default),
                ParamKind::Popup { default, .. } => Value::Number(f64::from(*default)),
                ParamKind::Point { default } => Value::List(vec![default[0] * f64::from(size[0]), default[1] * f64::from(size[1])]),
                ParamKind::Point3 { default } => Value::List(default.to_vec()),
                ParamKind::Color { default } => Value::List(default.to_vec()),
                ParamKind::Text { default } | ParamKind::Custom { default } => Value::Text(default.clone()),
                ParamKind::Gradient { default } => Value::Gradient(default.clone()),
                ParamKind::Curve { default, channels } => Value::Text(craft_plugin_host::default_curves_text(default, *channels)),
                ParamKind::Layer | ParamKind::Mask => return None,
            };
            Some((p.id.clone(), value))
        })
        .collect()
}

fn source(size: [u32; 2]) -> Vec<Pixel> {
    let (width, height) = (size[0] as usize, size[1] as usize);
    (0..width * height)
        .map(|i| {
            let (x, y) = ((i % width) as f32 / width as f32, (i / width) as f32 / height as f32);
            let spot = if (x * 37.0).fract() < 0.05 && (y * 23.0).fract() < 0.05 { 4.0 } else { 0.0 };
            [x + spot, y + spot, 0.5 * (x + y), 1.0]
        })
        .collect()
}

fn median_ms(rounds: usize, mut reset: impl FnMut(), mut run: impl FnMut()) -> [f64; 2] {
    reset();
    run();
    let mut times: Vec<f64> = (0..rounds)
        .map(|_| {
            reset();
            let start = Instant::now();
            run();
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(f64::total_cmp);
    [times[rounds / 2], times[0]]
}

fn padded(pixels: &[Pixel], size: [u32; 2], padding: [u32; 4]) -> Vec<Pixel> {
    let [left, top, right, bottom] = padding.map(|side| side as usize);
    let width = size[0] as usize;
    let wide = left + width + right;
    let mut out = vec![[0.0; 4]; wide * (top + size[1] as usize + bottom)];
    for (y, row) in pixels.chunks_exact(width).enumerate() {
        let at = (y + top) * wide + left;
        out[at..at + width].copy_from_slice(row);
    }
    out
}

fn render(plugin: &Plugin, effect: &str, size: [u32; 2], rounds: usize) -> ([f64; 2], Vec<Pixel>) {
    let mut cx = RenderCx { size, scale: 1.0, time: 1.25, frame_rate: 24.0, layer_size: [f64::from(size[0]), f64::from(size[1])], ..Default::default() };
    cx.params = defaults(plugin, effect, size);
    let layout: Layout = plugin.call(&Call::Layout { effect: effect.into(), cx: cx.clone() }, None, &mut NoHost).unwrap();
    let original = padded(&source(size), size, layout.padding);
    let [left, top, right, bottom] = layout.padding;
    cx.size = [size[0] + left + right, size[1] + top + bottom];
    cx.offset = [f64::from(left), f64::from(top)];
    let pixels = std::cell::RefCell::new(original.clone());
    let ms = median_ms(
        rounds,
        || pixels.borrow_mut().copy_from_slice(&original),
        || match layout.gpu.as_ref().filter(|plan| layout.cpu && plan.runs_on_cpu()) {
            Some(plan) => plugin.run_passes(effect, &cx, plan, &mut pixels.borrow_mut(), || NoHost).unwrap(),
            None => plugin.render(effect, &cx, &layout, None, &mut pixels.borrow_mut(), || NoHost).map(|_| ()).unwrap(),
        },
    );
    (ms, pixels.into_inner())
}

fn digest(pixels: &[Pixel]) -> u64 {
    pixels.iter().flatten().fold(0xcbf2_9ce4_8422_2325, |hash, v| (hash ^ u64::from(v.to_bits())).wrapping_mul(0x100_0000_01b3))
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let plugins = root.join("../../plugins");
    let subjects = [
        Subject { name: "Probe invert", dir: root.join("tests/probe"), library: "probe", effect: "invert" },
        Subject { name: "Plasma Beam", dir: plugins.join("plasma-beam"), library: "plasma_beam", effect: "beam" },
        Subject { name: "Lumen Glow CPU", dir: plugins.join("lumen-glow"), library: "lumen_glow", effect: "glow" },
        Subject { name: "Halo Flares", dir: plugins.join("halo"), library: "halo", effect: "flare" },
        Subject { name: "Retro CRT", dir: plugins.join("retro"), library: "retro", effect: "crt" },
        Subject { name: "Retro Neon", dir: plugins.join("retro"), library: "retro", effect: "neon" },
    ];
    let only: Vec<String> = std::env::args().skip(1).collect();
    let scratch = std::env::temp_dir().join("craft-plugin-bench");
    let rounds = 15;
    println!("f32 RGBA, {} rayon threads, median and minimum of {rounds}", rayon::current_num_threads());
    println!(
        "{:<16} {:>9} {:>11} {:>11} {:>7} {:>11} {:>11} {:>7} {:>11} {:>17}",
        "effect", "size", "native ms", "wasm ms", "ratio", "native min", "wasm min", "ratio", "load ms", "wasm digest"
    );
    for subject in subjects.iter().filter(|s| only.is_empty() || only.iter().any(|o| s.library.contains(o.as_str()))) {
        let native = load(&scratch, subject, false);
        let start = Instant::now();
        let wasm = load(&scratch, subject, true);
        let load_ms = start.elapsed().as_secs_f64() * 1000.0;
        for size in [[1920, 1080], [3840, 2160]] {
            let (native_ms, native_out) = render(&native, subject.effect, size, rounds);
            let (wasm_ms, wasm_out) = render(&wasm, subject.effect, size, rounds);
            let same = if native_out == wasm_out { "same as native" } else { "differs from native" };
            println!(
                "{:<16} {:>9} {:>11.2} {:>11.2} {:>7.2} {:>11.2} {:>11.2} {:>7.2} {load_ms:>11.1} {:>17x} {same}",
                subject.name,
                format!("{}x{}", size[0], size[1]),
                native_ms[0],
                wasm_ms[0],
                wasm_ms[0] / native_ms[0],
                native_ms[1],
                wasm_ms[1],
                wasm_ms[1] / native_ms[1],
                digest(&wasm_out)
            );
        }
    }
}
