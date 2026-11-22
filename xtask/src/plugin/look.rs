use std::path::{Path, PathBuf};
use std::time::Duration;

use craft_plugin::{ParamKind, RunMode, Value};
use craft_plugin_host::Bundle;
use craft_plugin_host::testing::{Pattern, Picture, Render, TestPlugin};

use super::{Messages, USAGE, assemble, option, options, plugin_dir, positional};

const VALUED: [&str; 10] = ["--effect", "--set", "--in", "--pattern", "--size", "--time", "--scale", "--out", "--rounds", "--bridge"];
const SIZES: [[u32; 2]; 2] = [[1920, 1080], [3840, 2160]];

pub(super) fn value(text: &str) -> Value {
    match text {
        "true" | "on" => return Value::Bool(true),
        "false" | "off" => return Value::Bool(false),
        _ => {}
    }
    if let Ok(number) = text.parse() {
        return Value::Number(number);
    }
    match text.split(',').map(|part| part.trim().parse()).collect::<Result<Vec<f64>, _>>() {
        Ok(list) if list.len() > 1 => Value::List(list),
        _ => Value::Text(text.to_string()),
    }
}

fn size(text: &str) -> Result<[u32; 2], String> {
    let (w, h) = text.split_once('x').ok_or(format!("--size {text}: write it as WIDTHxHEIGHT, such as 1920x1080"))?;
    let side = |s: &str| s.parse::<u32>().ok().filter(|n| (1..=16384).contains(n));
    Ok([side(w).ok_or(format!("--size {text}: the width must be 1 to 16384"))?, side(h).ok_or(format!("--size {text}: the height must be 1 to 16384"))?])
}

fn number(args: &[String], name: &str, default: f64) -> Result<f64, String> {
    option(args, name).map_or(Ok(default), |text| text.parse().map_err(|_| format!("{name} {text}: not a number")))
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

fn layers_from_source<'a>(plugin: &TestPlugin, effect: &str, render: Render<'a>, source: &Picture) -> Render<'a> {
    let specs = &plugin.plugin().contributions().effects;
    let layers = specs.iter().filter(|spec| spec.id == effect).flat_map(|spec| &spec.params).filter(|param| param.kind == ParamKind::Layer);
    layers.fold(render, |render, param| render.layer(&param.id, source.clone()))
}

pub(super) fn render(root: &Path, args: &[String]) -> Result<(), String> {
    let dir = plugin_dir(root, positional(args, &VALUED).ok_or(USAGE)?);
    let native = args.iter().any(|a| a == "--native");
    let mut plugins = loaded(root, &dir, native)?;
    let (mode, plugin) = plugins.pop().ok_or("no build to render with")?;
    let effect = option(args, "--effect").map(String::from).map_or_else(|| first_effect(&plugin), Ok)?;
    let mut render = plugin.render(&effect).time(number(args, "--time", 0.0)?).scale(number(args, "--scale", 1.0)?);
    if let Some(text) = option(args, "--size") {
        let [w, h] = size(text)?;
        render = render.size(w, h);
    }
    if let Some(name) = option(args, "--pattern") {
        render = render.on(Pattern::named(name).ok_or(format!("--pattern {name}: pick one of {}", Pattern::NAMES.join(", ")))?);
    }
    if let Some(path) = option(args, "--in") {
        render = render.input(Picture::open(Path::new(path))?);
    }
    for pair in options(args, "--set") {
        let (id, text) = pair.split_once('=').ok_or(format!("--set {pair}: write it as ID=VALUE, such as radius=20 or tint=1,0.5,0,1"))?;
        render = render.set(id, value(text));
    }
    let source = render.source();
    let render = layers_from_source(&plugin, &effect, render, &source);
    let rounds = number(args, "--rounds", 1.0)?.max(1.0) as u32;
    let (picture, took) = render.timed(rounds)?;
    let out = option(args, "--out").map_or_else(|| dir.join("target").join(format!("{effect}.png")), PathBuf::from);
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
    let dir = plugin_dir(root, positional(args, &VALUED).ok_or(USAGE)?);
    let rounds = number(args, "--rounds", 5.0)?.max(1.0) as u32;
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
    use super::*;

    #[test]
    fn a_set_value_is_a_bool_a_number_a_list_or_text() {
        assert_eq!(value("on"), Value::Bool(true));
        assert_eq!(value("2.5"), Value::Number(2.5));
        assert_eq!(value("1, 0.5,0,1"), Value::List(vec![1.0, 0.5, 0.0, 1.0]));
        assert_eq!(value("Hello, world"), Value::Text("Hello, world".into()));
        assert_eq!(size("1920x1080"), Ok([1920, 1080]));
        assert_eq!(size("1920"), Err("--size 1920: write it as WIDTHxHEIGHT, such as 1920x1080".into()));
    }
}
