mod gate;
mod native;
mod serve;
mod wasm;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use craft_plugin::Permission;

use self::native::NativeGuest;
use self::serve::{HostCx, Identity};
use self::wasm::{WasmGuest, WasmModule};
use crate::host::Host;
use crate::plugin::{Io, PluginError};
use crate::registry::Bundle;

enum Kind {
    Wasm { module: WasmModule, main: Mutex<Option<Box<WasmGuest>>>, pool: Mutex<Vec<WasmGuest>> },
    Native(NativeGuest),
}

pub(crate) struct Engine {
    identity: Identity,
    kind: Kind,
}

impl Engine {
    pub fn open(bundle: &Bundle, grants: BTreeSet<Permission>, data_dir: PathBuf) -> Result<Engine, PluginError> {
        let entry = bundle.entry_path().map_err(|e| PluginError::Load(e.to_string()))?;
        let kind = if bundle.manifest.sandboxed() {
            let bytes = std::fs::read(&entry).map_err(|e| PluginError::Load(format!("{}: {e}", entry.display())))?;
            Kind::Wasm { module: WasmModule::compile(&bytes)?, main: Mutex::new(None), pool: Mutex::new(Vec::new()) }
        } else {
            Kind::Native(NativeGuest::load(&entry)?)
        };
        let identity = Identity { id: bundle.manifest.id.clone(), dir: bundle.dir.clone(), data_dir, grants };
        Ok(Engine { identity, kind })
    }

    pub fn copies_pixels(&self) -> bool {
        matches!(self.kind, Kind::Wasm { .. })
    }

    pub fn call(&self, request: &[u8], io: Io, host: &mut dyn Host, stateful: bool, budget: Duration) -> Result<Vec<u8>, PluginError> {
        let mut cx = HostCx { host, plugin: &self.identity };
        match &self.kind {
            Kind::Native(guest) => match io {
                Io::None => Ok(guest.call(request, None, &mut cx)),
                Io::InPlace(pixels) => Ok(guest.call(request, Some(pixels), &mut cx)),
                Io::Region { .. } => Err(PluginError::Protocol("native plugins render in place".into())),
            },
            Kind::Wasm { module, main, .. } if stateful => {
                let mut slot = main.lock().map_err(|_| PluginError::Trap("an earlier call panicked".into()))?;
                let mut guest = match slot.take() {
                    Some(guest) => guest,
                    None => Box::new(module.instantiate()?),
                };
                let reply = guest.call(request, io, &mut cx, budget)?;
                *slot = Some(guest);
                Ok(reply)
            }
            Kind::Wasm { module, pool, .. } => {
                let pooled = pool.lock().ok().and_then(|mut pool| pool.pop());
                let mut guest = match pooled {
                    Some(guest) => guest,
                    None => module.instantiate()?,
                };
                let reply = guest.call(request, io, &mut cx, budget)?;
                if let Ok(mut pool) = pool.lock()
                    && pool.len() < rayon::current_num_threads()
                {
                    pool.push(guest);
                }
                Ok(reply)
            }
        }
    }
}
