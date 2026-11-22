use std::fmt;
use std::sync::Mutex;

use serde::Serialize;

use crate::effect::{Cx, Effect, Frame};
use crate::ui::{Panel, Ui};
use crate::wire::{Call, CommandSpec, Contributions, ControlsReply, EffectSpec, Outcome, PanelSpec, Pixel};

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

#[derive(Default)]
pub struct Plugin {
    effects: Vec<(EffectSpec, Box<dyn Effect>)>,
    panels: Vec<(PanelSpec, Mutex<Box<dyn Panel>>)>,
    commands: Vec<(CommandSpec, CommandFn)>,
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

    pub fn contributions(&self) -> Contributions {
        Contributions {
            effects: self.effects.iter().map(|(spec, _)| spec.clone()).collect(),
            panels: self.panels.iter().map(|(spec, _)| spec.clone()).collect(),
            commands: self.commands.iter().map(|(spec, _)| spec.clone()).collect(),
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
                if pixels.len() != width * height {
                    return Err(format!("frame is {width}x{height} but {} pixels were passed", pixels.len()).into());
                }
                let origin = [frame.origin[0] as usize, frame.origin[1] as usize];
                let mut frame = Frame { width, height, origin, pixels };
                self.find_effect(&effect)?.render(&Cx::new(cx), &mut frame)?;
                Ok(serde_json::Value::Null)
            }
            Call::Panel { panel, input } => {
                let (_, state) = self.panels.iter().find(|(spec, _)| spec.id == panel).ok_or_else(|| Error(format!("no panel `{panel}`")))?;
                let mut ui = Ui::new(&input);
                state.lock().map_err(|_| Error("the panel panicked earlier".into()))?.ui(&mut ui);
                json(&ui.finish())
            }
            Call::Controls { effect, param, value, input } => {
                let mut edited = value.clone();
                let mut ui = Ui::new(&input);
                self.find_effect(&effect)?.controls(&param, &mut edited, &mut ui);
                json(&ControlsReply { widgets: ui.finish(), value: (edited != value).then_some(edited) })
            }
            Call::Command { command } => {
                let (_, run) = self.commands.iter().find(|(spec, _)| spec.id == command).ok_or_else(|| Error(format!("no command `{command}`")))?;
                run()?;
                Ok(serde_json::Value::Null)
            }
        }
    }

    fn find_effect(&self, id: &str) -> Result<&dyn Effect> {
        self.effects.iter().find(|(spec, _)| spec.id == id).map(|(_, effect)| effect.as_ref()).ok_or_else(|| Error(format!("no effect `{id}`")))
    }
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

        fn controls(&self, param: &str, value: &mut String, ui: &mut Ui) {
            if ui.button(param) {
                *value = if value == "off" { "on".into() } else { "off".into() };
            }
        }
    }

    fn controls(plugin: &Plugin, value: &str, events: Vec<UiEvent>) -> ControlsReply {
        let input = UiInput { events, ..Default::default() };
        let call = Call::Controls { effect: "toggle".into(), param: "state".into(), value: value.into(), input };
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
