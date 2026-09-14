//! The `config` subcommand: files, device I/O, and reporting around the pure
//! runtime-state model in [`moergo_config`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Subcommand, ValueEnum};
use moergo_config::{
    background_from_wire, background_to_wire, conditional_scene_from_advanced_wire,
    conditional_scene_from_wire, conditional_scene_to_advanced_wire, conditional_scene_to_wire,
    differences, effects_from_wire, effects_to_wire, live_param_tables, output_mode_from_wire,
    output_mode_to_wire, params_to_writes, runtime_config_from_moergo_json, scene_from_wire,
    scene_policy_from_wire, scene_policy_to_wire, scene_to_wire, snapshot_to_moergo_json,
    BehaviorSnapshot, EffectParams, EffectsConfig, LightingConfig, LightingSnapshot,
    OutputModeConfig, ParamSpec, RuntimeConfig, Snapshot,
};
use rynk::rmk_types::morse::MorseProfileName;
use rynk::rmk_types::pointing::PointingMode;
use rynk::rmk_types::protocol::rynk::{
    BleName, Cmd, LayerMetadata, LightingAdvancedConditionalSceneCell, LightingError,
    LightingExtendedConditionalSceneCell, LightingExtensionNameKind,
    LightingExtensionParamsRequest, LightingFeatureFlags, LightingMutableState,
    MorseProfileEntry as WireMorseProfileEntry, PointingCapabilities,
    PointingConfig as WirePointingConfig, RynkError, SetAutoMouseLayerConfigsRequest,
    SetKeymapBulkRequest, SetLightingExtensionLayersRequest, SetLightingExtensionParamRequest,
    SetLightingExtensionStateRequest, SetLightingLayerPolicyRequest, SetLightingOutputModeRequest,
    SetLightingStateRequest, SetLightingWakeLayersRequest, SetMorseHoldTriggerPositionsRequest,
    SetMorseProfileEntryRequest,
};
use rynk::{Client, RynkHostError};

use crate::transport::Selector;

pub use moergo_config::DiffFound;

const CONDITIONAL_READ_ATTEMPTS: usize = 3;
const CONDITIONAL_READ_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Validate a runtime TOML or MoErgo Layout Editor JSON file offline.
    Validate { file: PathBuf },
    /// Compare a runtime TOML or MoErgo JSON file with the keyboard.
    Diff {
        file: PathBuf,
        /// Treat the file as the whole managed state, so a behavior table it
        /// does not mention reads as empty rather than as "leave alone".
        #[arg(long)]
        exact: bool,
    },
    /// Apply a runtime TOML or MoErgo JSON file and verify it by read-back.
    Apply {
        file: PathBuf,
        /// Show differences without writing anything.
        #[arg(long)]
        dry_run: bool,
        /// Set the keyboard to exactly this file: behavior tables it does not
        /// mention are cleared instead of left as the keyboard holds them.
        ///
        /// Without this, an absent `[[morse]]` / `[[combo]]` / `[[fork]]` /
        /// `[[macro]]` section means "this file says nothing about that table",
        /// which is what keeps a configuration written before those sections
        /// existed from wiping them. That rule also means no file can ever clear
        /// one, which is what this flag is for.
        #[arg(long)]
        exact: bool,
    },
    /// Pull the connected keyboard's runtime state into a TOML or JSON file.
    Pull {
        file: PathBuf,
        /// Output format. Inferred from an existing file or its extension when omitted.
        #[arg(long, value_enum)]
        format: Option<ConfigFormat>,
    },
    /// Print the connected keyboard's runtime state.
    Show {
        #[arg(long, value_enum, default_value_t = ConfigFormat::Toml)]
        format: ConfigFormat,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum ConfigFormat {
    #[default]
    Toml,
    /// Experimental JSON backup format from the MoErgo Layout Editor.
    #[value(name = "moergo-json", alias = "json")]
    MoergoJson,
}

fn file_format(path: &Path, text: Option<&str>) -> ConfigFormat {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        || text.is_some_and(|text| text.trim_start().starts_with('{'))
    {
        ConfigFormat::MoergoJson
    } else {
        ConfigFormat::Toml
    }
}

fn parse_text(text: &str, format: ConfigFormat) -> Result<RuntimeConfig> {
    match format {
        ConfigFormat::Toml => RuntimeConfig::from_toml(text),
        ConfigFormat::MoergoJson => runtime_config_from_moergo_json(text),
    }
}

fn parse(path: &Path) -> Result<RuntimeConfig> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    parse_text(&text, file_format(path, Some(&text)))
        .with_context(|| format!("could not parse {}", path.display()))
}

/// The state a file asks for, with every readable selector in it lowered
/// through the topology the connected keyboard advertises.
async fn desired_snapshot(client: &Client, config: RuntimeConfig) -> Result<Snapshot> {
    if !config.needs_topology() {
        return config.snapshot();
    }
    let semantic_lighting = config
        .lighting
        .as_ref()
        .is_some_and(LightingConfig::has_semantic_targets);
    let topology = client
        .read_lighting_key_topology()
        .await
        .context("could not read semantic key topology")?;
    let mut config = config;
    if semantic_lighting {
        if let Some(lighting) = config.lighting.take() {
            config.lighting = Some(lighting.resolve_semantic_targets(&topology)?);
        }
    }
    config.snapshot_with_topology(&topology)
}

async fn read_advanced_runtime_conditionals(
    client: &Client,
) -> Result<Vec<LightingAdvancedConditionalSceneCell>> {
    let mut last_error = None;
    for _ in 0..CONDITIONAL_READ_ATTEMPTS {
        match tokio::time::timeout(
            CONDITIONAL_READ_TIMEOUT,
            client.read_all_lighting_advanced_runtime_conditional_scenes(),
        )
        .await
        {
            Ok(Ok((_, cells))) => return Ok(cells),
            Ok(Err(error)) => last_error = Some(anyhow!(error)),
            Err(_) => last_error = Some(anyhow!("advanced conditional table read timed out")),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("advanced conditional table read failed")))
}

