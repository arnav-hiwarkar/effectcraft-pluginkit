#[path = "../tests/support/build_plugin.rs"]
mod build_plugin;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use effectcraft_engine::Session;
use effectcraft_engine::project::ItemId;
use effectcraft_engine::raster::Image;
use effectcraft_engine::render::{Backend, LayerCache, RenderOpts, Renderer};
use effectcraft_engine::time::Tick;
use effectcraft_gpu::{Gpu, GpuCounters};
use serde_json::{Value, json};

const PLUGIN: &str = "bench.probe";
const SIZE: [u32; 2] = [1920, 1080];
const FPS: f64 = 24.0;
const WARM_FRAMES: u32 = 3;
const TOLERANCE: f32 = 1.0 / 255.0;

struct Case {
    name: &'static str,
    stack: &'static [(&'static str, &'static [(&'static str, f64)])],
}

const CASES: &[Case] = &[
    Case {
        name: "mixed-stack-1080p",
        stack: &[
            ("ec.generate.fractal", &[]),
            ("@gpu-invert", &[("amount", 0.8)]),
            ("@host-kernels", &[("kernel", 14.0)]),
            ("@invert", &[("amount", 0.5)]),
            ("ec.color.levels", &[]),
        ],
    },
    Case { name: "gpu-stack-1080p", stack: &[("@gpu-invert", &[("amount", 0.8)]), ("@host-kernels", &[("kernel", 14.0)]), ("ec.color.levels", &[])] },
];

