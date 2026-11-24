use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};

use craft_plugin_host::{Access, AudioRequest, FrameRequest, Host, Permission, PickFile, Pixels, Place, Samples, data};
use effectcraft_automation::confine::{self, Confinement, Lookup};
use effectcraft_engine::project::{ItemId, LayerId, Project};
use effectcraft_engine::render::{RenderOpts, Renderer, audio};
use effectcraft_engine::time::Tick;
use effectcraft_engine::{EngineError, Event, Session, commands};
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

const READ_ONLY: &[&str] = &["text.fontFeatures", "text.fonts"];

const WRITES_A_PROPERTY: &[&str] = &["prop.set", "prop.setExpression", "prop.pickWhip", "prop.addKey", "keys.set"];

const MAX_NOTICE: usize = 300;

const FILE_PERMISSIONS: [Permission; 2] = [Permission::FilesRead, Permission::FilesWrite];

const MAX_AUDIO_FRAMES: f64 = (1u64 << 25) as f64;
const AUDIO_RATES: std::ops::RangeInclusive<u32> = 1000..=192_000;
const MAX_FRAME_SIDE: f64 = 16384.0;
const MAX_FRAME_BYTES: f64 = (1u64 << 30) as f64;
const RGBA_F32_BYTES: f64 = 16.0;

pub type FilePicker = Box<dyn Fn(&PickFile) -> Option<PathBuf> + Send + Sync>;

static FILE_PICKER: OnceLock<FilePicker> = OnceLock::new();

pub fn set_file_picker(picker: FilePicker) {
    let _ = FILE_PICKER.set(picker);
}

fn find_comp(session: &Session, name: Option<&str>, ids: bool) -> Result<ItemId, String> {
    let Some(name) = name else { return session.active_comp_id().ok_or_else(|| "there is no active composition".to_string()) };
    if ids {
        return name.parse().ok().map(ItemId).filter(|id| session.project.comp(*id).is_some()).ok_or_else(|| format!("no composition with id {name}"));
    }
    let by_name = session.project.items.values().find(|item| item.name == name && item.as_comp().is_some()).map(|item| item.id);
    by_name.or_else(|| name.parse().ok().map(ItemId).filter(|id| session.project.comp(*id).is_some())).ok_or_else(|| format!("no composition `{name}`"))
}

fn find_layer(project: &Project, comp: ItemId, name: &str, ids: bool) -> Result<LayerId, String> {
    let comp = project.comp(comp).ok_or("the composition is gone")?;
    if ids {
        return name.parse().ok().map(LayerId).filter(|id| comp.layer(*id).is_some()).ok_or_else(|| format!("no layer with id {name}"));
    }
    let by_name = comp.layers.iter().find(|layer| layer.name == name).map(|layer| layer.id);
    by_name.or_else(|| name.parse().ok().map(LayerId).filter(|id| comp.layer(*id).is_some())).ok_or_else(|| format!("no layer `{name}`"))
}

