use std::{fs, path::Path};

use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use toml::Value;

use crate::Result;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Digests {
    pub configuration: String,
    pub platform_profile: String,
}

pub fn digests(path: &Path) -> Result<Digests> {
    let source = fs::read_to_string(path)?;
    digests_from_str(&source)
}

pub fn verify(stock: &Path, configured: &Path) -> Result<Digests> {
    let stock_digests = digests(stock)?;
    let configured_digests = digests(configured)?;
    if stock_digests.platform_profile != configured_digests.platform_profile {
        return Err(format!(
            "Go60 platform profile mismatch:\n  stock:      {} ({})\n  configured: {} ({})",
            stock_digests.platform_profile,
            stock.display(),
            configured_digests.platform_profile,
            configured.display(),
        )
        .into());
    }
    Ok(configured_digests)
}

fn digests_from_str(source: &str) -> Result<Digests> {
    let configuration: Value = toml::from_str(source)?;
    let platform_profile = platform_profile(&configuration)?;
    Ok(Digests {
        configuration: hash(&configuration)?,
        platform_profile: hash(&platform_profile)?,
    })
}

fn platform_profile(configuration: &Value) -> Result<Value> {
    let mut profile = configuration.clone();
    let root = profile
        .as_table_mut()
        .ok_or("keyboard configuration must be a TOML table")?;

    let layer_names = root
        .get("keymap")
        .and_then(Value::as_table)
        .and_then(|keymap| keymap.get("layer"))
        .and_then(Value::as_array)
        .ok_or("keyboard configuration must define [[keymap.layer]]")?
        .iter()
        .map(|layer| {
            layer
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| "every [[keymap.layer]] needs a name".into())
        })
        .collect::<Result<Vec<String>>>()?;

    if let Some(wake_layers) = root
        .get_mut("lighting")
        .and_then(Value::as_table_mut)
        .and_then(|lighting| lighting.get_mut("controls"))
        .and_then(Value::as_table_mut)
        .and_then(|controls| controls.get_mut("wake_layers"))
    {
        let semantic = wake_layers
            .as_array()
            .ok_or("lighting.controls.wake_layers must be an array")?
            .iter()
            .map(|layer| {
                let index = layer
                    .as_integer()
                    .ok_or("lighting wake layer must be an integer")?;
                let index = usize::try_from(index)
                    .map_err(|_| "lighting wake layer must not be negative")?;
                layer_names
                    .get(index)
                    .cloned()
                    .map(Value::String)
                    .ok_or_else(|| {
                        format!("lighting wake layer {index} has no named keymap layer").into()
                    })
            })
            .collect::<Result<Vec<Value>>>()?;
        *wake_layers = Value::Array(semantic);
    }

    let keymap = root
        .get_mut("keymap")
        .and_then(Value::as_table_mut)
        .ok_or("keyboard configuration must define [keymap]")?;
    keymap.remove("layer");
    root.remove("behavior");

    Ok(profile)
}

fn hash(value: &Value) -> Result<String> {
    let canonical: JsonValue = serde_json::to_value(value)?;
    let digest = Sha256::digest(serde_json::to_vec(&canonical)?);
    Ok(format!("{digest:x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOCK: &str = r#"
[keyboard]
name = "Go60"
[rmk]
ble_profiles_num = 3
combo_max_num = 32
morse_max_num = 64
[keymap]
layers = 16
[[keymap.layer]]
name = "Base"
keys = "A"
[[keymap.layer]]
name = "Magic"
keys = "MO(1)"
[lighting]
topology_revision = 1
[lighting.controls]
wake_layers = [1]
"#;

    const PERSONAL: &str = r#"
[keyboard]
name = "Go60"
[rmk]
ble_profiles_num = 3
combo_max_num = 32
morse_max_num = 64
[keymap]
layers = 16
[[keymap.layer]]
name = "Base"
keys = "B"
[[keymap.layer]]
name = "Lower"
keys = "_"
[[keymap.layer]]
name = "Magic"
keys = "MO(2)"
[behavior.morse.profiles.personal]
hold_timeout = "180ms"
[[behavior.morse.morses]]
profile = "personal"
tap = "A"
hold = "B"
[lighting]
topology_revision = 1
[lighting.controls]
wake_layers = [2]
"#;

    #[test]
    fn default_bindings_and_magic_index_do_not_change_platform_profile() {
        let stock = digests_from_str(STOCK).unwrap();
        let personal = digests_from_str(PERSONAL).unwrap();
        assert_ne!(stock.configuration, personal.configuration);
        assert_eq!(stock.platform_profile, personal.platform_profile);
    }

    #[test]
    fn comments_and_table_order_do_not_change_hashes() {
        let reordered = STOCK.replace(
            "[keyboard]\nname = \"Go60\"",
            "# comment\n[keyboard]\nname = \"Go60\"",
        );
        assert_eq!(
            digests_from_str(STOCK).unwrap(),
            digests_from_str(&reordered).unwrap()
        );
    }

    #[test]
    fn capacity_drift_changes_platform_profile() {
        let smaller = PERSONAL.replace("morse_max_num = 64", "morse_max_num = 8");
        assert_ne!(
            digests_from_str(STOCK).unwrap().platform_profile,
            digests_from_str(&smaller).unwrap().platform_profile
        );
    }

    #[test]
    fn hardware_drift_changes_platform_profile() {
        let changed = PERSONAL.replace("name = \"Go60\"", "name = \"Other\"");
        assert_ne!(
            digests_from_str(STOCK).unwrap().platform_profile,
            digests_from_str(&changed).unwrap().platform_profile
        );
    }

    #[test]
    fn wake_layer_must_resolve_to_a_named_layer() {
        let invalid = STOCK.replace("wake_layers = [1]", "wake_layers = [15]");
        assert!(digests_from_str(&invalid).is_err());
    }
}
