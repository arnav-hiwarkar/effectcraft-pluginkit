use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use craft_plugin_host::{Form, Value};
use effectcraft_engine::Session;

pub type Values = BTreeMap<String, Value>;

pub struct AskedForm {
    pub plugin: String,
    pub form: Form,
    pub values: Values,
}

struct Pending {
    id: u64,
    project: Arc<AtomicBool>,
    asked: AskedForm,
    reply: Sender<Option<Values>>,
}

static PENDING: Mutex<Vec<Pending>> = Mutex::new(Vec::new());
static NEXT: AtomicU64 = AtomicU64::new(0);

fn pending() -> std::sync::MutexGuard<'static, Vec<Pending>> {
    PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn ask(plugin: &str, project: Arc<AtomicBool>, form: &Form, reply: Sender<Option<Values>>) -> u64 {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let asked = AskedForm { plugin: plugin.to_string(), values: form.defaults(), form: form.clone() };
    pending().push(Pending { id, project, asked, reply });
    id
}

pub(crate) fn withdraw(id: u64) {
    pending().retain(|form| form.id != id);
}

pub fn asked_form<T>(session: &Session, look: impl FnOnce(&mut AskedForm) -> T) -> Option<T> {
    pending().iter_mut().find(|form| Arc::ptr_eq(&form.project, &session.project_closed)).map(|form| look(&mut form.asked))
}

pub fn answer_form(session: &Session, values: Option<Values>) -> Result<(), String> {
    let mut forms = pending();
    let at = forms.iter().position(|form| Arc::ptr_eq(&form.project, &session.project_closed)).ok_or("no plug-in form is open")?;
    let answer = match values {
        Some(values) => {
            let asked = &forms[at].asked;
            let mut merged = asked.values.clone();
            merged.extend(values);
            Some(asked.form.answer(merged)?)
        }
        None => None,
    };
    let _ = forms.remove(at).reply.send(answer);
    Ok(())
}
