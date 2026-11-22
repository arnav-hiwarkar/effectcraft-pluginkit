use std::fmt;
use std::sync::Mutex;

use serde::Serialize;

use crate::effect::{Cx, Effect, Frame, Pass, Rows};
use crate::ui::{Panel, Ui};
use crate::viewer::{Overlay, Space, Tool, ToolSpec};
use crate::wire::{Call, CommandSpec, Contributions, ControlsReply, EffectSpec, HostCall, Outcome, PanelSpec, Pixel, TaskSpec, Window};

#[derive(Debug, Clone, PartialEq)]
pub struct Error(pub String);

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Error(message)
    }
}

impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Error(message.into())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Error(error.to_string())
    }
}

type CommandFn = Box<dyn Fn() -> Result<()> + Send + Sync>;
type TaskFn = Box<dyn Fn(&Task) -> Result<serde_json::Value> + Send + Sync>;

pub struct Task {
    args: serde_json::Value,
}

impl Task {
    pub fn args(&self) -> &serde_json::Value {
        &self.args
    }

    pub fn progress(&self, fraction: f64, message: &str) -> Result<()> {
        let call = HostCall::Progress { fraction, message: message.into() };
        crate::host::call::<serde_json::Value>(&call, &[]).map(|_| ())
    }
}

#[derive(Default)]
pub struct Plugin {
    effects: Vec<(EffectSpec, Box<dyn Effect>)>,
    panels: Vec<(PanelSpec, Mutex<Box<dyn Panel>>)>,
    commands: Vec<(CommandSpec, CommandFn)>,
    tasks: Vec<(TaskSpec, TaskFn)>,
    tools: Vec<(ToolSpec, Mutex<Box<dyn Tool>>)>,
}

impl Plugin {
    pub fn new() -> Self {
        Plugin::default()
    }

    pub fn effect(mut self, effect: impl Effect) -> Self {
        self.effects.push((effect.spec(), Box::new(effect)));
        self
    }

    pub fn panel(mut self, id: &str, title: &str, panel: impl Panel) -> Self {
        let spec = PanelSpec { id: id.into(), title: title.into() };
        self.panels.push((spec, Mutex::new(Box::new(panel))));
        self
    }

    pub fn command(mut self, id: &str, title: &str, run: impl Fn() -> Result<()> + Send + Sync + 'static) -> Self {
        self.commands.push((CommandSpec { id: id.into(), title: title.into() }, Box::new(run)));
        self
    }

    pub fn task(mut self, id: &str, title: &str, run: impl Fn(&Task) -> Result<serde_json::Value> + Send + Sync + 'static) -> Self {
        self.tasks.push((TaskSpec { id: id.into(), title: title.into() }, Box::new(run)));
        self
    }

    pub fn tool(mut self, id: &str, name: &str, tool: impl Tool) -> Self {
        let spec = ToolSpec { id: id.into(), name: name.into(), icon: tool.icon(), cursor: tool.cursor() };
        self.tools.push((spec, Mutex::new(Box::new(tool))));
        self
    }

    pub fn contributions(&self) -> Contributions {
        Contributions {
            effects: self.effects.iter().map(|(spec, _)| spec.clone()).collect(),
            panels: self.panels.iter().map(|(spec, _)| spec.clone()).collect(),
            commands: self.commands.iter().map(|(spec, _)| spec.clone()).collect(),
            tasks: self.tasks.iter().map(|(spec, _)| spec.clone()).collect(),
            tools: self.tools.iter().map(|(spec, _)| spec.clone()).collect(),
            separate_input: true,
        }
    }

    pub fn dispatch(&self, request: &[u8], pixels: &mut [Pixel]) -> Vec<u8> {
        let outcome: Outcome<serde_json::Value> =
            serde_json::from_slice::<Call>(request).map_err(Error::from).and_then(|call| self.run(call, pixels)).map_err(|e| e.0).into();
        serde_json::to_vec(&outcome).unwrap_or_default()
    }

