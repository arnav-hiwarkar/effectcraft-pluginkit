use std::fmt;
use std::sync::Mutex;

use serde::Serialize;

use crate::effect::{Cx, Effect, Frame};
use crate::ui::{Panel, Ui};
use crate::wire::{Call, CommandSpec, Contributions, EffectSpec, HostCall, Outcome, PanelSpec, Pixel, TaskSpec};

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

    pub fn contributions(&self) -> Contributions {
        Contributions {
            effects: self.effects.iter().map(|(spec, _)| spec.clone()).collect(),
            panels: self.panels.iter().map(|(spec, _)| spec.clone()).collect(),
            commands: self.commands.iter().map(|(spec, _)| spec.clone()).collect(),
            tasks: self.tasks.iter().map(|(spec, _)| spec.clone()).collect(),
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
            Call::Command { command } => {
                let (_, run) = self.commands.iter().find(|(spec, _)| spec.id == command).ok_or_else(|| Error(format!("no command `{command}`")))?;
                run()?;
                Ok(serde_json::Value::Null)
            }
            Call::Task { task, args } => {
                let (_, run) = self.tasks.iter().find(|(spec, _)| spec.id == task).ok_or_else(|| Error(format!("no task `{task}`")))?;
                run(&Task { args })
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
