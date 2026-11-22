use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use craft_plugin_host::{Access, AudioRequest, FrameRequest, Host, Permission, PickFile, Pixels, Samples};
use effectcraft_automation::confine::{self, Confinement, Lookup};
use effectcraft_engine::project::{ItemId, LayerId, Project};
use effectcraft_engine::render::{RenderOpts, Renderer, audio};
use effectcraft_engine::time::Tick;
use effectcraft_engine::{Event, Session, commands};
use serde_json::{Value, json};

use crate::manage::Loaded;

const DOCUMENT_FAMILIES: &[&str] = &[
    "anim",
    "camera",
    "command",
    "comp",
    "editor",
    "effect",
    "engine",
    "essential",
    "expr",
    "item",
    "keys",
    "layer",
    "light",
    "markers",
    "mask",
    "material",
    "motion",
    "path",
    "paths",
    "project",
    "prop",
    "puppet",
    "shape",
    "text",
    "time",
    "timeline",
];

const FILE_FAMILIES: &[&str] = &["file"];

const REFUSED: &[&str] = &[
    "anim.browsePresets",
    "anim.clearRecentPresets",
    "comp.copyFrame",
    "comp.deletePreset",
    "comp.flowchart",
    "comp.miniFlowchart",
    "comp.openInEssentialGraphics",
    "comp.savePreset",
    "effect.manage",
    "layer.openLayer",
    "layer.revealInFinder",
    "layer.style.options",
    "project.deleteWithoutConfirmation",
    "prop.editGradient",
];

const WRITES_A_PROPERTY: &[&str] = &["prop.set", "prop.setExpression", "prop.pickWhip", "prop.addKey", "keys.set"];

const MAX_NOTICE: usize = 300;

const FILE_PERMISSIONS: [Permission; 2] = [Permission::FilesRead, Permission::FilesWrite];

const MAX_AUDIO_FRAMES: f64 = (1u64 << 25) as f64;
const AUDIO_RATES: std::ops::RangeInclusive<u32> = 1000..=192_000;
const MAX_FRAME_SCALE: f64 = 4.0;

pub type FilePicker = Box<dyn Fn(&PickFile) -> Option<PathBuf> + Send + Sync>;

static FILE_PICKER: OnceLock<FilePicker> = OnceLock::new();

pub fn set_file_picker(picker: FilePicker) {
    let _ = FILE_PICKER.set(picker);
}

fn find_comp(session: &Session, name: Option<&str>) -> Result<ItemId, String> {
    let Some(name) = name else { return session.active_comp_id().ok_or_else(|| "there is no active composition".to_string()) };
    let by_name = session.project.items.values().find(|item| item.name == name && item.as_comp().is_some()).map(|item| item.id);
    by_name
        .or_else(|| name.parse().ok().map(ItemId).filter(|id| session.project.comp(*id).is_some()))
        .ok_or_else(|| format!("no composition `{name}`"))
}

fn find_layer(project: &Project, comp: ItemId, name: &str) -> Result<LayerId, String> {
    let comp = project.comp(comp).ok_or("the composition is gone")?;
    let by_name = comp.layers.iter().find(|layer| layer.name == name).map(|layer| layer.id);
    by_name.or_else(|| name.parse().ok().map(LayerId).filter(|id| comp.layer(*id).is_some())).ok_or_else(|| format!("no layer `{name}`"))
}

fn only_layer(project: &Project, comp: ItemId, layer: LayerId, video: bool) -> Result<Project, String> {
    let mut solo = project.clone();
    let comp = solo.comp_mut(comp).ok_or("the composition is gone")?;
    for other in comp.layers.iter_mut() {
        let keep = other.id == layer;
        if video {
            other.switches.video &= keep;
        } else {
            other.switches.audio &= keep;
        }
    }
    Ok(solo)
}

pub(crate) struct SessionHost<'a> {
    session: &'a mut Session,
    plugin: &'a Loaded,
}

fn steps(batch: &Value) -> Result<Vec<(&str, &Value)>, String> {
    let steps = batch.get("steps").and_then(Value::as_array).ok_or("`engine.batch` needs a `steps` array")?;
    steps
        .iter()
        .map(|step| {
            let id = step.get("command").or_else(|| step.get("id")).and_then(Value::as_str).ok_or("a batch step names no command")?;
            Ok((id, step.get("params").unwrap_or(&Value::Null)))
        })
        .collect()
}

fn family(id: &str) -> &str {
    id.split('.').next().unwrap_or_default()
}