    fn run(&self, call: Call, pixels: &mut [Pixel]) -> Result<serde_json::Value> {
        match call {
            Call::Describe => json(&self.contributions()),
            Call::Layout { effect, cx } => json(&self.find_effect(&effect)?.layout(&Cx::new(cx))),
            Call::Render { effect, cx, frame } => {
                let (width, height) = (frame.width as usize, frame.height as usize);
                let origin = [frame.origin[0] as usize, frame.origin[1] as usize];
                let snapshot;
                let (read, window, out) = match frame.input {
                    None => {
                        snapshot = pixels.to_vec();
                        let whole = Window { size: [frame.width, frame.origin[1] + frame.height], first_row: frame.origin[1], rows: frame.height };
                        (&snapshot[..], whole, pixels)
                    }
                    Some(window) if frame.spread => {
                        let (read, _, out) = crate::abi::spread(pixels).ok_or("the frame addresses are missing")?;
                        (read, window, out)
                    }
                    Some(window) => {
                        let (read, out) = pixels.split_at_mut_checked(window.pixels()).ok_or("the input rows do not fit in the pixels passed")?;
                        (&*read, window, out)
                    }
                };
                let inside = origin[0] + width <= window.size[0] as usize && read.len() == window.pixels();
                if out.len() != width * height || !inside {
                    return Err(format!("frame is {width}x{height} at {origin:?} but {} pixels were passed", out.len()).into());
                }
                let input = rows(window, read);
                let mut frame = Frame { width, height, origin, pixels: out, input };
                self.find_effect(&effect)?.render(&Cx::new(cx), &mut frame)?;
                Ok(serde_json::Value::Null)
            }
            Call::Input { effect, cx, out } => json(&self.find_effect(&effect)?.input(&Cx::new(cx), out)),
            Call::Pass { effect, cx, kernel, f, u, src, aux, out, spread } => {
                let (src_len, aux_len) = (src.pixels(), aux.map_or(0, |aux| aux.pixels()));
                let parts = if spread {
                    crate::abi::spread(pixels)
                } else {
                    pixels.split_at_mut_checked(src_len + aux_len).and_then(|(read, band)| {
                        let (src, aux) = read.split_at_checked(src_len)?;
                        Some((src, aux, band))
                    })
                };
                let fits = |(src, aux, band): &(&[Pixel], &[Pixel], &mut [Pixel])| src.len() == src_len && aux.len() == aux_len && band.len() == out.pixels();
                let (src_pixels, aux_pixels, band) = parts.filter(fits).ok_or("the pass images do not match the pixels passed")?;
                let pass = Pass {
                    kernel: &kernel,
                    f: &f,
                    u: &u,
                    src: rows(src, src_pixels),
                    aux: aux.map(|aux| rows(aux, aux_pixels)),
                    out_size: out.size.map(|side| side as usize),
                };
                let input = rows(Window { size: out.size, first_row: 0, rows: 0 }, &[]);
                let mut frame = Frame { width: out.size[0] as usize, height: out.rows as usize, origin: [0, out.first_row as usize], pixels: band, input };
                self.find_effect(&effect)?.pass(&Cx::new(cx), &pass, &mut frame)?;
                Ok(serde_json::Value::Null)
            }
            Call::Panel { panel, input } => {
                let (_, state) = self.panels.iter().find(|(spec, _)| spec.id == panel).ok_or_else(|| Error(format!("no panel `{panel}`")))?;
                let mut ui = Ui::new(&input);
                state.lock().map_err(|_| Error("the panel panicked earlier; reload the plugin to start it again".into()))?.ui(&mut ui);
                json(&ui.reply())
            }
            Call::Controls { effect, param, instance, value, input } => {
                let mut edited = value.clone();
                let mut ui = Ui::new(&input);
                self.find_effect(&effect)?.controls(instance, &param, &mut edited, &mut ui);
                json(&ControlsReply { widgets: ui.finish(), value: (edited != value).then_some(edited) })
            }
            Call::Command { command } => {
                let (_, run) = self.commands.iter().find(|(spec, _)| spec.id == command).ok_or_else(|| Error(format!("no command `{command}`")))?;
                run()?;
                Ok(serde_json::Value::Null)
            }
            Call::Task { task, args } => {
                let (_, run) = self.tasks.iter().find(|(spec, _)| spec.id == task).ok_or_else(|| Error(format!("no task `{task}`")))?;
                run(&Task { args })
            }
            Call::Overlay { effect, cx } => {
                let mut overlay = Overlay::new();
                self.find_effect(&effect)?.overlay(&Cx::new(cx), &mut overlay);
                json(&overlay)
            }
            Call::Drag { effect, cx, key, to, value } => {
                let mut edited = value.clone();
                self.find_effect(&effect)?.drag(&Cx::new(cx), &key, to, &mut edited);
                json(&(edited != value).then_some(edited))
            }
            Call::Tool { tool, event } => {
                let (_, state) = self.tools.iter().find(|(spec, _)| spec.id == tool).ok_or_else(|| Error(format!("no tool `{tool}`")))?;
                let mut overlay = Overlay::in_space(Space::Comp);
                state.lock().map_err(|_| Error("the tool panicked earlier; reload the plugin to start it again".into()))?.event(&event, &mut overlay)?;
                json(&overlay)
            }
        }
    }

