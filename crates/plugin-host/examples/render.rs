use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use craft_plugin::{Call, Layout, MaskPath, Pixel, RenderCx, Scene, Value};
use craft_plugin_host::{Bundle, Host, Limits, Pixels, Plugin};

const USAGE: &str = "usage: render <bundle-dir> <effect-id> <out.png> [--size WxH] [--params JSON] [--masks JSON] \
[--input in.png | --pattern shapes|text|empty] [--time SECONDS] [--scale F] [--over black|none] [--rounds N] [--depth float]

--params  {\"radius\": 40, \"tint\": [1, 0.5, 0, 1], \"mode\": 2, \"on\": true}
--masks   [{\"points\": [[100,100],[500,300]], \"closed\": false}] in frame pixels
The source is also offered as every layer parameter and as the frame at every other time.";

struct Inputs {
    width: u32,
    height: u32,
    source: Vec<Pixel>,
    masks: Vec<MaskPath>,
}

impl Host for &Inputs {
    fn layer(&mut self, _param: &str, _comp_time: Option<f64>) -> Option<Pixels> {
        Some(Pixels { width: self.width, height: self.height, data: self.source.clone() })
    }

    fn source(&mut self, _layer_time: f64) -> Option<Pixels> {
        Some(Pixels { width: self.width, height: self.height, data: self.source.clone() })
    }

    fn masks(&mut self) -> Vec<MaskPath> {
        self.masks.clone()
    }

    fn scene(&mut self) -> Option<Scene> {
        Some(Scene::default())
    }
}

fn shapes(width: u32, height: u32) -> Vec<Pixel> {
    let (w, h) = (width as f32, height as f32);
    let mut out = vec![[0.0; 4]; (width * height) as usize];
    for (i, pixel) in out.iter_mut().enumerate() {
        let (x, y) = ((i as u32 % width) as f32, (i as u32 / width) as f32);
        let disc = ((x - w * 0.3).powi(2) + (y - h * 0.45).powi(2)).sqrt() < h * 0.14;
        let bar = (x - w * 0.62).abs() < w * 0.012 && (y - h * 0.5).abs() < h * 0.3;
        let block = (x - w * 0.8).abs() < w * 0.06 && (y - h * 0.3).abs() < h * 0.08;
        let dim = (x - w * 0.8).abs() < w * 0.06 && (y - h * 0.7).abs() < h * 0.08;
        *pixel = match (disc, bar, block, dim) {
            (true, ..) => [1.0, 1.0, 1.0, 1.0],
            (_, true, ..) => [0.1, 0.6, 1.0, 1.0],
            (_, _, true, _) => [1.0, 0.35, 0.05, 1.0],
            (_, _, _, true) => [0.25, 0.25, 0.25, 1.0],
            _ => [0.0; 4],
        };
    }
    out
}

fn text(width: u32, height: u32) -> Vec<Pixel> {
    let mut out = vec![[0.0; 4]; (width * height) as usize];
    let stroke = (height / 40).max(2) as i64;
    let glyphs: [&[(f32, f32, f32, f32)]; 3] = [
        &[(0.0, 0.0, 0.0, 1.0), (0.0, 0.0, 1.0, 0.0), (0.0, 0.5, 0.8, 0.5)],
        &[(0.0, 0.0, 1.0, 1.0), (1.0, 0.0, 0.0, 1.0)],
        &[(0.0, 0.0, 0.0, 1.0), (0.0, 1.0, 1.0, 1.0), (1.0, 1.0, 1.0, 0.0), (1.0, 0.0, 0.0, 0.0)],
    ];
    let (glyph_w, glyph_h) = (width as f32 * 0.16, height as f32 * 0.4);
    for (n, glyph) in glyphs.iter().enumerate() {
        let (left, top) = (width as f32 * (0.18 + 0.24 * n as f32), height as f32 * 0.3);
        for (x0, y0, x1, y1) in glyph.iter() {
            for step in 0..=400 {
                let t = step as f32 / 400.0;
                let (cx, cy) = (left + (x0 + (x1 - x0) * t) * glyph_w, top + (y0 + (y1 - y0) * t) * glyph_h);
                for dy in -stroke..=stroke {
                    for dx in -stroke..=stroke {
                        let (x, y) = (cx as i64 + dx, cy as i64 + dy);
                        if x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height {
                            out[(y as u32 * width + x as u32) as usize] = [1.0; 4];
                        }
                    }
                }
            }
        }
    }
    out
}

fn load_png(path: &Path) -> Result<(u32, u32, Vec<Pixel>), String> {
    let image = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?.to_rgba8();
    let pixels = image
        .pixels()
        .map(|p| {
            let a = f32::from(p[3]) / 255.0;
            [f32::from(p[0]) / 255.0 * a, f32::from(p[1]) / 255.0 * a, f32::from(p[2]) / 255.0 * a, a]
        })
        .collect();
    Ok((image.width(), image.height(), pixels))
}

fn save_png(path: &Path, width: u32, height: u32, pixels: &[Pixel], over_black: bool) -> Result<(), String> {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let bytes: Vec<u8> = pixels
        .iter()
        .flat_map(|p| {
            if over_black {
                [byte(p[0]), byte(p[1]), byte(p[2]), 255]
            } else {
                let a = p[3].max(1e-6);
                [byte(p[0] / a), byte(p[1] / a), byte(p[2] / a), byte(p[3])]
            }
        })
        .collect();
    image::save_buffer(path, &bytes, width, height, image::ColorType::Rgba8).map_err(|e| e.to_string())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [bundle_dir, effect, out, options @ ..] = args.as_slice() else { return Err(USAGE.into()) };
    let option = |name: &str| options.iter().position(|a| a == name).and_then(|i| options.get(i + 1)).map(String::as_str);
    let (mut width, mut height) = match option("--size").and_then(|s| s.split_once('x')) {
        Some((w, h)) => (w.parse().map_err(|_| USAGE)?, h.parse().map_err(|_| USAGE)?),
        None => (1280u32, 720u32),
    };
    let source = match (option("--input"), option("--pattern").unwrap_or("shapes")) {
        (Some(path), _) => {
            let (w, h, pixels) = load_png(&PathBuf::from(path))?;
            (width, height) = (w, h);
            pixels
        }
        (None, "text") => text(width, height),
        (None, "empty") => vec![[0.0; 4]; (width * height) as usize],
        (None, _) => shapes(width, height),
    };
    let params: BTreeMap<String, Value> = serde_json::from_str(option("--params").unwrap_or("{}")).map_err(|e| format!("--params: {e}"))?;
    let masks: Vec<MaskPath> = serde_json::from_str::<Vec<serde_json::Value>>(option("--masks").unwrap_or("[]"))
        .map_err(|e| format!("--masks: {e}"))?
        .into_iter()
        .map(|mut mask| {
            if let Some(fields) = mask.as_object_mut() {
                fields.entry("name").or_insert("Mask".into());
                fields.entry("closed").or_insert(false.into());
                fields.entry("inverted").or_insert(false.into());
            }
            serde_json::from_value(mask).map_err(|e| format!("--masks: {e}"))
        })
        .collect::<Result<_, _>>()?;
    let number = |name: &str, default: f64| option(name).and_then(|v| v.parse().ok()).unwrap_or(default);
    let scale = number("--scale", 1.0);
    let rounds = number("--rounds", 5.0) as u32;

    let bundle = Bundle::open(Path::new(bundle_dir)).map_err(|e| e.to_string())?;
    let data = std::env::temp_dir().join("craft-plugin-render").join(&bundle.manifest.id);
    let plugin = Plugin::load(&bundle, bundle.manifest.permissions.clone(), data, Limits::default()).map_err(|e| e.to_string())?;
    let spec = plugin
        .contributions()
        .effects
        .iter()
        .find(|e| &e.id == effect)
        .ok_or_else(|| {
            let known: Vec<&str> = plugin.contributions().effects.iter().map(|e| e.id.as_str()).collect();
            format!("no effect `{effect}`; this plugin has {known:?}")
        })?
        .clone();
    let defaults = spec.params.iter().filter_map(|p| Some((p.id.clone(), default_value(&p.kind, width, height)?)));
    let params: BTreeMap<String, Value> = defaults.chain(params).collect();

    let unpadded = Inputs { width, height, source, masks };
    let mut cx = RenderCx {
        params,
        time: number("--time", 0.0),
        comp_time: number("--time", 0.0),
        frame_rate: 30.0,
        scale,
        layer_size: [f64::from(width) / scale, f64::from(height) / scale],
        offset: [0.0, 0.0],
        size: [width, height],
        instance: 1,
        float: option("--depth") == Some("float"),
        linear: false,
    };
    let layout: Layout = plugin.call(&Call::Layout { effect: effect.clone(), cx: cx.clone() }, None, &mut &unpadded).map_err(|e| e.to_string())?;
    let pad = layout.padding;
    let [left, top, right, bottom] = pad;
    let shift = [f64::from(left), f64::from(top)];
    let (padded_w, padded_h) = (width + left + right, height + top + bottom);
    let mut padded = vec![[0.0; 4]; (padded_w * padded_h) as usize];
    for y in 0..height as usize {
        let at = (y + top as usize) * padded_w as usize + left as usize;
        padded[at..at + width as usize].copy_from_slice(&unpadded.source[y * width as usize..(y + 1) * width as usize]);
    }
    let masks = unpadded
        .masks
        .iter()
        .map(|mask| MaskPath { points: mask.points.iter().map(|p| [p[0] + shift[0], p[1] + shift[1]]).collect(), ..mask.clone() })
        .collect();
    let inputs = Inputs { width: padded_w, height: padded_h, source: padded.clone(), masks };
    cx.size = [padded_w, padded_h];
    cx.offset = shift;
    for value in cx.params.values_mut() {
        if let Value::List(point) = value
            && point.len() < 4
        {
            point.iter_mut().zip(shift).for_each(|(c, by)| *c += by);
        }
    }

    let mut best = f64::MAX;
    let mut pixels = Vec::new();
    for _ in 0..rounds.max(1) {
        pixels = padded.clone();
        let start = Instant::now();
        plugin.render(effect, &cx, &layout, None, &mut pixels, || &inputs).map_err(|e| e.to_string())?;
        best = best.min(start.elapsed().as_secs_f64() * 1000.0);
    }
    save_png(Path::new(out), padded_w, padded_h, &pixels, option("--over") != Some("none"))?;
    let kind = if bundle.mode == craft_plugin::RunMode::Sandboxed { "wasm" } else { "native" };
    println!(
        "{effect}: {padded_w}x{padded_h} (padding {pad:?}, reach {:?}, gpu plan {}) best of {rounds}: {best:.2} ms [{kind}] -> {out}",
        layout.reach,
        layout.gpu.is_some()
    );
    Ok(())
}

fn default_value(kind: &craft_plugin::ParamKind, width: u32, height: u32) -> Option<Value> {
    use craft_plugin::ParamKind as K;
    let (w, h) = (f64::from(width), f64::from(height));
    Some(match kind {
        K::Slider { default, .. } | K::Percent { default } | K::Angle { default } => Value::Number(*default),
        K::Checkbox { default } => Value::Bool(*default),
        K::Popup { default, .. } => Value::Number(f64::from(*default)),
        K::Point { default } => Value::List(vec![default[0] * w, default[1] * h]),
        K::Point3 { default } => Value::List(vec![default[0] * w, default[1] * h, default[2]]),
        K::Color { default } => Value::List(default.to_vec()),
        K::Text { default } | K::Custom { default } => Value::Text(default.clone()),
        K::Layer | K::Mask => return None,
    })
}

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