fn needed(id: &str, params: &Value, needs: &mut BTreeSet<Permission>) -> Result<(), String> {
    if id == "engine.batch" {
        return steps(params)?.into_iter().try_for_each(|(inner, inner_params)| needed(inner, inner_params, needs));
    }
    let spec = commands::find(id).ok_or_else(|| format!("unknown command `{id}`"))?;
    let document = DOCUMENT_FAMILIES.contains(&family(id));
    let names_files = confine::names_files(id) && (document || FILE_FAMILIES.contains(&family(id)));
    if REFUSED.contains(&id) || !(document || names_files) {
        return Err(format!("`{id}` is not available to plugins"));
    }
    if params.get("wait").and_then(Value::as_bool) == Some(true) {
        return Err(format!("`{id}` cannot wait for a background job when a plugin runs it"));
    }
    if names_files {
        needs.extend(FILE_PERMISSIONS);
    }
    needs.extend(outside_the_document(id, params));
    needs.insert(if spec.journal { Permission::DocumentWrite } else { Permission::DocumentRead });
    Ok(())
}

fn outside_the_document(id: &str, params: &Value) -> Option<Permission> {
    match id {
        "text.fonts" => Some(Permission::FilesRead),
        "layer.setText" if params.get("font").is_some() => Some(Permission::FilesWrite),
        _ => None,
    }
}

fn refers_to_a_step(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|text| text.starts_with('$'))
}

impl<'a> SessionHost<'a> {
    pub fn new(session: &'a mut Session, plugin: &'a Loaded) -> Self {
        SessionHost { session, plugin }
    }

    fn confinement(&self) -> Result<Confinement, String> {
        let (mut read, mut write) = (vec![self.plugin.bundle.dir.clone()], vec![self.plugin.data_dir.clone()]);
        if self.holds_file_permissions() {
            let everywhere = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
            read.push(everywhere.clone());
            write.push(everywhere);
        }
        std::fs::create_dir_all(&self.plugin.data_dir).map_err(|e| e.to_string())?;
        Confinement::new(&read, &write).map_err(|e| e.to_string())
    }

    fn holds_file_permissions(&self) -> bool {
        FILE_PERMISSIONS.iter().all(|p| self.plugin.bundle.manifest.permissions.contains(p))
    }

    fn property_write_allowed(&mut self, id: &str, params: &Value, reshaped: bool) -> Result<(), String> {
        if !WRITES_A_PROPERTY.contains(&id) {
            return Ok(());
        }
        let needs_files = |what: &str| format!("`{id}`: {what} needs the `files.read` and `files.write` permissions");
        let path = params.get("path").and_then(Value::as_str);
        let under_effects = path.is_none_or(|path| path.starts_with("effects"));
        if params.get("selected").and_then(Value::as_bool) == Some(true) && under_effects {
            return Err(needs_files("writing an effect property on every selected layer"));
        }
        let created_by_the_batch = ["layer", "prop", "path"].iter().any(|key| refers_to_a_step(params.get(*key)));
        if created_by_the_batch || reshaped {
            return if under_effects {
                Err(needs_files("writing an effect property after a batch step that can change which property the path names"))
            } else {
                Ok(())
            };
        }
        let mut target = serde_json::Map::new();
        for key in ["layer", "comp", "path", "prop"] {
            if let Some(value) = params.get(key) {
                target.insert(key.to_string(), value.clone());
            }
        }
        if !target.contains_key("path") && !target.contains_key("prop") {
            return Err(needs_files("writing the selected properties"));
        }
        let found = self.session.execute("prop.get", Value::Object(target)).map_err(|e| e.to_string())?;
        if found.get("type").and_then(Value::as_str) == Some("string") {
            return Err(needs_files("writing a text-valued property, which an effect may read as a file path,"));
        }
        Ok(())
    }

    fn property_writes_allowed(&mut self, id: &str, params: &Value) -> Result<(), String> {
        if self.holds_file_permissions() {
            return Ok(());
        }
        if id != "engine.batch" {
            return self.property_write_allowed(id, params, false);
        }
        let owned: Vec<(String, Value)> = steps(params)?.into_iter().map(|(inner, inner_params)| (inner.to_string(), inner_params.clone())).collect();
        let mut reshaped = false;
        for (inner, inner_params) in owned {
            self.property_write_allowed(&inner, &inner_params, reshaped)?;
            reshaped |= !WRITES_A_PROPERTY.contains(&inner.as_str()) && commands::find(&inner).is_some_and(|spec| spec.journal);
        }
        Ok(())
    }

    fn files_allowed(&mut self, id: &str, params: &Value) -> Result<(), String> {
        let confinement = self.confinement()?;
        let not_for_plugins = |e: effectcraft_automation::Error| format!("`{id}` is not available to this plugin: {e}");
        for lookup in confinement.check_command(id, params).map_err(not_for_plugins)? {
            let query = match lookup {
                Lookup::ProjectFile | Lookup::QueueDefault => "project.summary",
                Lookup::RenderOutputs => "renderQueue.list",
            };
            let found = self.session.execute(query, json!({})).map_err(|e| e.to_string())?;
            confinement.check_lookup(&lookup, &found).map_err(not_for_plugins)?;
        }
        Ok(())
    }
}

