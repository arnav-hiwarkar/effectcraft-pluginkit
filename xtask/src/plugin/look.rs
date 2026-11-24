use std::path::{Path, PathBuf};
use std::time::Duration;

use craft_plugin::{EffectSpec, ParamKind, RunMode, Value};
use craft_plugin_host::Bundle;
use craft_plugin_host::testing::{Pattern, Picture, Render, TestPlugin};

use super::{Flags, Messages, assemble, plugin_dir};

const VALUED: [&str; 9] = ["--effect", "--set", "--in", "--pattern", "--size", "--time", "--scale", "--out", "--rounds"];
const SIZES: [[u32; 2]; 2] = [[1920, 1080], [3840, 2160]];

pub(super) fn value(id: &str, text: &str, kind: &ParamKind) -> Result<Value, String> {
    let numbers = |count: &[usize]| {
        let list = text.split(',').map(|part| part.trim().parse::<f64>()).collect::<Result<Vec<f64>, _>>().ok()?;
        count.contains(&list.len()).then_some(list)
    };
    let wrong = |accepts: String| format!("--set {id}={text}: `{id}` takes {accepts}");
    match kind {
        ParamKind::Slider { .. } | ParamKind::Percent { .. } | ParamKind::Angle { .. } => {
            text.trim().parse().map(Value::Number).map_err(|_| wrong("a number, such as 2.5".into()))
        }
        ParamKind::Checkbox { .. } => match text {
            "true" | "on" => Ok(Value::Bool(true)),
            "false" | "off" => Ok(Value::Bool(false)),
            _ => Err(wrong("true, false, on or off".into())),
        },
        ParamKind::Popup { options, .. } => options
            .iter()
            .position(|option| option.eq_ignore_ascii_case(text))
            .or_else(|| text.parse::<usize>().ok().filter(|index| *index < options.len()))
            .map(|index| Value::Number(index as f64))
            .ok_or_else(|| wrong(format!("one of {} or its number from 0 to {}", options.join(", "), options.len().saturating_sub(1)))),
        ParamKind::Point { .. } => numbers(&[2]).map(Value::List).ok_or_else(|| wrong("a point as two numbers, such as 960,540".into())),
        ParamKind::Point3 { .. } => numbers(&[3]).map(Value::List).ok_or_else(|| wrong("a 3D point as three numbers, such as 960,540,0".into())),
        ParamKind::Color { .. } => numbers(&[3, 4])
            .map(|mut list| {
                list.resize(4, 1.0);
                Value::List(list)
            })
            .ok_or_else(|| wrong("a colour as three or four numbers from 0 to 1, such as 1,0.5,0,1".into())),
        ParamKind::Text { .. } | ParamKind::Custom { .. } | ParamKind::Curve { .. } => Ok(Value::Text(text.to_string())),
        ParamKind::Gradient { .. } => {
            serde_json::from_str(text).map(Value::Gradient).map_err(|_| wrong(r#"a gradient as JSON, such as {"colors":[[0,[0,0,0,1]],[1,[1,1,1,1]]]}"#.into()))
        }
        ParamKind::Layer | ParamKind::Mask => Err(wrong("a layer or a mask, which render fills from the input image; --set cannot change it".into())),
    }
}

fn set(spec: &EffectSpec, pair: &str) -> Result<(String, Value), String> {
    let (id, text) = pair.split_once('=').ok_or(format!("--set {pair}: write it as ID=VALUE, such as radius=20 or tint=1,0.5,0,1"))?;
    let param = spec.params.iter().find(|param| param.id == id).ok_or_else(|| {
        let ids: Vec<&str> = spec.params.iter().map(|param| param.id.as_str()).collect();
        format!("--set {pair}: effect `{}` has no parameter `{id}`; its parameters are {}", spec.id, ids.join(", "))
    })?;
    Ok((id.to_string(), value(id, text, &param.kind)?))
}

fn size(text: &str) -> Result<[u32; 2], String> {
    let (w, h) = text.split_once('x').ok_or(format!("--size {text}: write it as WIDTHxHEIGHT, such as 1920x1080"))?;
    let side = |s: &str| s.parse::<u32>().ok().filter(|n| (1..=16384).contains(n));
    Ok([side(w).ok_or(format!("--size {text}: the width must be 1 to 16384"))?, side(h).ok_or(format!("--size {text}: the height must be 1 to 16384"))?])
}

fn number(flags: &Flags, name: &str, default: f64) -> Result<f64, String> {
    flags.one(name).map_or(Ok(default), |text| text.parse().map_err(|_| format!("{name} {text}: not a number")))
}

fn loaded(root: &Path, dir: &Path, native: bool) -> Result<Vec<(RunMode, TestPlugin)>, String> {
    if !dir.join("Cargo.toml").is_file() {
        let bundle = Bundle::open(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        return Ok(vec![(bundle.mode, TestPlugin::load(&bundle)?)]);
    }
    let out = root.join("plugins/dist");
    let (bundle_dir, _) = assemble(dir, &out, native, Messages::Short)?;
    let bundle = Bundle::open(&bundle_dir).map_err(|e| e.to_string())?;
    let modes = if native { vec![RunMode::Sandboxed, RunMode::Unsandboxed] } else { vec![RunMode::Sandboxed] };
    modes.into_iter().map(|mode| Ok((mode, TestPlugin::load(&bundle.clone().with_mode(mode))?))).collect()
}

fn first_effect(plugin: &TestPlugin) -> Result<String, String> {
    plugin.plugin().contributions().effects.first().map(|spec| spec.id.clone()).ok_or("this plugin has no effects to render".into())
}

pub(super) fn layers_from_source<'a>(plugin: &TestPlugin, effect: &str, render: Render<'a>, source: &Picture) -> Render<'a> {
    let specs = &plugin.plugin().contributions().effects;
    let layers = specs.iter().filter(|spec| spec.id == effect).flat_map(|spec| &spec.params).filter(|param| param.kind == ParamKind::Layer);
    layers.fold(render, |render, param| render.layer(&param.id, source.clone()))
}

pub(super) fn render(root: &Path, args: &[String]) -> Result<(), String> {
    let flags = Flags::parse(args, &VALUED, &["--native"])?;
    let dir = plugin_dir(root, flags.positional);
    let native = flags.has("--native");
    let mut plugins = loaded(root, &dir, native)?;
    let (mode, plugin) = plugins.pop().ok_or("no build to render with")?;
    let effect = flags.one("--effect").map(String::from).map_or_else(|| first_effect(&plugin), Ok)?;
    let mut render = plugin.render(&effect).time(number(&flags, "--time", 0.0)?).scale(number(&flags, "--scale", 1.0)?);
    if let Some(text) = flags.one("--size") {
        let [w, h] = size(text)?;
        render = render.size(w, h);
    }
    if let Some(name) = flags.one("--pattern") {
        render = render.on(Pattern::named(name).ok_or(format!("--pattern {name}: pick one of {}", Pattern::NAMES.join(", ")))?);
    }
    if let Some(path) = flags.one("--in") {
        render = render.input(Picture::open(Path::new(path))?);
    }
    if let Some(spec) = plugin.plugin().contributions().effects.iter().find(|spec| spec.id == effect) {
        for pair in flags.all("--set") {
            let (id, value) = set(spec, pair)?;
            render = render.set(&id, value);
        }
    }
    let source = render.source();
    let render = layers_from_source(&plugin, &effect, render, &source);
    let rounds = number(&flags, "--rounds", 1.0)?.max(1.0) as u32;
    let (picture, took) = render.timed(rounds)?;
    let out = flags.one("--out").map_or_else(|| dir.join("target").join(format!("{effect}.png")), PathBuf::from);
    picture.save(&out)?;
    let kind = if mode == RunMode::Sandboxed { "sandboxed" } else { "native" };
    let best = if rounds > 1 { format!("best of {rounds}: ") } else { String::new() };
    println!("rendered `{effect}` at {}x{} in {best}{:.2} ms ({kind}): {}", picture.width, picture.height, millis(took), out.display());
    Ok(())
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn machine_load() -> String {
    std::process::Command::new("uptime")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.split("load average").nth(1).map(|rest| rest.trim_start_matches(['s', ':', ' ']).trim().to_string()))
        .unwrap_or_else(|| "not measured".into())
}

pub(super) fn bench(root: &Path, args: &[String]) -> Result<(), String> {
    let flags = Flags::parse(args, &["--rounds"], &[])?;
    let dir = plugin_dir(root, flags.positional);
    let rounds = number(&flags, "--rounds", 5.0)?.max(1.0) as u32;
    let plugins = loaded(root, &dir, true)?;
    let [(_, sandboxed), (_, native)] = plugins.as_slice() else { return Err("the sandboxed and the native build are both needed".into()) };
    let effects: Vec<String> = sandboxed.plugin().contributions().effects.iter().map(|spec| spec.id.clone()).collect();
    println!("load average before: {}; best of {rounds} renders on the photo pattern with default parameters", machine_load());
    println!("{:<24} {:>10} {:>12} {:>12} {:>8}", "effect", "size", "sandboxed", "native", "ratio");
    for effect in &effects {
        for [w, h] in SIZES {
            let time = |plugin: &TestPlugin| {
                let render = plugin.render(effect).size(w, h);
                let source = render.source();
                layers_from_source(plugin, effect, render, &source).timed(rounds).map(|(_, took)| took)
            };
            let (wasm, lib) = (time(sandboxed)?, time(native)?);
            let ratio = wasm.as_secs_f64() / lib.as_secs_f64().max(1e-9);
            println!("{effect:<24} {:>10} {:>9.2} ms {:>9.2} ms {ratio:>7.2}x", format!("{w}x{h}"), millis(wasm), millis(lib));
        }
    }
    println!("load average after: {}", machine_load());
    Ok(())
}

#[cfg(test)]
mod tests {
    use craft_plugin::Param;

    use super::*;

    #[test]
    fn a_set_value_is_read_as_the_kind_its_parameter_declares_and_a_wrong_value_names_the_parameter_and_what_it_takes() {
        let spec = EffectSpec::new("card", "Card", "Text").params(vec![
            Param::text("title", "Title", "Hello"),
            Param::checkbox("glow", "Glow", false),
            Param::slider("radius", "Radius", 1.0, 0.0..=10.0),
            Param::color("tint", "Tint", [1.0; 4]),
            Param::popup("mode", "Mode", &["Add", "Screen"], 0),
        ]);
        assert_eq!(set(&spec, "title=on"), Ok(("title".into(), Value::Text("on".into()))));
        assert_eq!(set(&spec, "title=Hello, world").unwrap().1, Value::Text("Hello, world".into()));
        assert_eq!(set(&spec, "glow=on").unwrap().1, Value::Bool(true));
        assert_eq!(set(&spec, "radius=2.5").unwrap().1, Value::Number(2.5));
        assert_eq!(set(&spec, "tint=1,0.5,0").unwrap().1, Value::List(vec![1.0, 0.5, 0.0, 1.0]));
        assert_eq!(set(&spec, "mode=screen").unwrap().1, Value::Number(1.0));
        assert_eq!(set(&spec, "radius=on").unwrap_err(), "--set radius=on: `radius` takes a number, such as 2.5");
        assert_eq!(set(&spec, "glow=1").unwrap_err(), "--set glow=1: `glow` takes true, false, on or off");
        assert_eq!(set(&spec, "mode=2").unwrap_err(), "--set mode=2: `mode` takes one of Add, Screen or its number from 0 to 1");
        assert_eq!(set(&spec, "tint=red").unwrap_err(), "--set tint=red: `tint` takes a colour as three or four numbers from 0 to 1, such as 1,0.5,0,1");
        assert_eq!(
            set(&spec, "radus=2").unwrap_err(),
            "--set radus=2: effect `card` has no parameter `radus`; its parameters are title, glow, radius, tint, mode"
        );
        assert_eq!(size("1920x1080"), Ok([1920, 1080]));
        assert_eq!(size("1920"), Err("--size 1920: write it as WIDTHxHEIGHT, such as 1920x1080".into()));
    }
}