fn only_layer(project: &Project, comp: ItemId, layer: LayerId, video: bool) -> Result<Project, String> {
    let mut solo = project.clone();
    let comp = solo.comp_mut(comp).ok_or("the composition is gone")?;
    for other in comp.layers.iter_mut() {
        let keep = other.id == layer;
        if !video {
            other.switches.audio &= keep;
        } else if keep {
            other.switches.video = true;
        } else if !other.is_light() && !other.is_camera() {
            other.switches.video = false;
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
    needs.insert(if spec.journal && !READ_ONLY.contains(&id) { Permission::DocumentWrite } else { Permission::DocumentRead });
    Ok(())
}

fn outside_the_document(id: &str, params: &Value) -> Option<Permission> {
    match id {
        "text.fonts" => Some(Permission::FilesRead),
        "layer.setText" if params.get("font").is_some() => Some(Permission::FilesWrite),
        _ => None,
    }
}

fn outside_the_effects_by_name(path: &str) -> bool {
    use effectcraft_engine::project::props::Seg;
    let segs = effectcraft_engine::project::parse_path(path);
    let no_uid = !segs.iter().any(|seg| matches!(seg, Seg::Uid(_)));
    no_uid && matches!(segs.first(), Some(Seg::Match(first, _)) if !first.eq_ignore_ascii_case("effects"))
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

    fn copied_keys_hold_text(&self) -> bool {
        use effectcraft_engine::project::Value as KeyValue;
        self.session.state.key_clipboard.iter().flat_map(|clip| &clip.keys).any(|key| matches!(key.value, KeyValue::Str(_)))
    }

    fn property_write_allowed(&mut self, id: &str, params: &Value, reshaped: bool) -> Result<(), String> {
        if id == "keys.paste" && (reshaped || self.copied_keys_hold_text()) {
            return Err(format!("`{id}`: pasting keyframes that may hold text needs the `files.read` and `files.write` permissions"));
        }
        if id == "essential.set" && params.get("value").is_some_and(Value::is_string) {
            return Err(format!("`{id}`: writing text through an essential property needs the `files.read` and `files.write` permissions"));
        }
        if !WRITES_A_PROPERTY.contains(&id) {
            return Ok(());
        }
        let needs_files = |what: &str| format!("`{id}`: {what} needs the `files.read` and `files.write` permissions");
        let path = params.get("path").and_then(Value::as_str);
        let named_outside_the_effects = params.get("prop").is_none() && path.is_some_and(outside_the_effects_by_name);
        if params.get("selected").and_then(Value::as_bool) == Some(true) && !named_outside_the_effects {
            return Err(needs_files("writing an effect property on every selected layer"));
        }
        let created_by_the_batch = ["layer", "prop", "path"].iter().any(|key| refers_to_a_step(params.get(*key)));
        if created_by_the_batch || reshaped {
            if params.get("value").is_some_and(Value::is_string) {
                return Err(needs_files("writing text into a property a batch step can change"));
            }
            return if !named_outside_the_effects {
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
        if found.get("type").and_then(Value::as_str) == Some("string") && !self.plugin_owned_text(&found) {
            return Err(needs_files("writing a text-valued property, which an effect may read as a file path,"));
        }
        Ok(())
    }

    fn plugin_owned_text(&self, found: &Value) -> bool {
        let (Some(layer), Some(uid)) = (found.get("layer").and_then(Value::as_u64), found.get("uid").and_then(Value::as_u64)) else { return false };
        use effectcraft_engine::project::{GroupKind, LayerId, Node, ParamUi};
        let own = &self.plugin.bundle.manifest.id;
        let owned_by_this_plugin = |effect: &str| effectcraft_engine::effects::dynamic::get(effect).is_some_and(|found| &found.effect.owner == own);
        let Some(layer) = self.session.project.comps().find_map(|(_, comp)| comp.layer(LayerId(layer))) else { return false };
        let Some(effects) = layer.props.sub("effects") else { return false };
        effects.children.iter().any(|node| match node {
            Node::Group(effect) => {
                matches!(&effect.kind, GroupKind::Effect { effect } if owned_by_this_plugin(effect))
                    && effect.find(uid).is_some_and(|prop| matches!(prop.ui, ParamUi::Custom))
            }
            Node::Prop(_) => false,
        })
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
            reshaped |= inner.starts_with("keys.") || !WRITES_A_PROPERTY.contains(&inner.as_str()) && commands::find(&inner).is_some_and(|spec| spec.journal);
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
        self.frame_job(request).map(|render| render())
    }

    fn audio(&mut self, request: &AudioRequest) -> Result<Samples, String> {
        self.audio_job(request).map(|mix| mix())
    }

    fn project_closed(&self) -> Option<Arc<AtomicBool>> {
        Some(self.session.project_closed.clone())
    }

    fn clipboard(&mut self) -> Result<String, String> {
        crate::panels::clipboard()?.text()
    }

    fn set_clipboard(&mut self, text: &str) -> Result<(), String> {
        crate::panels::clipboard()?.set_text(text)
    }

    fn pick_file(&mut self, request: &PickFile) -> Result<Option<PathBuf>, String> {
        let picker = FILE_PICKER.get().ok_or("a file dialog cannot open in a headless run")?;
        Ok(picker(request))
    }

    fn data(&mut self, plugin: &str, place: Place, key: &str) -> Result<Option<Vec<u8>>, String> {
        let project = &*self.session.project;
        let found = match place {
            Place::Layer(id) => &project.comps().find_map(|(_, comp)| comp.layer(LayerId(id))).ok_or(no_layer(id).to_string())?.plugin_data,
            _ => &project.plugin_data,
        };
        Ok(found.get(plugin).and_then(|keys| keys.get(key)).cloned())
    }

    fn set_data(&mut self, plugin: &str, place: Place, key: &str, value: Option<Vec<u8>>) -> Result<(), String> {
        let label = format!("{} Data", self.plugin.bundle.manifest.name);
        let held = |found: &effectcraft_engine::project::plugin_data::PluginData| found.get(plugin).map_or(0, data::size);
        self.session
            .edit(&label, None, |project, _| {
                let total =
                    held(&project.plugin_data) + project.comps().flat_map(|(_, comp)| &comp.layers).map(|layer| held(&layer.plugin_data)).sum::<usize>();
                let found = match place {
                    Place::Layer(id) => {
                        let comp = project.comps().find(|(_, comp)| comp.layer(LayerId(id)).is_some()).map(|(comp, _)| *comp).ok_or(no_layer(id))?;
                        &mut project.comp_mut(comp).and_then(|comp| comp.layer_mut(LayerId(id))).ok_or(no_layer(id))?.plugin_data
                    }
                    _ => &mut project.plugin_data,
                };
                let mut keys = found.remove(plugin).unwrap_or_default();
                let elsewhere = total - data::size(&keys);
                let written = data::write(&mut keys, elsewhere, key, value);
                if !keys.is_empty() {
                    found.insert(plugin.to_string(), keys);
                }
                written.map_err(EngineError::Other)
            })
            .map_err(|e| e.to_string())
    }
}

fn no_layer(id: u64) -> EngineError {
    EngineError::Other(format!("no layer {id} in the project; read layer ids from `layer.list` or the selection"))
}

pub(crate) type Job<T> = Box<dyn FnOnce() -> T + Send>;

fn frame_size(size: (u32, u32), scale: f64) -> Result<(u32, u32), String> {
    let (width, height) = ((f64::from(size.0) * scale).ceil(), (f64::from(size.1) * scale).ceil());
    if width > MAX_FRAME_SIDE || height > MAX_FRAME_SIDE || width * height * RGBA_F32_BYTES > MAX_FRAME_BYTES {
        return Err(format!(
            "a {width}×{height} frame is too large: a frame may be at most {MAX_FRAME_SIDE} pixels on a side and {} MiB as RGBA floats",
            MAX_FRAME_BYTES / f64::from(1u32 << 20)
        ));
    }
    Ok((width.max(1.0) as u32, height.max(1.0) as u32))
}

impl SessionHost<'_> {
    pub(crate) fn frame_job(&self, request: &FrameRequest) -> Result<Job<Pixels>, String> {
        craft_plugin_host::check_frame(request)?;
        let session = &*self.session;
        let comp = find_comp(session, request.comp.as_deref(), request.ids)?;
        let layer = request.layer.as_deref().map(|layer| find_layer(&session.project, comp, layer, request.ids)).transpose()?;
        if let (true, Some(layer)) = (request.source, layer) {
            return self.source_job(request, comp, layer);
        }
        let size = session.project.comp(comp).map(|found| (found.width, found.height)).ok_or("the composition is gone")?;
        frame_size(size, request.scale)?;
        let (project, cache) = match layer {
            Some(layer) => (Arc::new(only_layer(&session.project, comp, layer, true)?), None),
            None => (session.project.clone(), Some(session.layer_cache.clone())),
        };
        let opts = session.render_opts(RenderOpts { scale: request.scale, ..RenderOpts::default() });
        let (footage, expr, accel, time) = (session.footage.clone(), session.expr.clone(), session.accel.clone(), Tick::from_seconds_f64(request.time));
        Ok(Box::new(move || {
            let mut renderer = Renderer::new(&project, footage.as_ref(), opts);
            renderer.expr = expr.as_deref();
            renderer.accel = accel.as_deref();
            renderer.cache = cache.as_deref();
            let image = renderer.comp_frame(comp, time);
            Pixels { width: image.width, height: image.height, data: image.data }
        }))
    }

    fn source_job(&self, request: &FrameRequest, comp: ItemId, layer: LayerId) -> Result<Job<Pixels>, String> {
        let session = &*self.session;
        let project = session.project.clone();
        let found = project.comp(comp).ok_or("the composition is gone")?;
        let source = found.layer(layer).ok_or("the layer is gone")?;
        let size = match effectcraft_engine::render::source_size(&project, source) {
            (0, _) | (_, 0) => (found.width, found.height),
            size => size,
        };
        let (width, height) = frame_size(size, request.scale)?;
        let opts = session.render_opts(RenderOpts { scale: request.scale, ..RenderOpts::default() });
        let (footage, expr, scale, time) = (session.footage.clone(), session.expr.clone(), request.scale, Tick::from_seconds_f64(request.time));
        Ok(Box::new(move || {
            let mut renderer = Renderer::new(&project, footage.as_ref(), opts);
            renderer.expr = expr.as_deref();
            let read = renderer.eval_ctx(comp, time).and_then(|ctx| renderer.layer_source(&ctx, ctx.layer(layer)?));
            let mut data = vec![[0.0; 4]; width as usize * height as usize];
            if let Some(buf) = read {
                for (i, pixel) in data.iter_mut().enumerate() {
                    let (x, y) = ((i % width as usize) as f64 + 0.5, (i / width as usize) as f64 + 0.5);
                    let [bx, by] = [x, y].map(|v| v / scale * buf.scale);
                    *pixel = buf.img.get((bx + buf.offset[0]).floor() as i64, (by + buf.offset[1]).floor() as i64);
                }
            }
            Pixels { width, height, data }
        }))
    }

    pub(crate) fn audio_job(&self, request: &AudioRequest) -> Result<Job<Samples>, String> {
        let frames = request.duration * f64::from(request.rate);
        if !AUDIO_RATES.contains(&request.rate) || !(request.duration >= 0.0 && frames <= MAX_AUDIO_FRAMES) || !request.start.is_finite() {
            return Err(format!("audio needs a rate from {} to {} Hz and at most {MAX_AUDIO_FRAMES} sample frames", AUDIO_RATES.start(), AUDIO_RATES.end()));
        }
        let session = &*self.session;
        let comp = find_comp(session, None, false)?;
        let project = match &request.layer {
            Some(layer) => Arc::new(only_layer(&session.project, comp, find_layer(&session.project, comp, layer, false)?, false)?),
            None => session.project.clone(),
        };
        let (footage, expr, start, rate) = (session.footage.clone(), session.expr.clone(), Tick::from_seconds_f64(request.start), request.rate);
        Ok(Box::new(move || {
            let samples = audio::mix_comp(&project, footage.as_ref(), expr.as_deref(), comp, start, frames.round() as usize, rate);
            Samples { channels: 2, rate, samples }
        }))
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

    #[test]
    fn every_read_only_exception_is_a_command_the_engine_journals() {
        let stale: Vec<&str> = super::READ_ONLY.iter().copied().filter(|id| !effectcraft_engine::commands::find(id).is_some_and(|spec| spec.journal)).collect();
        assert!(stale.is_empty(), "the engine no longer journals these, so remove them from READ_ONLY: {stale:?}");
    }
}
