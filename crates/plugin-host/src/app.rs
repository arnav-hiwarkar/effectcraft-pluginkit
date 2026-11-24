use std::sync::Arc;

use craft_plugin::{Contributions, Param, ParamKind, Value as Wire};

pub trait ParamValues {
    type Value;

    fn to_wire(&self, param: &Param, value: &Self::Value) -> Wire;

    fn from_wire(param: &Param, value: Wire) -> Option<Self::Value>;
}

pub trait EffectRegistry {
    type Loaded;

    fn supports(&self, _kind: &ParamKind) -> bool {
        true
    }

    fn register(&self, loaded: &Arc<Self::Loaded>);

    fn unregister(&self, plugin: &str);

    fn admit(&self, contributions: &Contributions) -> Result<(), String> {
        let refused = contributions
            .effects
            .iter()
            .flat_map(|effect| effect.params.iter().map(move |param| (effect, param)))
            .find(|(_, param)| !self.supports(&param.kind));
        match refused {
            Some((effect, param)) => Err(format!(
                "effect `{}` parameter `{}` is a {} control, which this app does not offer; remove it or use another kind",
                effect.id,
                param.id,
                kind_name(&param.kind)
            )),
            None => Ok(()),
        }
    }
}

fn kind_name(kind: &ParamKind) -> String {
    serde_json::to_value(kind).ok().and_then(|value| value.get("kind")?.as_str().map(str::to_string)).unwrap_or_default()
}