struct Measured {
    counters: GpuCounters,
    unsteady: Vec<(&'static str, Vec<u64>)>,
    pixels: String,
    worst: f32,
}

struct Args {
    json: Option<PathBuf>,
    check: Option<PathBuf>,
}

fn args() -> Result<Args, String> {
    let mut args = Args { json: None, check: None };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut path = || it.next().map(PathBuf::from).ok_or_else(|| format!("{flag} needs a path"));
        match flag.as_str() {
            "--json" => args.json = Some(path()?),
            "--check" => args.check = Some(path()?),
            other => return Err(format!("unknown argument {other}; usage: gpu_bench [--json out.json] [--check bench/gpu-baseline.json]")),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(why) => {
            eprintln!("gpu bench: {why}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool, String> {
    let args = args()?;
    let gpu = Gpu::try_headless().map_err(|why| format!("no GPU: {why}"))?;
    let config = tempfile::tempdir().map_err(|e| e.to_string())?;
    let bundle = tempfile::tempdir().map_err(|e| e.to_string())?;
    let wasm = build_plugin::wasm(craft_plugin_host::testing::probe_dir(), "probe");
    std::fs::copy(&wasm, bundle.path().join("probe.wasm")).map_err(|e| e.to_string())?;
    let manifest = json!({"id": PLUGIN, "name": "Probe", "version": "1.0.0", "entry": {"wasm": "probe.wasm"}, "permissions": []});
    std::fs::write(bundle.path().join("plugin.json"), manifest.to_string()).map_err(|e| e.to_string())?;

    let mut measured = BTreeMap::new();
    for case in CASES {
        let mut session = effectcraft_host::session();
        let plugins = effectcraft_plugin::install(&mut session, Some(config.path()));
        plugins.load_for_session(bundle.path())?;
        let comp = build(&mut session, case)?;
        let result = measure(&gpu, &session, comp)?;
        for (effect, _) in case.stack {
            if let Some(local) = effect.strip_prefix('@')
                && let Some(why) = plugins.render_error(&format!("{PLUGIN}.{local}"))
            {
                return Err(format!("{}: {local} failed to render: {why}", case.name));
            }
        }
        measured.insert(case.name, result);
    }

    print_table(&measured);
    let out = json!({"cases": measured.iter().map(|(name, m)| (name.to_string(), case_json(m))).collect::<serde_json::Map<_, _>>()});
    let text = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())? + "\n";
    println!("{text}");
    if let Some(path) = &args.json {
        std::fs::write(path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let mut ok = true;
    for (name, m) in &measured {
        if m.worst > TOLERANCE {
            println!("FAIL {name}: the GPU frame differs from the CPU frame by {} (tolerance {TOLERANCE})", m.worst);
            ok = false;
        }
        if m.counters.dispatches == 0 {
            println!("FAIL {name}: no dispatch ran, the frame rendered on the CPU");
            ok = false;
        }
        for (counter, per_frame) in &m.unsteady {
            println!("note {name}: {counter} per warm frame was {per_frame:?}; the highest is reported");
        }
    }
    if let Some(path) = &args.check {
        ok &= check(path, &out)?;
    }
    println!("{}", if ok { "gpu bench: ok" } else { "gpu bench: FAILED" });
    Ok(ok)
}

fn build(session: &mut Session, case: &Case) -> Result<ItemId, String> {
    let [w, h] = SIZE;
    session.execute("comp.new", json!({"name": case.name, "width": w, "height": h, "frameRate": FPS, "duration": 2})).map_err(|e| e.to_string())?;
    session.execute("file.projectSettings", json!({"bitDepth": 8})).map_err(|e| e.to_string())?;
    let layer =
        session.execute("layer.newSolid", json!({"name": "Plate", "color": "#3070c0", "width": w, "height": h})).map_err(|e| e.to_string())?["layer"].clone();
    for (n, (effect, values)) in case.stack.iter().enumerate() {
        let id = match effect.strip_prefix('@') {
            Some(local) => format!("{PLUGIN}.{local}"),
            None => effect.to_string(),
        };
        session.execute("effect.apply", json!({"layer": layer, "effect": id})).map_err(|e| format!("{id}: {e}"))?;
        for (param, value) in *values {
            session
                .execute_checked("prop.set", json!({"layer": layer, "path": format!("effects/#{}/{param}", n + 1), "value": value}))
                .map_err(|e| format!("{id} {param}: {e}"))?;
        }
    }
    let last = case.stack.len();
    for (time, gamma) in [(0.0, 0.6), (1.0, 1.6)] {
        session
            .execute_checked("prop.addKey", json!({"layer": layer, "path": format!("effects/#{last}/gamma"), "time": time, "value": gamma}))
            .map_err(|e| format!("levels gamma key: {e}"))?;
    }
    session.active_comp_id().ok_or_else(|| "no active comp".to_string())
}

fn measure(gpu: &Gpu, session: &Session, comp: ItemId) -> Result<Measured, String> {
    let cache = LayerCache::new(1 << 30);
    let render = |backend, frame: u32, cache: Option<&LayerCache>| {
        let mut renderer = Renderer::new(&session.project, session.footage.as_ref(), RenderOpts { backend, ..Default::default() });
        renderer.cache = cache;
        let tick = Tick::from_seconds_f64(f64::from(frame) / FPS);
        match backend {
            Backend::Cpu => renderer.comp_frame(comp, tick),
            _ => renderer.with_accel(Some(gpu)).comp_frame(comp, tick),
        }
    };
    render(Backend::Gpu, 0, Some(&cache));
    let mut frames: Vec<GpuCounters> = Vec::new();
    let mut digest = Fnv::default();
    let mut worst = 0.0f32;
    for frame in 1..=WARM_FRAMES {
        gpu.context().reset_counters();
        let on_gpu = render(Backend::Gpu, frame, Some(&cache));
        frames.push(gpu.context().counters());
        digest.image(&on_gpu);
        let on_cpu = render(Backend::Cpu, frame, None);
        worst = worst.max(worst_difference(&on_gpu, &on_cpu)?);
    }
    let first = frames.first().copied().unwrap_or_default();
    let unsteady = first
        .fields()
        .into_iter()
        .enumerate()
        .map(|(row, (counter, _))| (counter, frames.iter().map(|f| f.fields().get(row).map_or(0, |field| field.1)).collect::<Vec<_>>()))
        .filter(|(_, per_frame)| per_frame.windows(2).any(|pair| pair[0] != pair[1]))
        .collect();
    let counters = frames.iter().fold(GpuCounters::default(), |most, f| most.max(*f));
    Ok(Measured { counters, unsteady, pixels: format!("{:016x}", digest.0), worst })
}

fn worst_difference(a: &Image, b: &Image) -> Result<f32, String> {
    if (a.width, a.height) != (b.width, b.height) {
        return Err(format!("the GPU frame is {}x{}, the CPU frame {}x{}", a.width, a.height, b.width, b.height));
    }
    Ok(a.data
        .iter()
        .zip(&b.data)
        .flat_map(|(p, q)| p.iter().zip(q).map(|(x, y)| if x.is_nan() || y.is_nan() { f32::INFINITY } else { (x - y).abs() }))
        .fold(0.0, f32::max))
}

#[derive(Default)]
struct Fnv(u64);

impl Fnv {
    fn image(&mut self, img: &Image) {
        if self.0 == 0 {
            self.0 = 0xcbf2_9ce4_8422_2325;
        }
        for byte in img.width.to_le_bytes().into_iter().chain(img.height.to_le_bytes()).chain(img.data.iter().flatten().flat_map(|c| c.to_bits().to_le_bytes()))
        {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

fn case_json(m: &Measured) -> Value {
    let counters: serde_json::Map<String, Value> = m.counters.fields().into_iter().map(|(name, n)| (name.to_string(), json!(n))).collect();
    json!({"counters": counters, "pixels": m.pixels, "worst": m.worst})
}

fn print_table(measured: &BTreeMap<&str, Measured>) {
    let names: Vec<&str> = measured.keys().copied().collect();
    println!("{:<22} {}", "counter per warm frame", names.iter().map(|n| format!("{n:>20}")).collect::<String>());
    for (row, (name, _)) in GpuCounters::default().fields().into_iter().enumerate() {
        let cells: String = measured.values().map(|m| format!("{:>20}", m.counters.fields().get(row).map_or(0, |f| f.1))).collect();
        println!("{name:<22} {cells}");
    }
    let worst: String = measured.values().map(|m| format!("{:>20.6}", m.worst)).collect();
    println!("{:<22} {worst}", "worst vs CPU");
}

fn check(path: &Path, measured: &Value) -> Result<bool, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let baseline: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let empty = serde_json::Map::new();
    let base = baseline["cases"].as_object().unwrap_or(&empty);
    let now = measured["cases"].as_object().unwrap_or(&empty);
    let mut ok = true;
    for name in base.keys().filter(|name| !now.contains_key(*name)) {
        println!("FAIL {name}: in the baseline but not in the suite");
        ok = false;
    }
    for (name, case) in now {
        let Some(expected) = base.get(name) else {
            println!("FAIL {name}: not in the baseline");
            ok = false;
            continue;
        };
        if expected["pending"].as_bool() == Some(true) {
            println!("pending {name}: measured values printed above, not checked");
            continue;
        }
        if expected["pixels"] != case["pixels"] {
            println!("FAIL {name}: pixels {} differ from the baseline {}", case["pixels"], expected["pixels"]);
            ok = false;
        }
        let counters = case["counters"].as_object().unwrap_or(&empty);
        let wanted = expected["counters"].as_object().unwrap_or(&empty);
        for (counter, value) in counters {
            let (now, was) = (value.as_u64().unwrap_or(0), wanted.get(counter).and_then(Value::as_u64));
            match was {
                None => {
                    println!("FAIL {name}: {counter} is not in the baseline");
                    ok = false;
                }
                Some(was) if now > was => {
                    println!("FAIL {name}: {counter} rose from {was} to {now}");
                    ok = false;
                }
                Some(was) if now < was => println!("note {name}: {counter} fell from {was} to {now}; lower the baseline to keep it"),
                Some(_) => {}
            }
        }
    }
    Ok(ok)
}