    fn find_effect(&self, id: &str) -> Result<&dyn Effect> {
        self.effects.iter().find(|(spec, _)| spec.id == id).map(|(_, effect)| effect.as_ref()).ok_or_else(|| {
            let known: Vec<&str> = self.effects.iter().map(|(spec, _)| spec.id.as_str()).collect();
            Error(format!("no effect `{id}`; this plugin has {known:?}"))
        })
    }
}

fn rows(window: Window, pixels: &[Pixel]) -> Rows<'_> {
    Rows { width: window.size[0] as usize, height: window.size[1] as usize, first_row: window.first_row as usize, pixels }
}

fn json(value: &impl Serialize) -> Result<serde_json::Value> {
    Ok(serde_json::to_value(value)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Param, ParamKind, UiEvent, UiEventKind, UiInput, Widget};

    struct Toggle;

    impl Effect for Toggle {
        fn spec(&self) -> EffectSpec {
            EffectSpec::new("toggle", "Toggle", "Test").param(Param::custom("state", "State", "off"))
        }

        fn controls(&self, _instance: u64, param: &str, value: &mut String, ui: &mut Ui) {
            if ui.button(param) {
                *value = if value == "off" { "on".into() } else { "off".into() };
            }
        }
    }

    fn controls(plugin: &Plugin, value: &str, events: Vec<UiEvent>) -> ControlsReply {
        let input = UiInput { events, ..Default::default() };
        let call = Call::Controls { effect: "toggle".into(), param: "state".into(), instance: 1, value: value.into(), input };
        let reply = plugin.dispatch(&serde_json::to_vec(&call).unwrap_or_default(), &mut []);
        match serde_json::from_slice::<Outcome<ControlsReply>>(&reply) {
            Ok(Outcome::Ok(reply)) => reply,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_custom_param_declares_its_default_text_and_its_control_returns_the_new_value_only_when_it_changes() {
        let plugin = Plugin::new().effect(Toggle);
        assert_eq!(plugin.contributions().effects[0].params[0].kind, ParamKind::Custom { default: "off".into() });
        let idle = controls(&plugin, "off", vec![]);
        assert_eq!(idle.value, None);
        let Widget::Button { id, .. } = idle.widgets[0] else { panic!("button expected") };
        let clicked = controls(&plugin, "off", vec![UiEvent { id, kind: UiEventKind::Click }]);
        assert_eq!(clicked.value.as_deref(), Some("on"));
    }
}
