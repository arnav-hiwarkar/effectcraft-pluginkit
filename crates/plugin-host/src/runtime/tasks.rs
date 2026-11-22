use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use craft_plugin::{TaskState, TaskStatus};

use super::serve::Bulk;

pub(crate) const MAX_RUNNING: usize = 4;
pub(crate) const GRACE: Duration = Duration::from_millis(500);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) struct Slot {
    state: Mutex<TaskState>,
    cancelled_at: Mutex<Option<Instant>>,
}

impl Slot {
    pub fn state(&self) -> TaskState {
        lock(&self.state).clone()
    }

    pub fn cancelled(&self) -> bool {
        lock(&self.cancelled_at).is_some()
    }

    pub fn overdue(&self) -> bool {
        lock(&self.cancelled_at).is_some_and(|at| at.elapsed() >= GRACE)
    }

    pub fn progress(&self, fraction: f64, message: String) -> Result<(), String> {
        if self.cancelled() {
            return Err("cancelled".into());
        }
        let mut state = lock(&self.state);
        state.progress = fraction.clamp(0.0, 1.0);
        state.message = message;
        Ok(())
    }

    pub fn finish(&self, outcome: Result<serde_json::Value, String>) {
        let cancelled = self.cancelled();
        let mut state = lock(&self.state);
        match outcome {
            _ if cancelled => state.status = TaskStatus::Cancelled,
            Ok(value) => {
                state.status = TaskStatus::Done;
                state.progress = 1.0;
                state.result = Some(value);
            }
            Err(error) => {
                state.status = TaskStatus::Failed;
                state.error = Some(error);
            }
        }
    }

    fn running(&self) -> bool {
        lock(&self.state).status == TaskStatus::Running
    }
}

pub(crate) struct Forwarded {
    pub task: Arc<Slot>,
    pub request: Vec<u8>,
    pub bulk: Vec<u8>,
    pub reply: Sender<(Vec<u8>, Bulk)>,
}

#[derive(Default)]
pub(crate) struct Tasks {
    known: Mutex<Vec<String>>,
    next: Mutex<u64>,
    slots: Mutex<BTreeMap<u64, Arc<Slot>>>,
    queue: Mutex<VecDeque<Forwarded>>,
}

impl Tasks {
    pub fn know(&self, names: Vec<String>) {
        *lock(&self.known) = names;
    }

    pub fn add(&self, task: &str) -> Result<(u64, Arc<Slot>), String> {
        if !lock(&self.known).iter().any(|known| known == task) {
            return Err(format!("this plugin has no task `{task}`"));
        }
        let mut slots = lock(&self.slots);
        if slots.values().filter(|slot| slot.running()).count() >= MAX_RUNNING {
            return Err(format!("this plugin already runs {MAX_RUNNING} tasks"));
        }
        let id = {
            let mut next = lock(&self.next);
            *next += 1;
            *next
        };
        let state = TaskState { id, task: task.to_string(), ..Default::default() };
        let slot = Arc::new(Slot { state: Mutex::new(state), cancelled_at: Mutex::new(None) });
        slots.insert(id, slot.clone());
        Ok((id, slot))
    }

    pub fn get(&self, id: u64) -> Option<Arc<Slot>> {
        lock(&self.slots).get(&id).cloned()
    }

    pub fn list(&self) -> Vec<TaskState> {
        lock(&self.slots).values().map(|slot| slot.state()).collect()
    }

    pub fn running(&self) -> usize {
        lock(&self.slots).values().filter(|slot| slot.running()).count()
    }

    pub fn cancel(&self, id: u64) -> bool {
        let Some(slot) = self.get(id) else { return false };
        lock(&slot.cancelled_at).get_or_insert_with(Instant::now);
        lock(&self.queue).retain(|forwarded| !Arc::ptr_eq(&forwarded.task, &slot));
        true
    }

    pub fn cancel_all(&self) {
        let ids: Vec<u64> = lock(&self.slots).keys().copied().collect();
        ids.into_iter().for_each(|id| {
            self.cancel(id);
        });
    }

    pub fn forward(&self, task: &Arc<Slot>, request: &[u8], bulk: &[u8]) -> Result<(Vec<u8>, Bulk), String> {
        let (reply, answer) = channel();
        {
            let mut queue = lock(&self.queue);
            if task.cancelled() {
                return Err("cancelled".into());
            }
            queue.push_back(Forwarded { task: task.clone(), request: request.to_vec(), bulk: bulk.to_vec(), reply });
        }
        answer.recv().map_err(|_| "cancelled".into())
    }

    pub fn take_queue(&self) -> Vec<Forwarded> {
        lock(&self.queue).drain(..).collect()
    }
}