async fn read_extended_runtime_conditionals(
    client: &Client,
) -> Result<Vec<LightingExtendedConditionalSceneCell>> {
    let mut last_error = None;
    for _ in 0..CONDITIONAL_READ_ATTEMPTS {
        match tokio::time::timeout(
            CONDITIONAL_READ_TIMEOUT,
            client.read_all_lighting_extended_runtime_conditional_scenes(),
        )
        .await
        {
            Ok(Ok((_, cells))) => return Ok(cells),
            Ok(Err(error)) => last_error = Some(anyhow!(error)),
            Err(_) => {
                last_error = Some(anyhow!(
                    "extended conditional table did not answer within {:?}",
                    CONDITIONAL_READ_TIMEOUT
                ));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("extended conditional table read failed")))
}

async fn read_legacy_runtime_conditionals(
    client: &Client,
) -> Result<Vec<rynk::rmk_types::protocol::rynk::LightingConditionalSceneCell>> {
    let mut last_error = None;
    for _ in 0..CONDITIONAL_READ_ATTEMPTS {
        match tokio::time::timeout(
            CONDITIONAL_READ_TIMEOUT,
            client.read_all_lighting_runtime_conditional_scenes(),
        )
        .await
        {
            Ok(Ok((_, cells))) => return Ok(cells),
            Ok(Err(error)) => last_error = Some(anyhow!(error)),
            Err(_) => {
                last_error = Some(anyhow!(
                    "conditional table did not answer within {:?}",
                    CONDITIONAL_READ_TIMEOUT
                ));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("conditional table read failed")))
}

fn render(
    config: &RuntimeConfig,
    snapshot: &Snapshot,
    format: ConfigFormat,
    template: Option<&str>,
) -> Result<String> {
    match format {
        ConfigFormat::Toml => config.to_toml(),
        ConfigFormat::MoergoJson => snapshot_to_moergo_json(snapshot, Some(config), template),
    }
}

pub fn run(selector: &Selector, command: &ConfigCommand) -> Result<()> {
    if let ConfigCommand::Validate { file } = command {
        let config = parse(file)?;
        println!("{} is valid", file.display());
        // Saying so matters: an offline check cannot tell whether these cover
        // the keys the author meant, only that they are well formed.
        for deferred in config.deferred_bindings() {
            println!("{deferred} resolves against the connected keyboard");
        }
        return Ok(());
    }
    crate::rynk_client::run_config(selector, command)
}

pub async fn operate(client: &Client, command: &ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Validate { .. } => unreachable!("validate is offline"),
        ConfigCommand::Show { format } => {
            let snapshot = read_snapshot(client).await?;
            let config = RuntimeConfig::from_snapshot(&snapshot, None);
            print!("{}", render(&config, &snapshot, *format, None)?);
        }
        ConfigCommand::Pull { file, format } => {
            let snapshot = read_snapshot(client).await?;
            let old_text = std::fs::read_to_string(file).ok();
            let inferred = format.unwrap_or_else(|| file_format(file, old_text.as_deref()));
            let labels = old_text
                .as_deref()
                .and_then(|text| parse_text(text, file_format(file, Some(text))).ok());
            let mut config = RuntimeConfig::from_snapshot(&snapshot, labels.as_ref());
            config.retain_non_default_params(&snapshot);
            let template = (inferred == ConfigFormat::MoergoJson)
                .then_some(old_text.as_deref())
                .flatten();
            let text = render(&config, &snapshot, inferred, template)?;
            std::fs::write(file, text)
                .with_context(|| format!("could not write {}", file.display()))?;
            println!("pulled live runtime configuration into {}", file.display());
        }
        ConfigCommand::Diff { file, exact } => {
            let mut desired = desired_snapshot(client, parse(file)?).await?;
            if *exact {
                claim_every_behavior_table(&mut desired);
            }
            let live = read_snapshot(client).await?;
            if !print_diff(&desired, &live) {
                return Err(DiffFound.into());
            }
        }
        ConfigCommand::Apply {
            file,
            dry_run,
            exact,
        } => {
            let mut desired = desired_snapshot(client, parse(file)?).await?;
            if *exact {
                claim_every_behavior_table(&mut desired);
            }
            let before = read_snapshot(client).await?;
            let pending = differences(&desired, &before);
            if pending.is_empty() {
                println!("keyboard already matches {}", file.display());
                return Ok(());
            }
            for difference in &pending {
                println!("{difference}");
            }
            if *dry_run {
                println!("dry run: no changes written");
                return Ok(());
            }
            crate::rynk_client::require_maintenance_mode(client).await?;
            apply_snapshot(client, &desired, &before).await?;
            let after = read_snapshot(client).await?;
            let remaining = differences(&desired, &after);
            if !remaining.is_empty() {
                bail!("read-back verification failed:\n{}", remaining.join("\n"));
            }
            println!("applied and verified {}", file.display());
        }
    }
    Ok(())
}

async fn read_snapshot(client: &Client) -> Result<Snapshot> {
    let capabilities = client.get_capabilities().await?;
    let rows = capabilities.num_rows;
    let cols = capabilities.num_cols;
    let layer_size = usize::from(rows) * usize::from(cols);
    let actions = crate::rynk_client::read_all_actions(client, &capabilities).await?;
    let layers = actions
        .chunks(layer_size)
        .map(|actions| actions.to_vec())
        .collect::<Vec<_>>();

    let lighting_caps = client.get_lighting_capabilities().await?;
    let state = client.get_lighting_state().await?;
    let output_mode_state = if lighting_caps
        .features
        .contains(LightingFeatureFlags::OUTPUT_MODE)
    {
        Some(client.get_lighting_output_mode().await?)
    } else {
        None
    };
    let output_mode = output_mode_state
        .as_ref()
        .map_or(OutputModeConfig::AlwaysOn, |state| {
            output_mode_from_wire(state.mode)
        });
    let wake_layers = output_mode_state
        .as_ref()
        .map(|state| {
            (0..64)
                .filter(|layer| state.wake_layers & (1u64 << layer) != 0)
                .map(|layer| layer as u8)
                .collect()
        })
        .unwrap_or_default();
    let scene_status = client.get_lighting_scene_status().await?;
    let (_, scene_cells) = client.read_all_lighting_scenes().await?;
    // Firmware that predates the runtime conditional table reports nothing
    // rather than an empty table, so a file that names no rules does not read
    // as "delete what the board has".
    let conditional_scenes = if lighting_caps
        .features
        .contains(LightingFeatureFlags::RUNTIME_LAYER_INDICATOR_CONDITIONS)
    {
        Some(
            read_advanced_runtime_conditionals(client)
                .await?
                .into_iter()
                .map(conditional_scene_from_advanced_wire)
                .collect::<Result<Vec<_>>>()?,
        )
    } else if lighting_caps
        .features
        .contains(LightingFeatureFlags::RUNTIME_EFFECTS_CONDITIONS)
    {
        let cells = read_extended_runtime_conditionals(client).await?;
        Some(
            cells
                .into_iter()
                .map(conditional_scene_from_wire)
                .collect::<Vec<_>>(),
        )
    } else if lighting_caps
        .features
        .contains(LightingFeatureFlags::RUNTIME_CONDITIONAL_SCENES)
    {
        let cells = read_legacy_runtime_conditionals(client).await?;
        Some(
            cells
                .into_iter()
                .map(|cell| {
                    conditional_scene_from_wire(LightingExtendedConditionalSceneCell {
                        cell,
                        connection: None,
                        effects: None,
                    })
                })
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    let mut scenes = scene_cells
        .into_iter()
        .map(scene_from_wire)
        .collect::<Vec<_>>();
    scenes.sort();
    let (effects, extension_params) = if lighting_caps
        .features
        .contains(LightingFeatureFlags::EXTENSION_EFFECTS)
    {
        let extension = client.get_lighting_extension().await?;
        let effect_names = read_extension_names(client, LightingExtensionNameKind::Effects).await?;
        let palette_names =
            read_extension_names(client, LightingExtensionNameKind::Palettes).await?;
        let overlay = if lighting_caps
            .features
            .contains(LightingFeatureFlags::EXTENSION_LAYERING)
        {
            client.get_lighting_extension_layers().await?.overlay
        } else {
            None
        };
        let extension_params = read_extension_params(client, &effect_names).await?;
        (
            Some(effects_from_wire(
                extension.state,
                overlay,
                &effect_names,
                &palette_names,
                live_param_tables(extension_params.as_deref()),
            )?),
            extension_params,
        )
    } else {
        (None, None)
    };
    Ok(Snapshot {
        rows,
        cols,
        bluetooth_name: optional_endpoint(client.get_ble_name().await)?
            .map(|name| name.template.as_str().to_owned()),
        default_layer: client.get_default_layer().await?,
        layers,
        layer_names: read_layer_names(client, capabilities.num_layers).await?,
        behaviors: read_behaviors(client).await?,
        pointing: optional_endpoint(client.get_pointing_config().await)?,
        lighting: Some(LightingSnapshot {
            brightness: state.output_brightness,
            output_mode,
            wake_layers,
            scene_policy: scene_policy_from_wire(scene_status.policy),
            conditional_scenes,
            background: background_from_wire(state.background),
            effects,
            params: extension_params,
            scenes,
        }),
    })
}

/// Read every extension name of one kind as owned strings.
pub(crate) async fn read_extension_names(
    client: &Client,
    kind: LightingExtensionNameKind,
) -> Result<Vec<String>> {
    Ok(client
        .read_all_lighting_extension_names(kind)
        .await?
        .iter()
        .map(|name| name.as_str().to_owned())
        .collect())
}

/// Read the parameters of every effect that advertises any, or `None` when the
/// keyboard has no parameter surface at all. Effects without parameters are
/// omitted rather than recorded as empty lists.
pub(crate) async fn read_extension_params(
    client: &Client,
    effect_names: &[String],
) -> Result<Option<Vec<EffectParams>>> {
    let mut sets = Vec::new();
    for (index, effect) in effect_names.iter().enumerate() {
        let index = u8::try_from(index).context("effect index exceeds u8")?;
        let params = match read_effect_params(client, index).await {
            Ok(params) => params,
            Err(error) if params_unsupported(&error) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !params.is_empty() {
            sets.push(EffectParams {
                index,
                effect: effect.clone(),
                params,
            });
        }
    }
    Ok(Some(sets))
}

/// Page one effect's parameter list. Pages are shaped like extension-name
/// pages, so they are walked the same way: advance by the items returned and
/// stop at the advertised total.
async fn read_effect_params(client: &Client, effect: u8) -> Result<Vec<ParamSpec>, RynkHostError> {
    let mut params = Vec::new();
    let mut offset: u8 = 0;
    loop {
        let page = client
            .get_lighting_extension_params(LightingExtensionParamsRequest { effect, offset })
            .await?;
        if offset >= page.total {
            break;
        }
        if page.items.is_empty() || usize::from(offset) + page.items.len() > usize::from(page.total)
        {
            return Err(RynkHostError::InconsistentResponse {
                cmd: Cmd::GetLightingExtensionParams,
                reason: "parameter page is empty or extends beyond the advertised total",
            });
        }
        offset += page.items.len() as u8;
        params.extend(page.items.iter().map(|item| ParamSpec {
            name: item.name.as_str().to_owned(),
            min: item.min,
            max: item.max,
            default: item.default,
            value: item.value,
        }));
    }
    Ok(params)
}

/// Firmware without the parameter commands answers `UnknownCmd`, and firmware
/// whose lighting source advertises no parameter descriptor answers
/// `Unsupported`. Both mean the same thing to a host: there is nothing to read.
fn params_unsupported(error: &RynkHostError) -> bool {
    matches!(
        error,
        RynkHostError::Rejected(RynkError::UnknownCmd | RynkError::Unimplemented)
            | RynkHostError::LightingRejected(LightingError::Unsupported)
            | RynkHostError::Unsupported(..)
    )
}

/// Read the behavior tables a keymap cell addresses by index.
///
/// Firmware without these commands answers `UnknownCmd`, which reads as "this
/// keyboard has no such table" rather than as a failure, so an older device
/// still pulls and diffs its keymap.
/// One macro chunk, which the protocol fixes for both directions.
const MACRO_CHUNK: usize = rynk::rmk_types::constants::MACRO_DATA_SIZE;

async fn read_behaviors(client: &Client) -> Result<BehaviorSnapshot> {
    let capabilities = client.get_capabilities().await?;
    let options = optional_endpoint(client.get_behavior_options().await)?;
    let hold_trigger_positions =
        optional_endpoint(client.get_morse_hold_trigger_positions().await)?.map(|state| {
            state
                .positions
                .into_iter()
                .map(|position| moergo_config::HoldTriggerPosition {
                    profile: position.profile,
                    row: position.row,
                    col: position.col,
                })
                .collect::<Vec<_>>()
        });
    let morse_profiles = match client.read_morse_profile_state().await {
        Ok(state) => Some(
            state
                .entries
                .into_iter()
                .map(|entry| moergo_config::MorseProfileEntry {
                    index: entry.index,
                    name: entry.name.as_str().to_owned(),
                    profile: entry.profile,
                })
                .collect(),
        ),
        Err(error) if endpoint_unsupported(&error) => {
            let mut profiles = optional_endpoint(client.read_all_morse_profiles().await)?;
            if let (Some(profiles), Some(options)) = (&mut profiles, options) {
                let required = hold_trigger_positions
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .filter(|position| position.profile != u8::MAX)
                    .map(|position| usize::from(position.profile) + 1)
                    .max()
                    .unwrap_or_default();
                while profiles.len() > required
                    && profiles.last() == Some(&options.morse_default_profile)
                {
                    profiles.pop();
                }
            }
            profiles.map(|profiles| {
                profiles
                    .into_iter()
                    .enumerate()
                    .map(|(index, profile)| moergo_config::MorseProfileEntry {
                        index: index as u8,
                        name: format!("profile_{index:03}"),
                        profile,
                    })
                    .collect()
            })
        }
        Err(error) => return Err(error.into()),
    };
    let combos = if capabilities.max_combos == 0 {
        None
    } else {
        Some(client.read_all_combo_definitions().await?)
    };
    Ok(BehaviorSnapshot {
        config: optional_endpoint(client.get_behavior().await)?,
        options,
        morse_profiles,
        hold_trigger_positions,
        auto_mouse_layers: optional_endpoint(client.get_auto_mouse_layer_configs().await)?
            .map(|state| state.configs),
        morses: if capabilities.max_morse > 0 {
            Some(client.read_all_morses().await?)
        } else {
            None
        },
        combos,
        macros: if capabilities.macro_space_size > 0 {
            Some(read_macro_space(client).await?)
        } else {
            None
        },
        forks: if capabilities.max_forks > 0 {
            Some(read_all_forks(client).await?)
        } else {
            None
        },
    })
}

fn endpoint_unsupported(error: &RynkHostError) -> bool {
    matches!(
        error,
        RynkHostError::Rejected(RynkError::UnknownCmd | RynkError::Unimplemented)
            | RynkHostError::Unsupported(..)
    )
}

fn optional_endpoint<T>(result: std::result::Result<T, RynkHostError>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if endpoint_unsupported(&error) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn pointing_uses_keypad(config: &WirePointingConfig) -> bool {
    config
        .devices()
        .iter()
        .any(|entry| matches!(entry.mode, PointingMode::Keypad(_)))
        || config
            .overrides()
            .iter()
            .any(|entry| matches!(entry.mode, PointingMode::Keypad(_)))
}

fn pointing_uses_cursor_remap(config: &WirePointingConfig) -> bool {
    config
        .devices()
        .iter()
        .any(|entry| matches!(entry.mode, PointingMode::CursorRemap(_)))
        || config
            .overrides()
            .iter()
            .any(|entry| matches!(entry.mode, PointingMode::CursorRemap(_)))
}

fn require_keypad_capability(
    result: std::result::Result<PointingCapabilities, RynkHostError>,
) -> Result<()> {
    match result {
        Ok(capabilities) if capabilities.supports_keypad() => Ok(()),
        Ok(_) => bail!(
            "connected firmware does not support keypad pointing mode; flash updated firmware first"
        ),
        Err(error) if endpoint_unsupported(&error) => bail!(
            "connected firmware does not advertise keypad pointing support; flash updated firmware first"
        ),
        Err(error) => Err(error.into()),
    }
}

fn require_cursor_remap_capability(
    result: std::result::Result<PointingCapabilities, RynkHostError>,
) -> Result<()> {
    match result {
        Ok(capabilities) if capabilities.supports_cursor_remap() => Ok(()),
        Ok(_) => bail!(
            "connected firmware does not support cursor button remapping; flash updated firmware first"
        ),
        Err(error) if endpoint_unsupported(&error) => bail!(
            "connected firmware does not advertise cursor button remapping; flash updated firmware first"
        ),
        Err(error) => Err(error.into()),
    }
}

/// Read every physical layer slot's persistent metadata.
///
/// Firmware without the layer-metadata endpoints reads as `None`, which leaves
/// names out of the diff rather than reporting every layer as changed.
async fn read_layer_names(client: &Client, num_layers: u8) -> Result<Option<Vec<LayerMetadata>>> {
    let mut slots = Vec::with_capacity(usize::from(num_layers));
    for layer in 0..num_layers {
        match optional_endpoint(client.get_layer_metadata(layer).await)? {
            Some(metadata) => slots.push(metadata),
            None => return Ok(None),
        }
    }
    Ok(Some(slots))
}

/// Read the fork table a slot at a time.
///
/// The protocol has no bulk form for forks, so this walks to the capacity the
/// device advertises. Any rejection means the firmware has no fork table, which
/// the caller reads as "nothing to manage" rather than as a failure.
async fn read_all_forks(client: &Client) -> Result<Vec<rynk::rmk_types::fork::Fork>> {
    let capabilities = client.get_capabilities().await?;
    let mut forks = Vec::new();
    for index in 0..capabilities.max_forks {
        forks.push(client.get_fork(index).await?);
    }
    Ok(forks)
}

/// Read macro space by walking it a chunk at a time.
///
/// Chunks come back full size and zero-filled past the end, so there is no
/// short read to stop on; the walk stops when a chunk adds nothing but padding.
async fn read_macro_space(client: &Client) -> Result<Vec<u8>> {
    let mut space = Vec::new();
    let mut offset = 0u16;
    loop {
        let chunk = client.get_macro(offset).await?;
        if chunk.data.is_empty() || chunk.data.iter().all(|byte| *byte == 0) {
            break;
        }
        space.extend_from_slice(&chunk.data);
        offset = offset
            .checked_add(u16::try_from(chunk.data.len()).context("macro chunk too large")?)
            .context("macro space offset overflowed")?;
    }
    // Trailing padding is not part of any sequence, but retain the final zero:
    // it terminates the last encoded macro and belongs to the logical macro
    // space compared against a configuration snapshot.
    while space.ends_with(&[0, 0]) {
        space.pop();
    }
    Ok(space)
}

/// Make a snapshot speak for every behavior table, so one it did not mention
/// reads as empty rather than as silence.
///
/// This is the whole of `--exact`: the merge semantics live in the `Option`s, and
/// filling them in is what turns "leave this alone" into "there is nothing here".
fn claim_every_behavior_table(snapshot: &mut moergo_config::Snapshot) {
    let behaviors = &mut snapshot.behaviors;
    behaviors.morses.get_or_insert_with(Vec::new);
    behaviors.morse_profiles.get_or_insert_with(Vec::new);
    behaviors
        .hold_trigger_positions
        .get_or_insert_with(Vec::new);
    behaviors.auto_mouse_layers.get_or_insert_with(Vec::new);
    behaviors.combos.get_or_insert_with(Vec::new);
    behaviors.forks.get_or_insert_with(Vec::new);
    behaviors.macros.get_or_insert_with(Vec::new);
}

/// One past the last slot holding anything, so a table only has to be written as
/// far as it was actually used.
fn last_populated<T>(slots: &[T], populated: impl Fn(&T) -> bool) -> usize {
    slots
        .iter()
        .rposition(populated)
        .map_or(0, |index| index + 1)
}

/// Write the behavior tables, skipping any the source is silent about.
async fn apply_behaviors(
    client: &Client,
    desired: &BehaviorSnapshot,
    before: &BehaviorSnapshot,
    macro_chunk: u16,
) -> Result<()> {
    if let Some(config) = desired.config {
        if before.config != Some(config) {
            client
                .set_behavior(config)
                .await
                .context("could not write global behavior timing")?;
        }
    }
    if let Some(options) = desired.options {
        if before.options != Some(options) {
            client
                .set_behavior_options(options)
                .await
                .context("could not write global behavior options")?;
        }
    }
    if let Some(profiles) = &desired.morse_profiles {
        if before.morse_profiles.as_deref() != Some(profiles.as_slice()) {
            let mut writes = 0usize;
            let mut unsupported = false;
            for entry in profiles {
                if before
                    .morse_profiles
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .any(|old| old == entry)
                {
                    continue;
                }
                let name = MorseProfileName::try_from(entry.name.as_str()).map_err(|_| {
                    anyhow::anyhow!("morse profile name '{}' is too long", entry.name)
                })?;
                if let Err(error) = client
                    .set_morse_profile_entry(SetMorseProfileEntryRequest {
                        entry: WireMorseProfileEntry {
                            index: entry.index,
                            name,
                            profile: entry.profile,
                        },
                    })
                    .await
                {
                    if writes == 0 && params_unsupported(&error) {
                        unsupported = true;
                        break;
                    }
                    return Err(error).context("could not write named morse profile");
                }
                writes += 1;
            }
            if !unsupported {
                for old in before.morse_profiles.as_deref().unwrap_or_default() {
                    if profiles.iter().any(|entry| entry.index == old.index) {
                        continue;
                    }
                    if let Err(error) = client.delete_morse_profile(old.index).await {
                        if writes == 0 && params_unsupported(&error) {
                            unsupported = true;
                            break;
                        }
                        return Err(error).context("could not delete named morse profile");
                    }
                    writes += 1;
                }
            }
            if unsupported {
                let default_profile = desired
                    .options
                    .or(before.options)
                    .map(|options| options.morse_default_profile)
                    .context("older firmware needs a default morse profile")?;
                let length = profiles
                    .iter()
                    .map(|entry| usize::from(entry.index) + 1)
                    .max()
                    .unwrap_or_default();
                let mut dense = vec![default_profile; length];
                for entry in profiles {
                    dense[usize::from(entry.index)] = entry.profile;
                }
                client
                    .write_all_morse_profiles(dense)
                    .await
                    .context("could not write morse profiles")?;
            }
        }
    }
    if let Some(positions) = &desired.hold_trigger_positions {
        if before.hold_trigger_positions.as_ref() != Some(positions) {
            client
                .set_morse_hold_trigger_positions(SetMorseHoldTriggerPositionsRequest {
                    positions: positions
                        .iter()
                        .map(
                            |position| rynk::rmk_types::protocol::rynk::MorseHoldTriggerPosition {
                                profile: position.profile,
                                row: position.row,
                                col: position.col,
                            },
                        )
                        .collect(),
                })
                .await
                .context("could not write morse hold trigger positions")?;
        }
    }
    if let Some(configs) = &desired.auto_mouse_layers {
        if before.auto_mouse_layers.as_ref() != Some(configs) {
            client
                .set_auto_mouse_layer_configs(SetAutoMouseLayerConfigsRequest {
                    configs: configs.clone(),
                })
                .await
                .context("could not write auto mouse layers")?;
        }
    }
    // The bulk writers write exactly the slots they are given, so a table that
    // shrank would keep its tail. Pad to what the keyboard currently holds and
    // the surplus is overwritten with empty slots.
    if let Some(morses) = &desired.morses {
        let mut morses = morses.clone();
        // Pad only as far as the last slot the keyboard actually has something
        // in. Padding to full capacity rewrote sixty-four slots to change one,
        // which is a lot of flash traffic for no effect.
        let held = last_populated(before.morses.as_deref().unwrap_or_default(), |morse| {
            !morse.actions.is_empty()
        });
        morses.resize(morses.len().max(held), Default::default());
        if before.morses.as_deref() != Some(morses.as_slice()) {
            client
                .write_all_morses(morses)
                .await
                .context("could not write the morse table")?;
        }
    }
    if let Some(combos) = &desired.combos {
        let mut combos = combos.clone();
        let held = last_populated(before.combos.as_deref().unwrap_or_default(), |combo| {
            !combo.is_empty()
        });
        let empty = rynk::rmk_types::combo::ComboDefinition::empty();
        combos.resize(combos.len().max(held), empty);
        if before.combos.as_deref() != Some(combos.as_slice()) {
            client
                .write_all_combo_definitions(combos)
                .await
                .context("could not write the combo table")?;
        }
    }
    if let Some(forks) = &desired.forks {
        // No bulk form for forks, so write slot by slot. Slots past the end of
        // the file are cleared rather than left behind, or a shrinking table
        // would keep its tail firing.
        let present = before.forks.as_deref().unwrap_or_default();
        let empty = rynk::rmk_types::fork::Fork::default();
        for index in 0..forks.len().max(present.len()) {
            let wanted = forks.get(index).unwrap_or(&empty);
            if present.get(index) == Some(wanted) {
                continue;
            }
            let slot = u8::try_from(index).context("more forks than the protocol can address")?;
            client
                .set_fork(slot, *wanted)
                .await
                .with_context(|| format!("could not write fork {slot}"))?;
        }
    }
    if let Some(macros) = &desired.macros {
        if before.macros.as_ref() != Some(macros) {
            write_macro_space(client, macros, macro_chunk).await?;
        }
    }
    Ok(())
}

async fn write_macro_space(client: &Client, space: &[u8], macro_chunk: u16) -> Result<()> {
    // Chunk by what the device advertises, not by this build's constant. The two
    // are generated differently on purpose — a host's `MACRO_DATA_SIZE` is the
    // protocol ceiling so it can talk to any firmware, while a firmware's is its
    // own `protocol_macro_chunk_size`. Sending a ceiling-sized chunk to firmware
    // built with a smaller one overruns the vec it decodes into, and the write
    // comes back as a bare `Malformed`.
    let chunk_size = usize::from(macro_chunk).clamp(1, MACRO_CHUNK);
    // One extra terminator so a shorter sequence set does not leave the tail
    // of a longer one behind to be parsed as another macro.
    let mut payload = space.to_vec();
    payload.push(0);
    for (index, chunk) in payload.chunks(chunk_size).enumerate() {
        let offset = u16::try_from(index * chunk_size).context("macro space is too large")?;
        let data = rynk::rmk_types::protocol::rynk::MacroData {
            data: heapless::Vec::from_slice(chunk)
                .map_err(|_| anyhow::anyhow!("macro chunk exceeds the protocol's chunk size"))?,
        };
        client
            .set_macro(offset, data)
            .await
            .context("could not write macro space")?;
    }
    Ok(())
}

/// Cells per keymap write before waiting for the firmware to persist them.
///
/// The firmware hands every written cell to its flash task through a bounded
/// channel and only accepts a page it can queue whole, answering `Busy`
/// otherwise (`rmk/src/host/context.rs`, `wait_for_persist_room`); a page
/// longer than the channel falls back to streaming through it and stalls the
/// session for the length of any flash page migration in the middle. RMK's
/// default channel is four deep, so four cells per page keep every write
/// reply instant on any firmware and move all of the waiting into
/// [`persist_barrier`] and [`write_page`]'s `Busy` retries, where it is timed
/// and reported.
///
/// `MOERGO_PERSIST_BATCH` overrides the page size (still capped by the
/// firmware's `max_bulk_keys`).
const PERSIST_BATCH_DEFAULT: usize = 4;
/// How long a page may keep drawing `Busy` before the apply gives up. The
/// firmware waits about half a second for flash-queue room before answering,
/// and a storage page migration holds the queue for tens of seconds.
const PERSIST_BUSY_LIMIT: Duration = Duration::from_secs(300);
const PERSIST_BUSY_RETRY_DELAY: Duration = Duration::from_millis(100);
/// A single persist wait at or beyond this is reported as storage pressure:
/// it means sequential-storage closed a full page and migrated the previous
/// one's live items through MPSL flash timeslots.
const PERSIST_SLOW_THRESHOLD: Duration = Duration::from_secs(2);

/// How keymap cells are written and confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WritePolicy {
    /// Cells per write request.
    batch: usize,
    /// Rewrite whole layers (the pre-diff behaviour) instead of only the
    /// cells that differ from the keyboard. `MOERGO_WRITE_WHOLE_LAYERS=1`.
    whole_layers: bool,
}

impl WritePolicy {
    fn from_env(capabilities: &rynk::rmk_types::protocol::rynk::DeviceCapabilities) -> Self {
        let bulk_cap = if capabilities.bulk_transfer_supported {
            usize::from(capabilities.max_bulk_keys).max(1)
        } else {
            usize::MAX
        };
        let batch = std::env::var("MOERGO_PERSIST_BATCH")
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|value| *value >= 1)
            .unwrap_or(PERSIST_BATCH_DEFAULT)
            .min(bulk_cap);
        let whole_layers = std::env::var("MOERGO_WRITE_WHOLE_LAYERS")
            .map(|value| matches!(value.trim(), "1" | "true" | "yes"))
            .unwrap_or(false);
        Self {
            batch,
            whole_layers,
        }
    }
}

/// What a `config apply` had to push to flash, and how long flash made it wait.
#[derive(Debug, Default)]
struct PersistStats {
    cells: usize,
    pages: usize,
    layers: usize,
    barriers: usize,
    waited: Duration,
    longest: Duration,
    /// `Busy` answers to page writes, each meaning the flash queue had no room
    /// for the page yet.
    busy: usize,
    /// `(layer, flat offset)` of the write behind the longest wait.
    longest_at: Option<(u8, usize)>,
    /// The firmware has no layer-metadata endpoint, so persistence could not
    /// be confirmed; writes are still queued in order.
    barrier_unsupported: bool,
}

impl PersistStats {
    fn summary(&self) -> String {
        let mut out = format!(
            "persisted {} cell(s) in {} write(s) across {} layer(s)",
            self.cells, self.pages, self.layers
        );
        if self.barrier_unsupported {
            out.push_str("; firmware cannot confirm persistence (no layer metadata endpoint)");
            return out;
        }
        out.push_str(&format!(
            "; waited {:.1}s for flash in total, longest {:.1}s",
            self.waited.as_secs_f64(),
            self.longest.as_secs_f64()
        ));
        if let Some((layer, offset)) = self.longest_at {
            out.push_str(&format!(" (layer {layer}, offset {offset})"));
        }
        if self.busy > 0 {
            out.push_str(&format!(
                "; {} write(s) answered Busy and were retried",
                self.busy
            ));
        }
        out
    }

    fn pressure_hint(&self) -> Option<String> {
        (self.longest >= PERSIST_SLOW_THRESHOLD).then(|| {
            format!(
                "flash waits of {:.0}s+ mean sequential-storage is migrating nearly full pages: \
                 the settings partition is close to capacity. Every rebuild re-stores all compiled \
                 layers, so grow `[storage] num_sectors` in the board's keyboard.toml before the \
                 store fills and writes start failing silently.",
                PERSIST_SLOW_THRESHOLD.as_secs_f64()
            )
        })
    }
}

/// Contiguous runs of flat cell indices where `wanted` differs from `present`.
///
/// A keyboard whose layer could not be read (`present` empty or the wrong
/// length) gets the whole layer, since nothing can be assumed about it.
fn changed_runs(
    wanted: &[rynk::rmk_types::action::KeyAction],
    present: &[rynk::rmk_types::action::KeyAction],
) -> Vec<std::ops::Range<usize>> {
    if wanted.is_empty() {
        return Vec::new();
    }
    if present.len() != wanted.len() {
        return std::iter::once(0..wanted.len()).collect();
    }
    let mut runs: Vec<std::ops::Range<usize>> = Vec::new();
    for (index, (want, have)) in wanted.iter().zip(present).enumerate() {
        if want == have {
            continue;
        }
        match runs.last_mut() {
            Some(run) if run.end == index => run.end = index + 1,
            _ => runs.push(index..index + 1),
        }
    }
    runs
}

/// Split runs into pages of at most `batch` cells.
fn pages_of(runs: &[std::ops::Range<usize>], batch: usize) -> Vec<std::ops::Range<usize>> {
    let batch = batch.max(1);
    runs.iter()
        .flat_map(|run| {
            run.clone()
                .step_by(batch)
                .map(move |start| start..(start + batch).min(run.end))
        })
        .collect()
}

/// Wait until every cell queued so far has reached flash.
///
/// `GetLayerMetadata` is served through the same FIFO channel as the keymap
/// writes (`rmk/src/storage/mod.rs`, `read_layer_metadata`), so its reply
/// arrives only after the flash task has finished everything queued before
/// it. That makes it the one request whose latency measures persistence.
async fn persist_barrier(
    client: &Client,
    layer: u8,
    offset: usize,
    stats: &mut PersistStats,
) -> Result<()> {
    if stats.barrier_unsupported {
        return Ok(());
    }
    let started = std::time::Instant::now();
    match client.get_layer_metadata(layer).await {
        Ok(_) => {}
        Err(error) if endpoint_unsupported(&error) => {
            stats.barrier_unsupported = true;
            return Ok(());
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!("waiting for layer {layer} cells from offset {offset} to reach flash")
            })
        }
    }
    let waited = started.elapsed();
    stats.barriers += 1;
    stats.waited += waited;
    if waited > stats.longest {
        stats.longest = waited;
        stats.longest_at = Some((layer, offset));
    }
    if waited >= PERSIST_SLOW_THRESHOLD {
        eprintln!(
            "  layer {layer}: flash took {:.1}s to absorb the write at offset {offset} \
             (page migration in the settings partition)",
            waited.as_secs_f64()
        );
    }
    Ok(())
}

/// Run one write, sending it again while the firmware answers `Busy`: its
/// flash queue has no room for the page yet, most likely because a storage
/// page migration is holding it. Every `Busy` is counted in `stats`.
async fn write_until_accepted(
    stats: &mut PersistStats,
    mut write: impl AsyncFnMut() -> Result<(), RynkHostError>,
) -> Result<(), RynkHostError> {
    let started = std::time::Instant::now();
    loop {
        match write().await {
            Err(RynkHostError::Rejected(RynkError::Busy))
                if started.elapsed() < PERSIST_BUSY_LIMIT =>
            {
                stats.busy += 1;
                tokio::time::sleep(PERSIST_BUSY_RETRY_DELAY).await;
            }
            result => return result,
        }
    }
}

/// Write one page of cells starting at flat `offset`.
async fn write_page(
    client: &Client,
    capabilities: &rynk::rmk_types::protocol::rynk::DeviceCapabilities,
    layer: u8,
    offset: usize,
    cells: &[rynk::rmk_types::action::KeyAction],
    stats: &mut PersistStats,
) -> Result<()> {
    let cols = usize::from(capabilities.num_cols).max(1);
    if capabilities.bulk_transfer_supported {
        let request = SetKeymapBulkRequest {
            layer,
            start_row: (offset / cols) as u8,
            start_col: (offset % cols) as u8,
            actions: cells.to_vec(),
        };
        return write_until_accepted(stats, async || {
            client.set_keymap_bulk(request.clone()).await
        })
        .await
        .with_context(|| format!("writing layer {layer} from offset {offset}"));
    }
    for (index, action) in cells.iter().copied().enumerate() {
        let flat = offset + index;
        let row = (flat / cols) as u8;
        let col = (flat % cols) as u8;
        write_until_accepted(stats, async || {
            client.set_key(layer, row, col, action).await
        })
        .await
        .with_context(|| format!("writing layer {layer} r{row},c{col}"))?;
    }
    Ok(())
}

/// Write the cells of one layer that differ from what the keyboard holds, in
/// small pages, waiting for flash after each page.
///
/// Only changed cells go out: every written cell becomes a new item in the
/// firmware's sequential-storage map whether or not its value changed, and
/// that map is what fills up and forces slow page migrations. Rewriting a
/// whole 84-cell layer for one edit is pure churn.
///
/// Pages are short (see [`PERSIST_BATCH_DEFAULT`]) so a write reply never
/// waits on flash; the wait is taken, timed, and reported by
/// [`persist_barrier`] instead. Firmware without bulk transfer falls back to
/// per-key writes with the same pacing.
async fn write_layer(
    client: &Client,
    capabilities: &rynk::rmk_types::protocol::rynk::DeviceCapabilities,
    layer: u8,
    wanted: &[rynk::rmk_types::action::KeyAction],
    present: &[rynk::rmk_types::action::KeyAction],
    policy: WritePolicy,
    stats: &mut PersistStats,
) -> Result<()> {
    let runs = if policy.whole_layers {
        std::iter::once(0..wanted.len()).collect()
    } else {
        changed_runs(wanted, present)
    };
    let pages = pages_of(&runs, policy.batch);
    if pages.is_empty() {
        return Ok(());
    }
    stats.layers += 1;
    for page in pages {
        write_page(
            client,
            capabilities,
            layer,
            page.start,
            &wanted[page.clone()],
            stats,
        )
        .await?;
        stats.pages += 1;
        stats.cells += page.len();
        persist_barrier(client, layer, page.start, stats).await?;
    }
    Ok(())
}

async fn apply_snapshot(client: &Client, desired: &Snapshot, before: &Snapshot) -> Result<()> {
    let lighting_features = if desired.lighting.is_some() {
        Some(client.get_lighting_capabilities().await?.features)
    } else {
        None
    };
    if let Some(lighting) = &desired.lighting {
        require_layer_conditions_capability(lighting, lighting_features.unwrap())?;
    }
    let capabilities = client.get_capabilities().await?;
    if desired.rows != capabilities.num_rows || desired.cols != capabilities.num_cols {
        bail!(
            "configuration is {}x{}, but device reports {}x{}",
            desired.rows,
            desired.cols,
            capabilities.num_rows,
            capabilities.num_cols
        );
    }
    if desired.layers.len() > usize::from(capabilities.num_layers) {
        bail!(
            "configuration has {} layers but device supports {}",
            desired.layers.len(),
            capabilities.num_layers
        );
    }
    if let Some(name) = &desired.bluetooth_name {
        if before.bluetooth_name.as_ref() != Some(name) {
            client
                .set_ble_name(&BleName {
                    template: heapless::String::try_from(name.as_str())
                        .context("bluetooth_name exceeds the firmware limit")?,
                })
                .await
                .context("could not write bluetooth name")?;
        }
    }
    // Before the keymap: a cell holding `TD(n)` or `TriggerMacro(n)` addresses
    // a table slot by index, so the tables have to be in place before any key
    // can point at them.
    apply_behaviors(
        client,
        &desired.behaviors,
        &before.behaviors,
        capabilities.macro_chunk_size,
    )
    .await?;

    // A source file owns the layers it lists. Fixed-capacity trailing layers
    // remain untouched rather than being destructively cleared, which is why this
    // writes layer by layer rather than handing the whole keymap to
    // `write_all_keymap`.
    let policy = WritePolicy::from_env(&capabilities);
    let mut persist = PersistStats::default();
    for layer in 0..u8::try_from(desired.layers.len()).context("too many configured layers")? {
        let wanted = &desired.layers[usize::from(layer)];
        let present = before
            .layers
            .get(usize::from(layer))
            .map_or(&[][..], Vec::as_slice);
        if wanted == present {
            continue;
        }
        write_layer(
            client,
            &capabilities,
            layer,
            wanted,
            present,
            policy,
            &mut persist,
        )
        .await?;
    }
    if persist.cells > 0 {
        eprintln!("{}", persist.summary());
        if let Some(hint) = persist.pressure_hint() {
            eprintln!("note: {hint}");
        }
    }
    if desired.default_layer != before.default_layer {
        client.set_default_layer(desired.default_layer).await?;
    }
    // Names follow the keymap so a slot is already occupied by the time it is
    // labelled. Trailing slots the file does not list keep their metadata,
    // matching how their keys are left alone.
    if let Some(wanted) = &desired.layer_names {
        for (layer, metadata) in wanted.iter().enumerate() {
            let present = before
                .layer_names
                .as_ref()
                .and_then(|names| names.get(layer));
            if present == Some(metadata) {
                continue;
            }
            let layer = u8::try_from(layer).context("too many configured layers")?;
            client
                .set_layer_metadata(layer, metadata.clone())
                .await
                .with_context(|| format!("could not write layer {layer} name"))?;
        }
    }
    if let Some(wanted) = desired.pointing {
        let differs = before.pointing.as_ref().is_none_or(|present| {
            wanted.devices() != present.devices() || wanted.overrides() != present.overrides()
        });
        if differs {
            if pointing_uses_keypad(&wanted) {
                require_keypad_capability(client.get_pointing_capabilities().await)?;
            }
            if pointing_uses_cursor_remap(&wanted) {
                require_cursor_remap_capability(client.get_pointing_capabilities().await)?;
            }
            let mut next = wanted;
            next.revision = before.pointing.map_or(0, |present| present.revision);
            client
                .set_pointing_config(next)
                .await
                .context("could not write pointing configuration")?;
        }
    }

    if let Some(wanted) = &desired.lighting {
        let present = before
            .lighting
            .as_ref()
            .context("device has no lighting state")?;
        if wanted.output_mode != present.output_mode {
            let revision = client.get_lighting_state().await?.revision;
            client
                .set_lighting_output_mode(SetLightingOutputModeRequest {
                    expected_revision: revision,
                    mode: output_mode_to_wire(wanted.output_mode),
                })
                .await?;
        }
        if wanted.wake_layers != present.wake_layers {
            let layers = wanted
                .wake_layers
                .iter()
                .fold(0u64, |mask, layer| mask | (1u64 << layer));
            let revision = client.get_lighting_state().await?.revision;
            client
                .set_lighting_wake_layers(SetLightingWakeLayersRequest {
                    expected_revision: revision,
                    layers,
                })
                .await?;
        }
        if wanted.brightness != present.brightness || wanted.background != present.background {
            let state = client.get_lighting_state().await?;
            client
                .set_lighting_state(SetLightingStateRequest {
                    expected_revision: state.revision,
                    state: LightingMutableState {
                        output_enabled: state.output_enabled,
                        output_brightness: wanted.brightness,
                        background: background_to_wire(&wanted.background),
                    },
                })
                .await?;
        }
        let selection_differs = wanted.effects.as_ref().map(EffectsConfig::selection)
            != present.effects.as_ref().map(EffectsConfig::selection);
        if selection_differs {
            let wanted = wanted
                .effects
                .as_ref()
                .context("cannot remove a firmware-provided effects extension")?;
            let effect_names =
                read_extension_names(client, LightingExtensionNameKind::Effects).await?;
            let palette_names =
                read_extension_names(client, LightingExtensionNameKind::Palettes).await?;
            let (state, overlay) = effects_to_wire(wanted, &effect_names, &palette_names)?;
            let revision = client.get_lighting_state().await?.revision;
            client
                .set_lighting_extension_state(SetLightingExtensionStateRequest {
                    expected_revision: revision,
                    state,
                })
                .await?;
            if wanted.overlay.is_some()
                || present
                    .effects
                    .as_ref()
                    .and_then(|effects| effects.overlay.as_ref())
                    .is_some()
            {
                let revision = client.get_lighting_state().await?.revision;
                client
                    .set_lighting_extension_layers(SetLightingExtensionLayersRequest {
                        expected_revision: revision,
                        overlay,
                    })
                    .await?;
            }
        }
        if let Some(effects) = wanted.effects.as_ref().filter(|it| !it.params.is_empty()) {
            apply_params(client, &effects.params, present.params.as_deref()).await?;
        }
        if wanted.scene_policy != present.scene_policy {
            let status = client.get_lighting_scene_status().await?;
            client
                .set_lighting_layer_policy(SetLightingLayerPolicyRequest {
                    expected_revision: status.revision,
                    policy: scene_policy_to_wire(wanted.scene_policy),
                })
                .await?;
        }
        // Order carries meaning here, so this compares and writes the table as
        // a sequence rather than as a set of addressable cells.
        if let Some(wanted_conditional) = wanted.conditional_scenes.as_ref() {
            match present.conditional_scenes.as_ref() {
                None if wanted_conditional.is_empty() => {}
                None => bail!(
                    "file configures {} conditional lighting rule(s) but the keyboard does not expose a runtime conditional table",
                    wanted_conditional.len()
                ),
                Some(live) if live == wanted_conditional => {}
                Some(_) => {
                    let status = client.get_lighting_runtime_conditional_scene_status().await?;
                    let features = lighting_features.unwrap();
                    if features.contains(LightingFeatureFlags::RUNTIME_LAYER_INDICATOR_CONDITIONS) {
                        let cells = wanted_conditional
                            .iter()
                            .map(conditional_scene_to_advanced_wire)
                            .collect::<Result<Vec<_>>>()?;
                        client
                            .replace_all_lighting_advanced_runtime_conditional_scenes(
                                status.revision,
                                &cells,
                            )
                            .await?;
                    } else if features.contains(LightingFeatureFlags::RUNTIME_EFFECTS_CONDITIONS) {
                        let cells = wanted_conditional
                            .iter()
                            .map(conditional_scene_to_wire)
                            .collect::<Result<Vec<_>>>()?;
                        client
                            .replace_all_lighting_extended_runtime_conditional_scenes(
                                status.revision,
                                &cells,
                            )
                            .await?;
                    } else {
                        if let Some(gated) = wanted_conditional
                            .iter()
                            .position(|c| c.connection.is_some() || c.effects.is_some())
                        {
                            bail!(
                                "conditional rule {gated} names a connection or effects condition but the keyboard's firmware predates the extended conditional cell"
                            );
                        }
                        let legacy = wanted_conditional
                            .iter()
                            .map(|cell| conditional_scene_to_wire(cell).map(|cell| cell.cell))
                            .collect::<Result<Vec<_>>>()?;
                        client
                            .replace_all_lighting_runtime_conditional_scenes(status.revision, &legacy)
                            .await?;
                    }
                }
            }
        }
        if wanted.scenes != present.scenes {
            let state = client.get_lighting_state().await?;
            let cells = wanted
                .scenes
                .iter()
                .map(scene_to_wire)
                .collect::<Result<Vec<_>>>()?;
            client
                .replace_all_lighting_scenes(state.revision, &cells)
                .await?;
        }
    }
    Ok(())
}

fn require_layer_conditions_capability(
    lighting: &LightingSnapshot,
    features: LightingFeatureFlags,
) -> Result<()> {
    if !features.contains(LightingFeatureFlags::RUNTIME_LAYER_INDICATOR_CONDITIONS)
        && lighting.conditional_scenes.as_ref().is_some_and(|cells| {
            cells
                .iter()
                .any(|cell| cell.layers.is_some() || cell.indicators.is_some())
        })
    {
        bail!("configuration uses layer-set or host lock-indicator conditions but the keyboard does not advertise advanced conditional-scene support");
    }
    Ok(())
}

/// Write the parameters a file lists, resolving names against what the
/// keyboard advertises. Parameters that already hold the wanted value are left
/// alone, and parameters the file does not mention are never touched.
async fn apply_params(
    client: &Client,
    wanted: &BTreeMap<String, BTreeMap<String, u8>>,
    advertised: Option<&[EffectParams]>,
) -> Result<()> {
    for write in params_to_writes(wanted, advertised)? {
        if write.value == write.current {
            continue;
        }
        let revision = client.get_lighting_state().await?.revision;
        client
            .set_lighting_extension_param(SetLightingExtensionParamRequest {
                expected_revision: revision,
                effect: write.effect,
                index: write.index,
                value: write.value,
            })
            .await
            .with_context(|| format!("writing parameter '{}'", write.label))?;
    }
    Ok(())
}

fn print_diff(desired: &Snapshot, live: &Snapshot) -> bool {
    let differences = differences(desired, live);
    if differences.is_empty() {
        println!("keyboard matches configuration");
        true
    } else {
        for difference in &differences {
            println!("{difference}");
        }
        println!("{} difference(s)", differences.len());
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rynk::rmk_types::pointing::{CursorRemapConfig, KeypadConfig};
    use rynk::rmk_types::protocol::rynk::{
        PointingDeviceConfig, POINTING_MODE_CURSOR_REMAP, POINTING_MODE_KEYPAD,
    };

    fn action(code: u8) -> rynk::rmk_types::action::KeyAction {
        use rynk::rmk_types::action::{Action, KeyAction};
        use rynk::rmk_types::keycode::KeyCode;
        KeyAction::Single(Action::Key(KeyCode::Hid(code.into())))
    }

    #[test]
    fn changed_runs_groups_adjacent_differences() {
        let present = vec![
            action(4),
            action(5),
            action(6),
            action(7),
            action(8),
            action(9),
        ];
        let mut wanted = present.clone();
        wanted[1] = action(20);
        wanted[2] = action(21);
        wanted[5] = action(22);
        assert_eq!(changed_runs(&wanted, &present), vec![1..3, 5..6]);
        assert!(changed_runs(&present, &present).is_empty());
    }

    #[test]
    fn changed_runs_rewrites_unreadable_layers() {
        let wanted = vec![action(4), action(5), action(6)];
        assert_eq!(changed_runs(&wanted, &[]), vec![0..3]);
        assert_eq!(changed_runs(&wanted, &wanted[..2]), vec![0..3]);
        assert!(changed_runs(&[], &[]).is_empty());
    }

    #[test]
    fn pages_never_exceed_the_batch_or_cross_a_run() {
        let pages = pages_of(&[0..10, 12..13], 4);
        assert_eq!(pages, vec![0..4, 4..8, 8..10, 12..13]);
        assert_eq!(pages_of(&[0..3], 0), vec![0..1, 1..2, 2..3]);
    }

    #[test]
    fn persist_stats_report_pressure_only_for_slow_waits() {
        let mut stats = PersistStats {
            cells: 8,
            pages: 2,
            layers: 1,
            barriers: 2,
            waited: Duration::from_millis(300),
            longest: Duration::from_millis(200),
            longest_at: Some((2, 4)),
            busy: 0,
            barrier_unsupported: false,
        };
        assert!(stats.pressure_hint().is_none());
        assert!(stats.summary().contains("8 cell(s) in 2 write(s)"));
        assert!(!stats.summary().contains("Busy"));
        stats.busy = 3;
        assert!(stats.summary().contains("3 write(s) answered Busy"));
        stats.longest = PERSIST_SLOW_THRESHOLD;
        assert!(stats.pressure_hint().is_some());
        stats.barrier_unsupported = true;
        assert!(stats.summary().contains("cannot confirm persistence"));
    }

    #[test]
    fn layer_conditions_require_advanced_firmware_before_apply() {
        let config: LightingConfig = toml::from_str(
            r##"
            brightness = 100
            output_mode = "always-on"
            scene_policy = "active-stack"
            [[conditional_scene]]
            led = 1
            color = "#ff00ff"
            layers = { active = [2, 4], inactive = [3] }
            [background]
            enabled = false
            hue = 0
            saturation = 0
            value = 0
            speed = 0
            mode = "solid"
        "##,
        )
        .unwrap();
        let mut snapshot = config.snapshot().unwrap();
        let legacy = LightingFeatureFlags(LightingFeatureFlags::RUNTIME_EFFECTS_CONDITIONS);
        assert!(require_layer_conditions_capability(&snapshot, legacy).is_err());
        assert!(require_layer_conditions_capability(
            &snapshot,
            LightingFeatureFlags(LightingFeatureFlags::RUNTIME_LAYER_INDICATOR_CONDITIONS)
        )
        .is_ok());
        snapshot.conditional_scenes.as_mut().unwrap()[0].layers = None;
        assert!(require_layer_conditions_capability(&snapshot, legacy).is_ok());
    }

    #[test]
    fn optional_endpoints_do_not_hide_transport_failures() {
        assert_eq!(optional_endpoint(Ok("name")).unwrap(), Some("name"));
        assert_eq!(
            optional_endpoint::<()>(Err(RynkHostError::Rejected(RynkError::UnknownCmd))).unwrap(),
            None
        );
        assert!(optional_endpoint::<()>(Err(RynkHostError::Transport(
            "read",
            "disconnected".into()
        )))
        .is_err());
    }

    #[test]
    fn keypad_capability_probe_handles_new_and_old_firmware() {
        let mut pointing = WirePointingConfig {
            device_count: 1,
            ..Default::default()
        };
        pointing.devices[0] = PointingDeviceConfig {
            device_id: 0,
            mode: PointingMode::Keypad(KeypadConfig::default()),
        };
        assert!(pointing_uses_keypad(&pointing));
        assert!(require_keypad_capability(Ok(PointingCapabilities {
            mode_flags: POINTING_MODE_KEYPAD,
        }))
        .is_ok());

        let error = require_keypad_capability(Err(RynkHostError::Rejected(RynkError::UnknownCmd)))
            .unwrap_err();
        assert!(error.to_string().contains("flash updated firmware first"));
    }

    #[test]
    fn cursor_remap_capability_probe_handles_new_and_old_firmware() {
        let mut pointing = WirePointingConfig {
            device_count: 1,
            ..Default::default()
        };
        pointing.devices[0] = PointingDeviceConfig {
            device_id: 0,
            mode: PointingMode::CursorRemap(CursorRemapConfig {
                primary_button: 2,
                ..Default::default()
            }),
        };
        assert!(pointing_uses_cursor_remap(&pointing));
        assert!(require_cursor_remap_capability(Ok(PointingCapabilities {
            mode_flags: POINTING_MODE_CURSOR_REMAP,
        }))
        .is_ok());

        let error =
            require_cursor_remap_capability(Err(RynkHostError::Rejected(RynkError::UnknownCmd)))
                .unwrap_err();
        assert!(error.to_string().contains("flash updated firmware first"));
    }
}