impl Host for SessionHost<'_> {
    fn access(&self, id: &str, params: &Value) -> Access {
        let mut needs = BTreeSet::new();
        match needed(id, params, &mut needs) {
            Ok(()) => Access::Needs(needs.into_iter().collect()),
            Err(why) => Access::Refused(why),
        }
    }

    fn notify(&mut self, message: &str) {
        let message: String = message.chars().take(MAX_NOTICE).collect();
        let message = format!("{}: {message}", self.plugin.bundle.manifest.name);
        self.session.events.push(Event::Toast { message, error: false });
    }

    fn command(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.property_writes_allowed(id, &params)?;
        self.files_allowed(id, &params)?;
        self.session.execute_checked(id, params).map_err(|e| e.to_string())
    }

    fn frame(&mut self, request: &FrameRequest) -> Result<Pixels, String> {
        if !(request.scale > 0.0 && request.scale <= MAX_FRAME_SCALE) || !request.time.is_finite() {
            return Err(format!("a frame needs a scale above 0 and at most {MAX_FRAME_SCALE}, and a finite time"));
        }
        let comp = find_comp(self.session, request.comp.as_deref())?;
        let solo = match &request.layer {
            Some(layer) => Some(only_layer(&self.session.project, comp, find_layer(&self.session.project, comp, layer)?, true)?),
            None => None,
        };
        let opts = self.session.render_opts(RenderOpts { scale: request.scale, ..RenderOpts::default() });
        let time = Tick::from_seconds_f64(request.time);
        let image = match &solo {
            Some(project) => {
                let mut renderer = Renderer::new(project, self.session.footage.as_ref(), opts);
                renderer.expr = self.session.expr.as_deref();
                renderer.accel = self.session.accel.as_deref();
                renderer.comp_frame(comp, time)
            }
            None => self.session.render(comp, time, opts),
        };
        Ok(Pixels { width: image.width as u32, height: image.height as u32, data: image.data })
    }

    fn audio(&mut self, request: &AudioRequest) -> Result<Samples, String> {
        let frames = request.duration * f64::from(request.rate);
        if !AUDIO_RATES.contains(&request.rate) || !(request.duration >= 0.0 && frames <= MAX_AUDIO_FRAMES) || !request.start.is_finite() {
            return Err(format!("audio needs a rate from {} to {} Hz and at most {MAX_AUDIO_FRAMES} sample frames", AUDIO_RATES.start(), AUDIO_RATES.end()));
        }
        let comp = find_comp(self.session, None)?;
        let solo = match &request.layer {
            Some(layer) => Some(only_layer(&self.session.project, comp, find_layer(&self.session.project, comp, layer)?, false)?),
            None => None,
        };
        let project = solo.as_ref().unwrap_or(&self.session.project);
        let start = Tick::from_seconds_f64(request.start);
        let samples = audio::mix_comp(project, self.session.footage.as_ref(), self.session.expr.as_deref(), comp, start, frames.round() as usize, request.rate);
        Ok(Samples { channels: 2, rate: request.rate, samples })
    }

    fn pick_file(&mut self, request: &PickFile) -> Result<Option<PathBuf>, String> {
        let picker = FILE_PICKER.get().ok_or("a file dialog cannot open in a headless run")?;
        Ok(picker(request))
    }
}

#[cfg(test)]
mod census {
    use std::collections::BTreeMap;

    fn decision(id: &str) -> String {
        let mut needs = std::collections::BTreeSet::new();
        match super::needed(id, &serde_json::Value::Null, &mut needs) {
            Ok(()) => needs.iter().map(|p| p.name()).collect::<Vec<_>>().join("+"),
            Err(_) => "refused".into(),
        }
    }

    #[test]
    fn every_command_in_an_allowed_family_has_a_recorded_decision() {
        let allowed = |id: &str| super::DOCUMENT_FAMILIES.contains(&super::family(id)) || super::FILE_FAMILIES.contains(&super::family(id));
        let found: BTreeMap<&str, String> =
            effectcraft_engine::commands::command_specs().iter().filter(|spec| allowed(spec.id)).map(|spec| (spec.id, decision(spec.id))).collect();
        let recorded: BTreeMap<&str, String> =
            include_str!("gate/census.tsv").lines().filter_map(|line| line.split_once('\t')).map(|(id, decision)| (id, decision.to_string())).collect();
        let differences: Vec<String> = found
            .iter()
            .filter(|(id, decision)| recorded.get(*id) != Some(*decision))
            .map(|(id, decision)| format!("{id}\t{decision}"))
            .chain(recorded.keys().filter(|id| !found.contains_key(*id)).map(|id| format!("{id}\tgone")))
            .collect();
        assert!(differences.is_empty(), "decide these commands and record them in census.tsv:\n{}", differences.join("\n"));
    }
}
