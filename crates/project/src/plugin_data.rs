use std::collections::BTreeMap;

use craft_mime::base64::STANDARD;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub type PluginData = BTreeMap<String, BTreeMap<String, Vec<u8>>>;

pub fn serialize<S: Serializer>(data: &PluginData, serializer: S) -> Result<S::Ok, S::Error> {
    let text: BTreeMap<&str, BTreeMap<&str, String>> =
        data.iter().map(|(plugin, keys)| (plugin.as_str(), keys.iter().map(|(key, value)| (key.as_str(), STANDARD.encode(value))).collect())).collect();
    text.serialize(serializer)
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PluginData, D::Error> {
    let text = BTreeMap::<String, BTreeMap<String, String>>::deserialize(deserializer)?;
    text.into_iter()
        .map(|(plugin, keys)| {
            let keys = keys
                .into_iter()
                .map(|(key, value)| STANDARD.decode(&value).map(|bytes| (key, bytes)))
                .collect::<Result<_, _>>()
                .map_err(|_| serde::de::Error::custom(format!("plug-in data of `{plugin}` is not base64")))?;
            Ok((plugin, keys))
        })
        .collect()
}
