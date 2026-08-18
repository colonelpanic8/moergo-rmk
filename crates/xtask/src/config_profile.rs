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

pub fn verify_stock_defaults(
    stock: &Path,
    configured: &Path,
    bilateral_thumbs: bool,
) -> Result<Digests> {
    let stock_source = fs::read_to_string(stock)?;
    let configured_source = fs::read_to_string(configured)?;
    let stock_configuration: Value = toml::from_str(&stock_source)?;
    let configured_configuration: Value = toml::from_str(&configured_source)?;

    verify_stock_values(
        stock_configuration,
        configured_configuration,
        bilateral_thumbs,
    )
    .map_err(|error| {
        format!(
            "compiled defaults differ from stock ({error}):\n  stock:      {}\n  configured: {}",
            stock.display(),
            configured.display(),
        )
    })?;

    digests_from_str(&configured_source)
}

fn verify_stock_values(mut stock: Value, configured: Value, bilateral_thumbs: bool) -> Result<()> {
    if bilateral_thumbs {
        mark_glove80_thumbs_bilateral(&mut stock)?;
    }
    if stock != configured {
        return Err("configured defaults differ from stock".into());
    }
    Ok(())
}

fn mark_glove80_thumbs_bilateral(configuration: &mut Value) -> Result<()> {
    let map = configuration
        .get_mut("layout")
        .and_then(Value::as_table_mut)
        .and_then(|layout| layout.get_mut("map"))
        .and_then(|value| value.as_str())
        .ok_or("stock configuration must define layout.map")?;
    let mut transformed = map.to_owned();
    for (row, col, hand) in [
        (0, 6, 'L'),
        (1, 6, 'L'),
        (2, 6, 'L'),
        (3, 6, 'L'),
        (4, 6, 'L'),
        (5, 6, 'L'),
        (0, 7, 'R'),
        (1, 7, 'R'),
        (2, 7, 'R'),
        (3, 7, 'R'),
        (4, 7, 'R'),
        (5, 7, 'R'),
    ] {
        let original = format!("({row},{col},{hand},");
        let bilateral = format!("({row},{col},*,");
        if !transformed.contains(&original) {
            return Err(format!("stock layout.map is missing Glove80 thumb {original}").into());
        }
        transformed = transformed.replace(&original, &bilateral);
    }
    *configuration
        .get_mut("layout")
        .and_then(Value::as_table_mut)
        .and_then(|layout| layout.get_mut("map"))
        .expect("layout.map was checked above") = Value::String(transformed);
    Ok(())
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

    #[test]
    fn stock_defaults_reject_personal_bindings() {
        let stock: Value = toml::from_str(STOCK).unwrap();
        let personal: Value = toml::from_str(PERSONAL).unwrap();
        assert!(verify_stock_values(stock.clone(), stock, false).is_ok());
        assert!(verify_stock_values(toml::from_str(STOCK).unwrap(), personal, false).is_err());
    }

    #[test]
    fn glove80_stock_normalization_only_changes_thumb_hands() {
        let mut stock: Value = toml::from_str(
            r#"
[layout]
map = "(0,6,L,@thumb) (1,6,L,@thumb) (2,6,L,@thumb) (3,6,L,@thumb) (4,6,L,@thumb) (5,6,L,@thumb) (0,7,R,@thumb) (1,7,R,@thumb) (2,7,R,@thumb) (3,7,R,@thumb) (4,7,R,@thumb) (5,7,R,@thumb)"
"#,
        )
        .unwrap();
        mark_glove80_thumbs_bilateral(&mut stock).unwrap();
        let map = stock["layout"]["map"].as_str().unwrap();
        assert_eq!(map.matches(",*,").count(), 12);
        assert!(!map.contains(",L,"));
        assert!(!map.contains(",R,"));
    }
}
