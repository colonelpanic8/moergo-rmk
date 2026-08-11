//! Bidirectional TOML representation of managed Rynk runtime state.

use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use rynk::rmk_types::action::{Action, KeyAction};
use rynk::rmk_types::auto_mouse::AutoMouseLayerConfig as WireAutoMouseLayerConfig;
use rynk::rmk_types::ble::BleState as WireBleState;
use rynk::rmk_types::combo::{Combo, ComboDefinition, MatrixPosition, PositionCombo};
use rynk::rmk_types::morse::{Morse, MorseMode, MorseProfile, MORSE_PROFILE_NAME_MAX_LEN};
use rynk::rmk_types::pointing::{
    CaretConfig as WireCaretConfig, CursorConfig as WireCursorConfig, DragConfig as WireDragConfig,
    PointingMode, ScrollConfig as WireScrollConfig, SniperConfig as WireSniperConfig,
};
use rynk::rmk_types::protocol::rynk::{
    BehaviorConfig as WireBehaviorConfig, BehaviorOptions as WireBehaviorOptions,
    LightingActiveTransport, LightingBackgroundMode, LightingBackgroundState,
    LightingBatteryCondition, LightingBondedSlotCondition, LightingChargeCondition,
    LightingConditionSet, LightingConditionalSceneCell, LightingConnectionCondition,
    LightingEffect, LightingEffectsCondition, LightingExtendedConditionalSceneCell,
    LightingExtensionState, LightingLayerCondition, LightingLayerPolicy, LightingLedId,
    LightingNodeId, LightingOutputMode, LightingRgb8, LightingSceneCell,
    PointingConfig as WirePointingConfig, PointingDeviceConfig as WirePointingDeviceConfig,
    PointingLayerOverride as WirePointingLayerOverride, BLE_NAME_MAX_LEN,
};
use rynk::{KeyId, KeyTopology};
use serde::{Deserialize, Serialize};

pub const ROWS: u8 = 6;
pub const COLS: u8 = 14;
pub const LAYER_SIZE: usize = ROWS as usize * COLS as usize;
pub const HOLES: [usize; 4] = [5, 8, 75, 78];

const fn default_rows() -> u8 {
    ROWS
}

const fn default_cols() -> u8 {
    COLS
}

fn is_default_rows(value: &u8) -> bool {
    *value == ROWS
}

fn is_default_cols(value: &u8) -> bool {
    *value == COLS
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoldTriggerPosition {
    pub profile: u8,
    pub row: u8,
    pub col: u8,
}

/// One occupied, stable runtime morse-profile slot.
///
/// Names belong to the managed configuration rather than the timing value,
/// while `index` is what key actions and hold-trigger positions persist.  Both
/// therefore have to cross the file/device snapshot boundary together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MorseProfileEntry {
    pub index: u8,
    pub name: String,
    pub profile: MorseProfile,
}

#[derive(Debug)]
pub struct DiffFound;

impl std::fmt::Display for DiffFound {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("keyboard configuration differs")
    }
}

impl std::error::Error for DiffFound {}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RuntimeConfig {
    /// Logical matrix dimensions. The Glove80 defaults are omitted from
    /// serialized files; sibling boards such as Go60 declare their shape.
    #[serde(default = "default_rows", skip_serializing_if = "is_default_rows")]
    pub rows: u8,
    #[serde(default = "default_cols", skip_serializing_if = "is_default_cols")]
    pub cols: u8,
    /// BLE advertising name. `{slot}` expands to the one-based active profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bluetooth_name: Option<String>,
    #[serde(default)]
    pub default_layer: u8,
    #[serde(default, rename = "layer")]
    pub layers: Vec<LayerConfig>,
    /// The behavior tables the keymap addresses by index. An absent section
    /// means the file says nothing about that table and apply leaves it alone,
    /// so a configuration written before these existed still round-trips.
    #[serde(default, rename = "morse", skip_serializing_if = "Vec::is_empty")]
    pub morses: Vec<MorseConfig>,
    #[serde(default, rename = "combo", skip_serializing_if = "Vec::is_empty")]
    pub combos: Vec<ComboConfig>,
    #[serde(default, rename = "macro", skip_serializing_if = "Vec::is_empty")]
    pub macros: Vec<MacroConfig>,
    #[serde(default, rename = "fork", skip_serializing_if = "Vec::is_empty")]
    pub forks: Vec<ForkConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<BehaviorConfig>,
    /// Persisted pointing-device policy. An absent section leaves it alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointing: Option<PointingConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lighting: Option<LightingConfig>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PointingConfig {
    #[serde(default, rename = "device", skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<PointingDeviceConfig>,
    #[serde(default, rename = "override", skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<PointingLayerOverride>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PointingDeviceConfig {
    pub device_id: u8,
    #[serde(flatten)]
    pub mode: PointingModeConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PointingLayerOverride {
    pub layer: u8,
    pub device_id: u8,
    #[serde(flatten)]
    pub mode: PointingModeConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum PointingModeConfig {
    Cursor {
        #[serde(default = "one")]
        multiplier_x: u8,
        #[serde(default = "one")]
        multiplier_y: u8,
        #[serde(default)]
        invert_x: bool,
        #[serde(default)]
        invert_y: bool,
    },
    Scroll {
        #[serde(default = "one")]
        multiplier_x: u8,
        #[serde(default = "eight")]
        divisor_x: u8,
        #[serde(default = "one")]
        multiplier_y: u8,
        #[serde(default = "eight")]
        divisor_y: u8,
        #[serde(default)]
        invert_x: bool,
        #[serde(default)]
        invert_y: bool,
    },
    Sniper {
        #[serde(default = "one")]
        multiplier: u8,
        #[serde(default = "four")]
        divisor: u8,
        #[serde(default)]
        invert_x: bool,
        #[serde(default)]
        invert_y: bool,
    },
    Caret {
        #[serde(default)]
        disable_x: bool,
        #[serde(default)]
        disable_y: bool,
        #[serde(default)]
        invert_x: bool,
        #[serde(default)]
        invert_y: bool,
        #[serde(default = "hundred")]
        threshold: i16,
        #[serde(default = "up_keycode")]
        keycode_up: u8,
        #[serde(default = "down_keycode")]
        keycode_down: u8,
        #[serde(default = "left_keycode")]
        keycode_left: u8,
        #[serde(default = "right_keycode")]
        keycode_right: u8,
    },
    Drag {
        #[serde(default = "one")]
        multiplier_x: u8,
        #[serde(default = "one")]
        multiplier_y: u8,
        #[serde(default)]
        invert_x: bool,
        #[serde(default)]
        invert_y: bool,
        #[serde(default = "one")]
        toggled_by: u8,
        #[serde(default = "one")]
        latches: u8,
    },
}

const fn one() -> u8 {
    1
}
const fn four() -> u8 {
    4
}
const fn eight() -> u8 {
    8
}
const fn hundred() -> i16 {
    100
}
const fn up_keycode() -> u8 {
    rynk::rmk_types::keycode::HidKeyCode::Up as u8
}
const fn down_keycode() -> u8 {
    rynk::rmk_types::keycode::HidKeyCode::Down as u8
}
const fn left_keycode() -> u8 {
    rynk::rmk_types::keycode::HidKeyCode::Left as u8
}
const fn right_keycode() -> u8 {
    rynk::rmk_types::keycode::HidKeyCode::Right as u8
}

/// Global and parameterized behavior state managed over Rynk.
///
/// The field names deliberately mirror the firmware configuration while using
/// plain millisecond integers, as the rest of the runtime format does.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BehaviorConfig {
    #[serde(default = "default_combo_timeout_ms")]
    pub combo_timeout_ms: u16,
    #[serde(default = "default_oneshot_timeout_ms")]
    pub oneshot_timeout_ms: u16,
    #[serde(default = "default_tap_interval_ms")]
    pub tap_interval_ms: u16,
    #[serde(default = "default_tap_interval_ms")]
    pub tap_capslock_interval_ms: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tri_layer: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combo_prior_idle_ms: Option<u16>,
    #[serde(default)]
    pub oneshot_activate_on_keypress: bool,
    #[serde(default)]
    pub oneshot_quick_release: bool,
    #[serde(default)]
    pub morse: MorseBehaviorConfig,
    #[serde(
        default,
        rename = "auto_mouse_layer",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub auto_mouse_layers: Vec<AutoMouseLayerConfig>,
}

const fn default_combo_timeout_ms() -> u16 {
    50
}

const fn default_oneshot_timeout_ms() -> u16 {
    1_000
}

const fn default_tap_interval_ms() -> u16 {
    20
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            combo_timeout_ms: default_combo_timeout_ms(),
            oneshot_timeout_ms: default_oneshot_timeout_ms(),
            tap_interval_ms: default_tap_interval_ms(),
            tap_capslock_interval_ms: default_tap_interval_ms(),
            tri_layer: None,
            combo_prior_idle_ms: None,
            oneshot_activate_on_keypress: false,
            oneshot_quick_release: false,
            morse: MorseBehaviorConfig::default(),
            auto_mouse_layers: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MorseBehaviorConfig {
    #[serde(default)]
    pub enable_flow_tap: bool,
    #[serde(default = "default_morse_prior_idle_ms")]
    pub prior_idle_ms: u16,
    #[serde(default)]
    pub default_profile: MorseProfileConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hold_trigger_key_positions: Vec<[u8; 2]>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, MorseProfileConfig>,
}

const fn default_morse_prior_idle_ms() -> u16 {
    120
}

impl Default for MorseBehaviorConfig {
    fn default() -> Self {
        Self {
            enable_flow_tap: false,
            prior_idle_ms: default_morse_prior_idle_ms(),
            default_profile: MorseProfileConfig {
                unilateral_tap: Some(false),
                mode: Some("normal".to_owned()),
                hold_timeout_ms: Some(250),
                gap_timeout_ms: Some(250),
                ..MorseProfileConfig::default()
            },
            hold_trigger_key_positions: Vec::new(),
            profiles: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MorseProfileConfig {
    /// Stable runtime slot. Omitted profiles are assigned the lowest free slot
    /// in name order for backwards compatibility with the original format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_flow_tap: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_timeout_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_timeout_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick_tap_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_idle_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unilateral_tap: Option<bool>,
    /// Arm the hold at its timeout, but emit it only when the next key is on
    /// the opposite hand (or is tagged bilateral). Same-hand ordinary keys
    /// settle this tap-hold as a tap regardless of how long it was held.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opposite_hand_hold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retro_tap: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_trigger_on_release: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hold_trigger_key_positions: Vec<[u8; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AutoMouseLayerConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<u8>,
    pub target_layer: u8,
    #[serde(default = "default_auto_mouse_timeout_ms")]
    pub timeout_ms: u32,
    #[serde(default = "default_auto_mouse_threshold")]
    pub threshold: u16,
    #[serde(default)]
    pub deactivate_on_key: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_mouse_keys: Vec<String>,
    #[serde(default)]
    pub reset_timeout_on_key: bool,
}

const fn default_auto_mouse_timeout_ms() -> u32 {
    500
}

const fn default_auto_mouse_threshold() -> u16 {
    1
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LayerConfig {
    pub id: String,
    pub name: String,
    pub keys: String,
}

/// A morse (tap-dance) key, addressed from the keymap as `TD(n)` by its
/// position in the file.
///
/// Actions are written with the same keycode names `keys` uses. Timing fields
/// left out fall through to the keyboard's global defaults, which is how a
/// thumb key and a home row mod can run different windows.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MorseConfig {
    /// Host-side label; the firmware does not store it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// The wire `Morse` holds each pattern independently, so both of these are
    /// optional: a tap-dance defines `tap` and `double_tap` with no hold at all,
    /// and rendering `hold = ""` for it would produce a file that no longer
    /// parses. At least one action must be present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub double_tap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_after_tap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_timeout_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_timeout_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick_tap_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_idle_ms: Option<u16>,
    /// Send the tap when the interrupting key is on the same hand. This is
    /// what makes a home row mod bilateral, and it needs the firmware's layout
    /// to declare each key's hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unilateral_tap: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opposite_hand_hold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retro_tap: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_trigger_on_release: Option<bool>,
    /// `normal`, `permissive-hold`, `hold-on-other-press` or
    /// `tap-unless-interrupted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// A fork: one key whose output swaps while a modifier is held — ZMK's
/// mod-morph.
///
/// A fork matches on the *action* it replaces rather than a key position, so
/// `trigger` follows that action wherever it sits. `keep_mods` names the
/// modifiers passed through to the output instead of being consumed.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ForkConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// The action the fork replaces, and its output when the condition is unmet.
    pub trigger: String,
    /// Output while any modifier in `mods` is held.
    pub output: String,
    /// Modifiers that select `output`, spelled the way `keys` spells them.
    pub mods: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keep_mods: Vec<String>,
}

/// A combo: pressing every trigger in `keys` or `positions` together emits
/// `output`. Exactly one trigger representation is used by a populated slot.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComboConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
    /// Physical matrix coordinates, stable across keymap and layer changes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positions: Vec<[u8; 2]>,
    pub output: String,
    /// Restrict the combo to one layer. Absent means every layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<u8>,
}

/// A macro, addressed from the keymap as `TriggerMacro(n)` by its position in
/// the file. The operation vocabulary is the firmware's own.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MacroConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub operations: Vec<MacroOperationConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "lowercase")]
pub enum MacroOperationConfig {
    Tap { keycode: String },
    Down { keycode: String },
    Up { keycode: String },
    Delay { ms: u16 },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LightingConfig {
    pub brightness: u8,
    pub output_mode: OutputModeConfig,
    #[serde(default)]
    pub wake_layers: Vec<u8>,
    pub scene_policy: ScenePolicyConfig,
    pub background: BackgroundConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<EffectsConfig>,
    #[serde(default, rename = "scene")]
    pub scenes: Vec<SceneConfig>,
    #[serde(default, rename = "conditional_scene")]
    pub conditional_scenes: Vec<ConditionalSceneConfig>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OutputModeConfig {
    AlwaysOn,
    AlwaysOff,
    PoweredOnly,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ScenePolicyConfig {
    EffectiveOnly,
    ActiveStack,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BackgroundModeConfig {
    Solid,
    Breathe,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct BackgroundConfig {
    pub enabled: bool,
    pub hue: u8,
    pub saturation: u8,
    pub value: u8,
    pub speed: u8,
    pub mode: BackgroundModeConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct EffectsConfig {
    pub effect: String,
    /// Optional second effect from the same advertised list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay: Option<String>,
    pub palette: String,
    pub value: u8,
    pub speed: u8,
    /// Per-effect tunable parameters, keyed by the effect name and then by the
    /// parameter name the firmware advertises:
    ///
    /// ```toml
    /// [lighting.effects.params.Rain]
    /// Density = 6
    /// ```
    ///
    /// A file owns only the parameters it lists. Parameters it omits keep
    /// whatever value the keyboard already holds; they are never reset to
    /// their firmware defaults. `pull` records only parameters that differ
    /// from their default so pulled files stay small.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, BTreeMap<String, u8>>,
}

impl EffectsConfig {
    /// The extension selection without the parameter tables, so parameter
    /// differences are reported per parameter instead of as one opaque blob.
    pub fn selection(&self) -> EffectSelection<'_> {
        EffectSelection {
            effect: &self.effect,
            overlay: self.overlay.as_deref(),
            palette: &self.palette,
            value: self.value,
            speed: self.speed,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct EffectSelection<'a> {
    effect: &'a str,
    overlay: Option<&'a str>,
    palette: &'a str,
    value: u8,
    speed: u8,
}

/// One extension effect's advertised parameters, as read from a keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectParams {
    /// Index of the effect within the advertised effect-name list.
    pub index: u8,
    pub effect: String,
    pub params: Vec<ParamSpec>,
}

/// One parameter's static descriptor plus its live value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamSpec {
    pub name: String,
    pub min: u8,
    pub max: u8,
    pub default: u8,
    pub value: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum EffectKind {
    Solid,
    Blink,
    Breathe,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(untagged)]
pub enum LightingTargetConfig {
    Led { led: u16 },
    Key { key: u16 },
}

impl LightingTargetConfig {
    pub const fn led(led: u16) -> Self {
        Self::Led { led }
    }

    pub const fn key(key: u16) -> Self {
        Self::Key { key }
    }

    pub const fn led_id(&self) -> Option<u16> {
        match *self {
            Self::Led { led } => Some(led),
            Self::Key { .. } => None,
        }
    }
}

impl std::fmt::Display for LightingTargetConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Led { led } => write!(formatter, "LED {led}"),
            Self::Key { key } => write!(formatter, "key {key}"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct SceneConfig {
    pub layer: u8,
    #[serde(flatten)]
    pub target: LightingTargetConfig,
    pub color: String,
    #[serde(default = "solid")]
    pub effect: EffectKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duty: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_ms: Option<u16>,
}

/// One conditional lighting rule the host owns, as opposed to the ones a board
/// compiles in. A cell applies when every condition it names is satisfied;
/// naming none makes it unconditional.
///
/// Order is meaningful — matching rules compose in table order and later cells
/// win the slots they share — so this list is never sorted, unlike
/// [`SceneConfig`].
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ConditionalSceneConfig {
    #[serde(flatten)]
    pub target: LightingTargetConfig,
    pub color: String,
    #[serde(default = "solid")]
    pub effect: EffectKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duty: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<LayerConditionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery: Option<BatteryConditionConfig>,
    /// Gate on the live output-mode policy, which is how the mode indicator is
    /// expressed as an ordinary rule instead of something compiled in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_mode: Option<OutputModeConfig>,
    /// Gate on the live connection: the transport carrying output, the
    /// selected BLE profile, and that profile's state. This is how the
    /// connection-slot indicator is expressed as ordinary rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<ConnectionConditionConfig>,
    /// Gate on whether the animated extension band is rendering, which is
    /// what `RGB_TOG` flips. This is how a key bound to that toggle can show
    /// its own state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<EffectsConditionConfig>,
}

/// Gate a rule on the extension band being on or off.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct EffectsConditionConfig {
    pub enabled: bool,
}

/// Gate a rule on the keyboard's live connection state. Every named field
/// must hold; `profile` and `ble_state` describe the selected BLE slot
/// whether or not BLE is the active transport.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ConnectionConditionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<TransportConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ble_state: Option<BleStateConfig>,
    /// Gate on one slot holding a stored bond, whichever profile is active.
    /// `profile` can only ever describe the selected slot, so this is what
    /// lets one rule per slot key say "paired" or "empty" for all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bonded: Option<BondedSlotConditionConfig>,
    /// Gate on USB being plugged and routable, whether or not it is the
    /// transport actually carrying output. `transport = "usb"` is the
    /// narrower "USB is carrying typing right now"; this is the difference
    /// between a USB key shown ready and one shown active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usb_connected: Option<bool>,
}

/// Highest addressable BLE profile slot. The board compiles in its profile
/// count, so this is the host-side mirror of that bound and has to move with
/// `ble_profiles_num`.
const MAX_BLE_SLOT: u8 = 3;

/// Gate a rule on one BLE slot's stored bond.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct BondedSlotConditionConfig {
    pub slot: u8,
    pub bonded: bool,
}

/// The transport actually carrying HID output; `none` matches a keyboard
/// that is neither USB-ready nor BLE-connected.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TransportConfig {
    Usb,
    Ble,
    None,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BleStateConfig {
    Advertising,
    Connected,
    Inactive,
}

/// Gate a rule on a layer being active, or deliberately inactive.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LayerConditionConfig {
    pub layer: u8,
    #[serde(default = "yes")]
    pub active: bool,
}

/// Gate a rule on one half's battery. Levels are percentages; omitting a bound
/// leaves that side open.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct BatteryConditionConfig {
    pub node: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_level: Option<u8>,
    #[serde(default, skip_serializing_if = "is_any_charge")]
    pub charge: ChargeConditionConfig,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ChargeConditionConfig {
    #[default]
    Any,
    Charging,
    Discharging,
    Unknown,
}

const fn yes() -> bool {
    true
}

fn is_any_charge(charge: &ChargeConditionConfig) -> bool {
    matches!(charge, ChargeConditionConfig::Any)
}

const fn solid() -> EffectKind {
    EffectKind::Solid
}

/// The comparable form of a whole managed configuration: keycodes resolved to
/// their VIA numbers and lighting canonicalized, so a file and a keyboard can
/// be diffed and applied field by field. Produced either by validating a
/// [`RuntimeConfig`] or by reading a live keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub rows: u8,
    pub cols: u8,
    /// `None` means the source or firmware does not manage the BLE name.
    pub bluetooth_name: Option<String>,
    pub default_layer: u8,
    pub layers: Vec<Vec<KeyAction>>,
    pub lighting: Option<LightingSnapshot>,
    /// The behavior tables a keymap cell addresses by index: morses for
    /// `TD(n)`, macros for `TriggerMacro(n)`, and the combos that fire
    /// alongside them.
    ///
    /// `None` means the source does not describe the table and it should be
    /// left alone, the way the lighting fields distinguish silence from
    /// emptiness — so a file written before the `[[morse]]`, `[[combo]]` and
    /// `[[macro]]` sections existed can never read as "clear them".
    pub behaviors: BehaviorSnapshot,
    /// `None` means the source or firmware does not manage pointing devices.
    pub pointing: Option<WirePointingConfig>,
}

impl PointingModeConfig {
    fn to_wire(&self) -> PointingMode {
        match *self {
            Self::Cursor {
                multiplier_x,
                multiplier_y,
                invert_x,
                invert_y,
            } => PointingMode::Cursor(WireCursorConfig {
                multiplier_x,
                multiplier_y,
                invert_x,
                invert_y,
            }),
            Self::Scroll {
                multiplier_x,
                divisor_x,
                multiplier_y,
                divisor_y,
                invert_x,
                invert_y,
            } => PointingMode::Scroll(WireScrollConfig {
                multiplier_x,
                divisor_x,
                multiplier_y,
                divisor_y,
                invert_x,
                invert_y,
            }),
            Self::Sniper {
                multiplier,
                divisor,
                invert_x,
                invert_y,
            } => PointingMode::Sniper(WireSniperConfig {
                multiplier,
                divisor,
                invert_x,
                invert_y,
            }),
            Self::Caret {
                disable_x,
                disable_y,
                invert_x,
                invert_y,
                threshold,
                keycode_up,
                keycode_down,
                keycode_left,
                keycode_right,
            } => PointingMode::Caret(WireCaretConfig {
                disable_x,
                disable_y,
                invert_x,
                invert_y,
                threshold,
                keycode_up: keycode_up.into(),
                keycode_down: keycode_down.into(),
                keycode_left: keycode_left.into(),
                keycode_right: keycode_right.into(),
            }),
            Self::Drag {
                multiplier_x,
                multiplier_y,
                invert_x,
                invert_y,
                toggled_by,
                latches,
            } => PointingMode::Drag(WireDragConfig {
                cursor: WireCursorConfig {
                    multiplier_x,
                    multiplier_y,
                    invert_x,
                    invert_y,
                },
                toggled_by,
                latches,
            }),
        }
    }

    fn from_wire(mode: PointingMode) -> Result<Self> {
        Ok(match mode {
            PointingMode::Cursor(config) => Self::Cursor {
                multiplier_x: config.multiplier_x,
                multiplier_y: config.multiplier_y,
                invert_x: config.invert_x,
                invert_y: config.invert_y,
            },
            PointingMode::Scroll(config) => Self::Scroll {
                multiplier_x: config.multiplier_x,
                divisor_x: config.divisor_x,
                multiplier_y: config.multiplier_y,
                divisor_y: config.divisor_y,
                invert_x: config.invert_x,
                invert_y: config.invert_y,
            },
            PointingMode::Sniper(config) => Self::Sniper {
                multiplier: config.multiplier,
                divisor: config.divisor,
                invert_x: config.invert_x,
                invert_y: config.invert_y,
            },
            PointingMode::Drag(config) => Self::Drag {
                multiplier_x: config.cursor.multiplier_x,
                multiplier_y: config.cursor.multiplier_y,
                invert_x: config.cursor.invert_x,
                invert_y: config.cursor.invert_y,
                toggled_by: config.toggled_by,
                latches: config.latches,
            },
            PointingMode::Caret(config) => Self::Caret {
                disable_x: config.disable_x,
                disable_y: config.disable_y,
                invert_x: config.invert_x,
                invert_y: config.invert_y,
                threshold: config.threshold,
                keycode_up: config.keycode_up as u8,
                keycode_down: config.keycode_down as u8,
                keycode_left: config.keycode_left as u8,
                keycode_right: config.keycode_right as u8,
            },
        })
    }
}

impl PointingConfig {
    fn to_wire(&self, layer_count: usize) -> Result<WirePointingConfig> {
        use rynk::rmk_types::protocol::rynk::{
            POINTING_DEVICE_CAPACITY, POINTING_LAYER_OVERRIDE_CAPACITY,
        };
        if self.devices.len() > POINTING_DEVICE_CAPACITY {
            bail!("pointing config has more than {POINTING_DEVICE_CAPACITY} devices");
        }
        if self.overrides.len() > POINTING_LAYER_OVERRIDE_CAPACITY {
            bail!("pointing config has more than {POINTING_LAYER_OVERRIDE_CAPACITY} overrides");
        }
        let mut result = WirePointingConfig {
            device_count: self.devices.len() as u8,
            override_count: self.overrides.len() as u8,
            ..Default::default()
        };
        for (slot, device) in self.devices.iter().enumerate() {
            if self.devices[..slot]
                .iter()
                .any(|old| old.device_id == device.device_id)
            {
                bail!(
                    "pointing device {} is configured more than once",
                    device.device_id
                );
            }
            result.devices[slot] = WirePointingDeviceConfig {
                device_id: device.device_id,
                mode: device.mode.to_wire(),
            };
        }
        for (slot, entry) in self.overrides.iter().enumerate() {
            if usize::from(entry.layer) >= layer_count {
                bail!(
                    "pointing override {slot} references missing layer {}",
                    entry.layer
                );
            }
            result.overrides[slot] = WirePointingLayerOverride {
                layer: entry.layer,
                device_id: entry.device_id,
                mode: entry.mode.to_wire(),
            };
        }
        Ok(result)
    }

    fn from_wire(config: &WirePointingConfig) -> Result<Self> {
        Ok(Self {
            devices: config
                .devices()
                .iter()
                .map(|device| {
                    Ok(PointingDeviceConfig {
                        device_id: device.device_id,
                        mode: PointingModeConfig::from_wire(device.mode)?,
                    })
                })
                .collect::<Result<_>>()?,
            overrides: config
                .overrides()
                .iter()
                .map(|entry| {
                    Ok(PointingLayerOverride {
                        layer: entry.layer,
                        device_id: entry.device_id,
                        mode: PointingModeConfig::from_wire(entry.mode)?,
                    })
                })
                .collect::<Result<_>>()?,
        })
    }
}

/// The behavior half of a [`Snapshot`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BehaviorSnapshot {
    pub config: Option<WireBehaviorConfig>,
    pub options: Option<WireBehaviorOptions>,
    pub morse_profiles: Option<Vec<MorseProfileEntry>>,
    pub hold_trigger_positions: Option<Vec<HoldTriggerPosition>>,
    pub auto_mouse_layers: Option<Vec<WireAutoMouseLayerConfig>>,
    pub morses: Option<Vec<rynk::rmk_types::morse::Morse>>,
    pub combos: Option<Vec<ComboDefinition>>,
    /// Macro space as the firmware stores it: the sequences concatenated, each
    /// ended by its own terminator, which is what `TriggerMacro` indexes into.
    pub macros: Option<Vec<u8>>,
    /// Forks: one key's output swapped while a modifier is held. Unlike the
    /// tables above, a keymap cell does not address these by index — a fork
    /// matches on the action it replaces.
    pub forks: Option<Vec<rynk::rmk_types::fork::Fork>>,
}

/// The lighting half of a [`Snapshot`]. Its `Option` fields distinguish state a
/// source is silent about from state it says is empty, which is what keeps
/// older firmware from reading as "delete everything".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LightingSnapshot {
    pub brightness: u8,
    pub output_mode: OutputModeConfig,
    pub wake_layers: Vec<u8>,
    pub scene_policy: ScenePolicyConfig,
    pub background: BackgroundConfig,
    pub effects: Option<EffectsConfig>,
    /// Parameters the keyboard advertises. `None` in a file snapshot, and also
    /// on firmware that does not implement the parameter commands at all.
    pub params: Option<Vec<EffectParams>>,
    pub scenes: Vec<SceneConfig>,
    /// Host-owned conditional rules. `None` on firmware that does not
    /// implement the runtime conditional commands, which keeps "not supported"
    /// distinct from "supported and empty".
    pub conditional_scenes: Option<Vec<ConditionalSceneConfig>>,
}

fn resolve_morse_profiles(behavior: &BehaviorConfig) -> Result<Vec<MorseProfileEntry>> {
    let mut occupied = [false; u8::MAX as usize];
    for (name, config) in &behavior.morse.profiles {
        if name.is_empty() {
            bail!("morse profile names must not be empty");
        }
        if name.len() > MORSE_PROFILE_NAME_MAX_LEN {
            bail!(
                "morse profile name '{name}' is {} bytes, but runtime names are limited to {MORSE_PROFILE_NAME_MAX_LEN} bytes",
                name.len()
            );
        }
        if let Some(index) = config.index {
            if index == u8::MAX {
                bail!("morse profile '{name}' uses reserved index {index}");
            }
            if std::mem::replace(&mut occupied[usize::from(index)], true) {
                bail!("morse profile index {index} is assigned more than once");
            }
        }
    }

    let mut entries = Vec::with_capacity(behavior.morse.profiles.len());
    for (name, config) in &behavior.morse.profiles {
        let index = match config.index {
            Some(index) => index,
            None => {
                let index = occupied
                    .iter()
                    .position(|used| !used)
                    .context("more than 255 morse profiles")?;
                occupied[index] = true;
                index as u8
            }
        };
        entries.push(MorseProfileEntry {
            index,
            name: name.clone(),
            profile: config.to_wire()?,
        });
    }
    entries.sort_by_key(|entry| entry.index);
    Ok(entries)
}

fn profile_names_by_slot(entries: &[MorseProfileEntry]) -> Vec<String> {
    let Some(last) = entries.iter().map(|entry| entry.index).max() else {
        return Vec::new();
    };
    let mut names = vec![String::new(); usize::from(last) + 1];
    for entry in entries {
        names[usize::from(entry.index)] = entry.name.clone();
    }
    names
}

impl RuntimeConfig {
    /// Deserialize and validate runtime TOML. Validation is exactly what
    /// [`Self::snapshot`] checks, so a config that parses here is one a
    /// keyboard can be asked to hold.
    pub fn from_toml(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text)?;
        config.snapshot().map(|_| config)
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        if self.rows == 0 || self.cols == 0 {
            bail!("rows and cols must both be non-zero");
        }
        if self.layers.is_empty() {
            bail!("configuration must contain at least one [[layer]]");
        }
        if let Some(name) = &self.bluetooth_name {
            if name.is_empty() {
                bail!("bluetooth_name must not be empty");
            }
            if name.len() > BLE_NAME_MAX_LEN {
                bail!(
                    "bluetooth_name is {} bytes, but BLE names are limited to {BLE_NAME_MAX_LEN}",
                    name.len()
                );
            }
        }
        let morse_profiles = self
            .behavior
            .as_ref()
            .map(resolve_morse_profiles)
            .transpose()?
            .unwrap_or_default();
        let profile_names = profile_names_by_slot(&morse_profiles);
        let mut ids = BTreeMap::new();
        let mut layers = Vec::with_capacity(self.layers.len());
        for (index, layer) in self.layers.iter().enumerate() {
            if layer.id.trim().is_empty() || layer.name.trim().is_empty() {
                bail!("layer {index} must have non-empty id and name");
            }
            if ids.insert(&layer.id, index).is_some() {
                bail!("duplicate layer id '{}'", layer.id);
            }
            layers.push(
                parse_key_actions_for_matrix(&layer.keys, &profile_names, self.rows, self.cols)
                    .with_context(|| format!("layer {} ({})", index, layer.id))?,
            );
        }
        if usize::from(self.default_layer) >= layers.len() {
            bail!(
                "default_layer {} is outside the {} configured layers",
                self.default_layer,
                layers.len()
            );
        }
        if let Some(behavior) = &self.behavior {
            if let Some(tri_layer) = behavior.tri_layer {
                for layer in tri_layer {
                    if usize::from(layer) >= layers.len() {
                        bail!(
                            "tri_layer references layer {layer}, but only {} layers are configured",
                            layers.len()
                        );
                    }
                }
            }
            for (index, auto_mouse) in behavior.auto_mouse_layers.iter().enumerate() {
                if usize::from(auto_mouse.target_layer) >= layers.len() {
                    bail!(
                        "behavior.auto_mouse_layer {index} targets layer {}, but only {} layers are configured",
                        auto_mouse.target_layer,
                        layers.len()
                    );
                }
            }
            for (location, positions) in
                std::iter::once(("behavior.morse", &behavior.morse.hold_trigger_key_positions))
                    .chain(behavior.morse.profiles.iter().map(|(name, profile)| {
                        (name.as_str(), &profile.hold_trigger_key_positions)
                    }))
            {
                for [row, col] in positions {
                    if *row >= self.rows || *col >= self.cols {
                        bail!(
                            "{location} hold trigger position [{row}, {col}] is outside the {}x{} matrix",
                            self.rows,
                            self.cols,
                        );
                    }
                }
            }
        }
        let lighting = self
            .lighting
            .as_ref()
            .map(LightingConfig::snapshot)
            .transpose()?;
        let behavior = self.behavior.as_ref();
        let layer_count = layers.len();
        if let Some(lighting) = &lighting {
            for layer in &lighting.wake_layers {
                if usize::from(*layer) >= layer_count {
                    bail!(
                        "lighting wake layer {layer} is outside the {layer_count} configured layers"
                    );
                }
            }
        }
        Ok(Snapshot {
            rows: self.rows,
            cols: self.cols,
            bluetooth_name: self.bluetooth_name.clone(),
            default_layer: self.default_layer,
            layers,
            lighting,
            pointing: self
                .pointing
                .as_ref()
                .map(|pointing| pointing.to_wire(layer_count))
                .transpose()?,
            behaviors: BehaviorSnapshot {
                config: behavior.map(BehaviorConfig::wire_config),
                options: behavior.map(BehaviorConfig::wire_options).transpose()?,
                morse_profiles: behavior.map(|_| morse_profiles.clone()),
                hold_trigger_positions: behavior.map(|behavior| {
                    let mut positions = behavior
                        .morse
                        .hold_trigger_key_positions
                        .iter()
                        .map(|[row, col]| HoldTriggerPosition {
                            profile: u8::MAX,
                            row: *row,
                            col: *col,
                        })
                        .collect::<Vec<_>>();
                    for entry in &morse_profiles {
                        let config = &behavior.morse.profiles[&entry.name];
                        positions.extend(config.hold_trigger_key_positions.iter().map(
                            |[row, col]| HoldTriggerPosition {
                                profile: entry.index,
                                row: *row,
                                col: *col,
                            },
                        ));
                    }
                    positions
                }),
                auto_mouse_layers: behavior
                    .map(|behavior| {
                        behavior
                            .auto_mouse_layers
                            .iter()
                            .map(AutoMouseLayerConfig::to_wire)
                            .collect::<Result<Vec<_>>>()
                    })
                    .transpose()?,
                morses: (!self.morses.is_empty())
                    .then(|| {
                        self.morses
                            .iter()
                            .enumerate()
                            .map(|(index, morse)| {
                                morse
                                    .to_wire()
                                    .with_context(|| format!("[[morse]] {index} ({})", morse.name))
                            })
                            .collect::<Result<Vec<_>>>()
                    })
                    .transpose()?,
                combos: (!self.combos.is_empty())
                    .then(|| {
                        self.combos
                            .iter()
                            .enumerate()
                            .map(|(index, combo)| {
                                combo
                                    .to_wire(&profile_names, self.rows, self.cols)
                                    .with_context(|| format!("[[combo]] {index} ({})", combo.name))
                            })
                            .collect::<Result<Vec<_>>>()
                    })
                    .transpose()?,
                forks: (!self.forks.is_empty())
                    .then(|| {
                        self.forks
                            .iter()
                            .enumerate()
                            .map(|(index, fork)| {
                                fork.to_wire()
                                    .with_context(|| format!("[[fork]] {index} ({})", fork.name))
                            })
                            .collect::<Result<Vec<_>>>()
                    })
                    .transpose()?,
                macros: (!self.macros.is_empty())
                    .then(|| {
                        let mut space = Vec::new();
                        for (index, entry) in self.macros.iter().enumerate() {
                            space.extend(
                                entry.to_wire().with_context(|| {
                                    format!("[[macro]] {index} ({})", entry.name)
                                })?,
                            );
                        }
                        Ok::<_, anyhow::Error>(space)
                    })
                    .transpose()?,
            },
        })
    }

    pub fn from_snapshot(snapshot: &Snapshot, labels: Option<&RuntimeConfig>) -> Self {
        let behavior = BehaviorConfig::from_snapshot(&snapshot.behaviors);
        let profile_names = profile_names_by_slot(
            snapshot
                .behaviors
                .morse_profiles
                .as_deref()
                .unwrap_or_default(),
        );
        let mut snapshot_layers = snapshot.layers.clone();
        trim_trailing_transparent_layers(&mut snapshot_layers);
        let layers = snapshot_layers
            .iter()
            .enumerate()
            .map(|(index, keys)| {
                let old = labels.and_then(|config| config.layers.get(index));
                LayerConfig {
                    id: old.map_or_else(|| format!("layer{index}"), |layer| layer.id.clone()),
                    name: old.map_or_else(|| format!("Layer {index}"), |layer| layer.name.clone()),
                    keys: render_key_actions_for_matrix(
                        keys,
                        &profile_names,
                        snapshot.rows,
                        snapshot.cols,
                    ),
                }
            })
            .collect();
        Self {
            rows: snapshot.rows,
            cols: snapshot.cols,
            bluetooth_name: snapshot.bluetooth_name.clone(),
            default_layer: snapshot.default_layer,
            layers,
            morses: used_slots(
                snapshot.behaviors.morses.as_deref().unwrap_or_default(),
                |morse| morse.actions.is_empty(),
            )
            .iter()
            .enumerate()
            .map(|(index, morse)| {
                let mut config = MorseConfig::from_wire(morse, index);
                // The firmware stores no label, so keep the one a previous
                // file gave this slot, as layer names are kept.
                if let Some(old) = labels.and_then(|config| config.morses.get(index)) {
                    config.name = old.name.clone();
                }
                config
            })
            .collect(),
            combos: used_slots(
                snapshot.behaviors.combos.as_deref().unwrap_or_default(),
                ComboDefinition::is_empty,
            )
            .iter()
            .enumerate()
            .map(|(index, combo)| {
                let mut config = ComboConfig::from_wire(combo, index, &profile_names);
                if let Some(old) = labels.and_then(|config| config.combos.get(index)) {
                    config.name = old.name.clone();
                }
                config
            })
            .collect(),
            forks: used_slots(
                snapshot.behaviors.forks.as_deref().unwrap_or_default(),
                is_empty_fork,
            )
            .iter()
            .enumerate()
            .map(|(index, fork)| {
                let mut config = ForkConfig::from_wire(fork, index);
                if let Some(old) = labels.and_then(|config| config.forks.get(index)) {
                    config.name = old.name.clone();
                }
                config
            })
            .collect(),
            macros: snapshot
                .behaviors
                .macros
                .as_deref()
                .map(MacroConfig::all_from_wire)
                .unwrap_or_default(),
            behavior,
            pointing: snapshot
                .pointing
                .as_ref()
                .map(PointingConfig::from_wire)
                .transpose()
                .ok()
                .flatten(),
            lighting: snapshot
                .lighting
                .as_ref()
                .map(LightingConfig::from_snapshot),
        }
    }

    pub fn to_toml(&self) -> Result<String> {
        let mut text =
            toml::to_string_pretty(self).context("could not serialize runtime configuration")?;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text)
    }
}

/// The occupied prefix of a fixed-capacity runtime table. Only the unused tail
/// may be removed: key actions address entries by slot, so interior gaps must
/// keep their indices.
fn used_slots<T>(table: &[T], empty: impl Fn(&T) -> bool) -> &[T] {
    let used = table.iter().rposition(|entry| !empty(entry));
    &table[..used.map_or(0, |index| index + 1)]
}

/// Spell a wire action the way the keymap's `keys` field spells one.
fn action_name(action: Action) -> String {
    crate::keycodes::format_keycode(crate::rynk_keycode::to_via_keycode(KeyAction::Single(
        action,
    )))
}

fn action_from_name(text: &str) -> Result<Action> {
    match crate::rynk_keycode::from_via_keycode(crate::keycodes::parse_keycode(text)?) {
        KeyAction::Single(action) => Ok(action),
        KeyAction::No => Ok(Action::No),
        _ => bail!("'{text}' is not a single action"),
    }
}

impl BehaviorConfig {
    fn wire_config(&self) -> WireBehaviorConfig {
        WireBehaviorConfig {
            combo_timeout_ms: self.combo_timeout_ms,
            oneshot_timeout_ms: self.oneshot_timeout_ms,
            tap_interval_ms: self.tap_interval_ms,
            tap_capslock_interval_ms: self.tap_capslock_interval_ms,
        }
    }

    fn wire_options(&self) -> Result<WireBehaviorOptions> {
        Ok(WireBehaviorOptions {
            tri_layer: self.tri_layer,
            combo_prior_idle_ms: self.combo_prior_idle_ms,
            oneshot_activate_on_keypress: self.oneshot_activate_on_keypress,
            oneshot_quick_release: self.oneshot_quick_release,
            morse_enable_flow_tap: self.morse.enable_flow_tap,
            morse_prior_idle_ms: self.morse.prior_idle_ms,
            morse_default_profile: self.morse.default_profile.to_wire()?,
        })
    }

    fn from_snapshot(snapshot: &BehaviorSnapshot) -> Option<Self> {
        let config = snapshot.config?;
        let options = snapshot.options?;
        let default_profile = MorseProfileConfig::from_wire(options.morse_default_profile);
        let hold_trigger_positions = snapshot
            .hold_trigger_positions
            .as_deref()
            .unwrap_or_default();
        let mut profiles = BTreeMap::new();
        for entry in snapshot.morse_profiles.as_deref().unwrap_or_default() {
            let mut config = MorseProfileConfig::from_wire(entry.profile);
            config.index = Some(entry.index);
            config.hold_trigger_key_positions = hold_trigger_positions
                .iter()
                .filter(|position| position.profile == entry.index)
                .map(|position| [position.row, position.col])
                .collect();
            profiles.insert(entry.name.clone(), config);
        }
        Some(Self {
            combo_timeout_ms: config.combo_timeout_ms,
            oneshot_timeout_ms: config.oneshot_timeout_ms,
            tap_interval_ms: config.tap_interval_ms,
            tap_capslock_interval_ms: config.tap_capslock_interval_ms,
            tri_layer: options.tri_layer,
            combo_prior_idle_ms: options.combo_prior_idle_ms,
            oneshot_activate_on_keypress: options.oneshot_activate_on_keypress,
            oneshot_quick_release: options.oneshot_quick_release,
            morse: MorseBehaviorConfig {
                enable_flow_tap: options.morse_enable_flow_tap,
                prior_idle_ms: options.morse_prior_idle_ms,
                default_profile,
                hold_trigger_key_positions: hold_trigger_positions
                    .iter()
                    .filter(|position| position.profile == u8::MAX)
                    .map(|position| [position.row, position.col])
                    .collect(),
                profiles,
            },
            auto_mouse_layers: snapshot
                .auto_mouse_layers
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(AutoMouseLayerConfig::from_wire)
                .collect(),
        })
    }
}

impl MorseProfileConfig {
    fn to_wire(&self) -> Result<MorseProfile> {
        if self.unilateral_tap == Some(true) && self.opposite_hand_hold == Some(true) {
            bail!("unilateral_tap and opposite_hand_hold are mutually exclusive");
        }
        let mode = match self.mode.as_deref() {
            None => None,
            Some("normal") => Some(MorseMode::Normal),
            Some("permissive-hold") => Some(MorseMode::PermissiveHold),
            Some("hold-on-other-press") => Some(MorseMode::HoldOnOtherPress),
            Some("tap-unless-interrupted") => Some(MorseMode::TapUnlessInterrupted),
            Some(other) => bail!("unknown morse mode '{other}'"),
        };
        Ok(MorseProfile::const_default()
            .with_enable_flow_tap(self.enable_flow_tap)
            .with_mode(mode)
            .with_hold_timeout_ms(self.hold_timeout_ms)
            .with_gap_timeout_ms(self.gap_timeout_ms)
            .with_quick_tap_timeout_ms(self.quick_tap_ms)
            .with_prior_idle_time_ms(self.prior_idle_ms)
            .with_unilateral_tap(self.unilateral_tap)
            .with_opposite_hand_hold(self.opposite_hand_hold)
            .with_retro_tap(self.retro_tap)
            .with_hold_trigger_on_release(self.hold_trigger_on_release))
    }

    fn from_wire(profile: MorseProfile) -> Self {
        Self {
            index: None,
            enable_flow_tap: profile.enable_flow_tap(),
            hold_timeout_ms: profile.hold_timeout_ms(),
            gap_timeout_ms: profile.gap_timeout_ms(),
            quick_tap_ms: profile.quick_tap_timeout_ms(),
            prior_idle_ms: profile.prior_idle_time_ms(),
            unilateral_tap: profile.unilateral_tap(),
            opposite_hand_hold: profile.opposite_hand_hold(),
            retro_tap: profile.retro_tap(),
            hold_trigger_on_release: profile.hold_trigger_on_release(),
            hold_trigger_key_positions: Vec::new(),
            mode: profile.mode().map(|mode| match mode {
                MorseMode::Normal => "normal".to_owned(),
                MorseMode::PermissiveHold => "permissive-hold".to_owned(),
                MorseMode::HoldOnOtherPress => "hold-on-other-press".to_owned(),
                MorseMode::TapUnlessInterrupted => "tap-unless-interrupted".to_owned(),
            }),
        }
    }
}

impl AutoMouseLayerConfig {
    fn to_wire(&self) -> Result<WireAutoMouseLayerConfig> {
        if self.timeout_ms == 0 {
            bail!("auto mouse timeout_ms must be at least 1");
        }
        if self.threshold == 0 {
            bail!("auto mouse threshold must be at least 1");
        }
        let mut extra_mouse_keys = Vec::with_capacity(self.extra_mouse_keys.len());
        for name in &self.extra_mouse_keys {
            match action_from_name(name)? {
                Action::Key(key) => extra_mouse_keys.push(key),
                _ => bail!("auto mouse extra key '{name}' is not a keycode"),
            }
        }
        Ok(WireAutoMouseLayerConfig {
            device_id: self.device_id,
            target_layer: self.target_layer,
            timeout_ms: self.timeout_ms,
            threshold: self.threshold,
            deactivate_on_key: self.deactivate_on_key,
            extra_mouse_keys,
            reset_timeout_on_key: self.reset_timeout_on_key,
        })
    }

    fn from_wire(config: &WireAutoMouseLayerConfig) -> Self {
        Self {
            device_id: config.device_id,
            target_layer: config.target_layer,
            timeout_ms: config.timeout_ms,
            threshold: config.threshold,
            deactivate_on_key: config.deactivate_on_key,
            extra_mouse_keys: config
                .extra_mouse_keys
                .iter()
                .copied()
                .map(|key| action_name(Action::Key(key)))
                .collect(),
            reset_timeout_on_key: config.reset_timeout_on_key,
        }
    }
}

impl MorseConfig {
    pub(crate) fn to_wire(&self) -> Result<Morse> {
        use rynk::rmk_types::morse::{DOUBLE_TAP, HOLD, HOLD_AFTER_TAP, TAP};

        if self.unilateral_tap == Some(true) && self.opposite_hand_hold == Some(true) {
            bail!("unilateral_tap and opposite_hand_hold are mutually exclusive");
        }
        let mode = match self.mode.as_deref() {
            None => None,
            Some("normal") => Some(MorseMode::Normal),
            Some("permissive-hold") => Some(MorseMode::PermissiveHold),
            Some("hold-on-other-press") => Some(MorseMode::HoldOnOtherPress),
            Some("tap-unless-interrupted") => Some(MorseMode::TapUnlessInterrupted),
            Some(other) => bail!("unknown morse mode '{other}'"),
        };
        let profile = MorseProfile::const_default()
            .with_mode(mode)
            .with_hold_timeout_ms(self.hold_timeout_ms)
            .with_gap_timeout_ms(self.gap_timeout_ms)
            .with_quick_tap_timeout_ms(self.quick_tap_ms)
            .with_prior_idle_time_ms(self.prior_idle_ms)
            .with_unilateral_tap(self.unilateral_tap)
            .with_opposite_hand_hold(self.opposite_hand_hold)
            .with_retro_tap(self.retro_tap)
            .with_hold_trigger_on_release(self.hold_trigger_on_release);

        let mut morse = Morse {
            profile,
            ..Morse::default()
        };
        if let Some(text) = &self.tap {
            let _ = morse.put(TAP, action_from_name(text).context("tap action")?);
        }
        if let Some(text) = &self.hold {
            let _ = morse.put(HOLD, action_from_name(text).context("hold action")?);
        }
        if let Some(text) = &self.double_tap {
            let _ = morse.put(DOUBLE_TAP, action_from_name(text).context("double tap")?);
        }
        if let Some(text) = &self.hold_after_tap {
            let _ = morse.put(
                HOLD_AFTER_TAP,
                action_from_name(text).context("hold after tap")?,
            );
        }
        if morse.actions.is_empty()
            && self.tap.is_none()
            && self.hold.is_none()
            && self.double_tap.is_none()
            && self.hold_after_tap.is_none()
        {
            bail!("a morse needs at least one of tap, hold, double_tap or hold_after_tap");
        }
        Ok(morse)
    }

    pub(crate) fn from_wire(morse: &Morse, index: usize) -> Self {
        use rynk::rmk_types::morse::{DOUBLE_TAP, HOLD, HOLD_AFTER_TAP, TAP};

        Self {
            name: format!("morse {index}"),
            tap: morse
                .get(TAP)
                .map(action_name)
                .or_else(|| morse.actions.is_empty().then(|| action_name(Action::No))),
            hold: morse.get(HOLD).map(action_name),
            double_tap: morse.get(DOUBLE_TAP).map(action_name),
            hold_after_tap: morse.get(HOLD_AFTER_TAP).map(action_name),
            hold_timeout_ms: morse.profile.hold_timeout_ms(),
            gap_timeout_ms: morse.profile.gap_timeout_ms(),
            quick_tap_ms: morse.profile.quick_tap_timeout_ms(),
            prior_idle_ms: morse.profile.prior_idle_time_ms(),
            unilateral_tap: morse.profile.unilateral_tap(),
            opposite_hand_hold: morse.profile.opposite_hand_hold(),
            retro_tap: morse.profile.retro_tap(),
            hold_trigger_on_release: morse.profile.hold_trigger_on_release(),
            mode: morse.profile.mode().map(|mode| {
                match mode {
                    MorseMode::Normal => "normal",
                    MorseMode::PermissiveHold => "permissive-hold",
                    MorseMode::HoldOnOtherPress => "hold-on-other-press",
                    MorseMode::TapUnlessInterrupted => "tap-unless-interrupted",
                }
                .to_owned()
            }),
        }
    }
}

impl ForkConfig {
    pub(crate) fn to_wire(&self) -> Result<rynk::rmk_types::fork::Fork> {
        use rynk::rmk_types::fork::{Fork, StateBits};

        let trigger =
            crate::rynk_keycode::from_via_keycode(crate::keycodes::parse_keycode(&self.trigger)?);
        let output =
            crate::rynk_keycode::from_via_keycode(crate::keycodes::parse_keycode(&self.output)?);
        Ok(Fork {
            trigger,
            negative_output: trigger,
            positive_output: output,
            match_any: StateBits {
                modifiers: modifier_list_to_wire(&self.mods)?,
                ..StateBits::default()
            },
            match_none: StateBits::default(),
            kept_modifiers: modifier_list_to_wire(&self.keep_mods)?,
            bindable: true,
        })
    }

    pub(crate) fn from_wire(fork: &rynk::rmk_types::fork::Fork, index: usize) -> Self {
        Self {
            name: format!("fork {index}"),
            trigger: crate::keycodes::format_keycode(crate::rynk_keycode::to_via_keycode(
                fork.trigger,
            )),
            output: crate::keycodes::format_keycode(crate::rynk_keycode::to_via_keycode(
                fork.positive_output,
            )),
            mods: modifier_list_from_wire(fork.match_any.modifiers),
            keep_mods: modifier_list_from_wire(fork.kept_modifiers),
        }
    }
}

/// Modifier names as the wire combination. Spelled like `keys` spells them
/// (`LShift`), not like ZMK does (`MOD_LSFT`), because this is a Rynk file.
fn modifier_list_to_wire(
    names: &[String],
) -> Result<rynk::rmk_types::modifier::ModifierCombination> {
    let mut combination = rynk::rmk_types::modifier::ModifierCombination::default();
    for name in names {
        combination = match name.trim().to_ascii_lowercase().as_str() {
            "lctrl" | "lctl" => combination.with_left_ctrl(true),
            "lshift" | "lsft" => combination.with_left_shift(true),
            "lalt" => combination.with_left_alt(true),
            "lgui" => combination.with_left_gui(true),
            "rctrl" | "rctl" => combination.with_right_ctrl(true),
            "rshift" | "rsft" => combination.with_right_shift(true),
            "ralt" => combination.with_right_alt(true),
            "rgui" => combination.with_right_gui(true),
            other => bail!("'{other}' is not a modifier name"),
        };
    }
    Ok(combination)
}

fn modifier_list_from_wire(
    combination: rynk::rmk_types::modifier::ModifierCombination,
) -> Vec<String> {
    let mut names = Vec::new();
    for (held, name) in [
        (combination.left_ctrl(), "LCtrl"),
        (combination.left_shift(), "LShift"),
        (combination.left_alt(), "LAlt"),
        (combination.left_gui(), "LGui"),
        (combination.right_ctrl(), "RCtrl"),
        (combination.right_shift(), "RShift"),
        (combination.right_alt(), "RAlt"),
        (combination.right_gui(), "RGui"),
    ] {
        if held {
            names.push(name.to_owned());
        }
    }
    names
}

impl ComboConfig {
    pub(crate) fn to_wire(
        &self,
        profile_names: &[String],
        rows: u8,
        cols: u8,
    ) -> Result<ComboDefinition> {
        if !self.keys.is_empty() && !self.positions.is_empty() {
            bail!("a combo cannot use both keys and positions");
        }
        let output =
            crate::rynk_keycode::from_via_keycode(crate::keycodes::parse_keycode(&self.output)?);
        if self.keys.is_empty() && self.positions.is_empty() {
            if output == KeyAction::No {
                return Ok(ComboDefinition::empty());
            }
            bail!("a combo needs at least two keys or positions");
        }
        if !self.positions.is_empty() {
            let mut positions = heapless::Vec::new();
            for [row, col] in &self.positions {
                if *row >= rows || *col >= cols {
                    bail!("combo position [{row}, {col}] is outside the {rows}x{cols} matrix");
                }
                let position = MatrixPosition {
                    row: *row,
                    col: *col,
                };
                if positions.contains(&position) {
                    bail!("combo position [{row}, {col}] is duplicated");
                }
                positions
                    .push(position)
                    .map_err(|_| anyhow::anyhow!("more positions than a combo can hold"))?;
            }
            if positions.len() < 2 {
                bail!("a combo needs at least two positions");
            }
            return Ok(ComboDefinition::Positions(PositionCombo {
                positions,
                output,
                layer: self.layer,
            }));
        }
        let mut actions = heapless::Vec::new();
        for key in &self.keys {
            let action = parse_key_action(key, profile_names)?;
            actions
                .push(action)
                .map_err(|_| anyhow::anyhow!("more keys than a combo can hold"))?;
        }
        if actions.len() < 2 {
            bail!("a combo needs at least two keys");
        }
        Ok(ComboDefinition::Actions(Combo {
            actions,
            output,
            layer: self.layer,
        }))
    }

    pub(crate) fn from_wire(
        definition: &ComboDefinition,
        index: usize,
        profile_names: &[String],
    ) -> Self {
        let (keys, positions, output, layer) = match definition {
            ComboDefinition::Actions(combo) => (
                combo
                    .actions
                    .iter()
                    .map(|action| render_key_action(*action, profile_names))
                    .collect(),
                Vec::new(),
                combo.output,
                combo.layer,
            ),
            ComboDefinition::Positions(combo) => (
                Vec::new(),
                combo
                    .positions
                    .iter()
                    .map(|position| [position.row, position.col])
                    .collect(),
                combo.output,
                combo.layer,
            ),
        };
        Self {
            name: format!("combo {index}"),
            keys,
            positions,
            output: crate::keycodes::format_keycode(crate::rynk_keycode::to_via_keycode(output)),
            layer,
        }
    }
}

impl MacroConfig {
    /// One macro's bytes, terminator included.
    pub(crate) fn to_wire(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        for operation in &self.operations {
            let (tag, keycode) = match operation {
                MacroOperationConfig::Tap { keycode } => (0x01, Some(keycode)),
                MacroOperationConfig::Down { keycode } => (0x02, Some(keycode)),
                MacroOperationConfig::Up { keycode } => (0x03, Some(keycode)),
                MacroOperationConfig::Delay { ms } => {
                    // Vial packs a delay as two bytes that are never zero.
                    bytes.extend_from_slice(&[
                        0x01,
                        0x04,
                        (ms % 255) as u8 + 1,
                        (ms / 255) as u8 + 1,
                    ]);
                    continue;
                }
            };
            let Some(keycode) = keycode else { continue };
            let code = crate::keycodes::parse_keycode(keycode)?;
            // A modified keycode has no one-byte form, so the modifiers are
            // pressed around the key instead.
            let modifiers = modifier_hid_keys((code >> 8) as u8);
            for modifier in &modifiers {
                bytes.extend_from_slice(&[0x01, 0x02, *modifier]);
            }
            bytes.extend_from_slice(&[0x01, tag, (code & 0xff) as u8]);
            for modifier in modifiers.iter().rev() {
                bytes.extend_from_slice(&[0x01, 0x03, *modifier]);
            }
        }
        bytes.push(0x00);
        Ok(bytes)
    }

    /// Split macro space back into one entry per terminator.
    pub(crate) fn all_from_wire(space: &[u8]) -> Vec<Self> {
        let mut out = Vec::new();
        for (index, sequence) in space.split(|byte| *byte == 0).enumerate() {
            if sequence.is_empty() {
                continue;
            }
            let mut operations = Vec::new();
            let mut cursor = 0;
            while cursor + 2 < sequence.len() + 1 && cursor + 1 < sequence.len() {
                match (sequence[cursor], sequence.get(cursor + 1)) {
                    (0x01, Some(0x04)) if cursor + 3 < sequence.len() => {
                        let ms = (sequence[cursor + 2].max(1) as u16 - 1)
                            + (sequence[cursor + 3].max(1) as u16 - 1) * 255;
                        operations.push(MacroOperationConfig::Delay { ms });
                        cursor += 4;
                    }
                    (0x01, Some(tag @ 0x01..=0x03)) if cursor + 2 < sequence.len() => {
                        let keycode = crate::keycodes::format_keycode(sequence[cursor + 2] as u16);
                        operations.push(match tag {
                            0x01 => MacroOperationConfig::Tap { keycode },
                            0x02 => MacroOperationConfig::Down { keycode },
                            _ => MacroOperationConfig::Up { keycode },
                        });
                        cursor += 3;
                    }
                    _ => break,
                }
            }
            out.push(Self {
                name: format!("macro {index}"),
                operations,
            });
        }
        out
    }
}

/// The HID keycodes for a VIA packed-modifier byte, in a stable order.
fn modifier_hid_keys(packed: u8) -> Vec<u8> {
    const MODIFIERS: [(u8, u8); 4] = [
        (0b0000_0001, 0xe0), // Ctrl
        (0b0000_0010, 0xe1), // Shift
        (0b0000_0100, 0xe2), // Alt
        (0b0000_1000, 0xe3), // Gui
    ];
    // Bit 4 selects the right-hand set, which sits four keycodes further on.
    let right = if packed & 0b0001_0000 != 0 { 4 } else { 0 };
    MODIFIERS
        .iter()
        .filter(|(bit, _)| packed & bit != 0)
        .map(|(_, key)| key + right)
        .collect()
}

impl LightingConfig {
    pub fn has_key_targets(&self) -> bool {
        self.scenes
            .iter()
            .any(|cell| matches!(cell.target, LightingTargetConfig::Key { .. }))
            || self
                .conditional_scenes
                .iter()
                .any(|cell| matches!(cell.target, LightingTargetConfig::Key { .. }))
    }

    /// Resolve semantic key targets to the device's stable LED IDs. One key
    /// may expand to any number of emitters; raw LED targets pass through.
    pub fn resolve_key_targets(&self, topology: &KeyTopology) -> Result<Self> {
        fn resolve(
            target: &LightingTargetConfig,
            topology: &KeyTopology,
        ) -> Result<Vec<LightingTargetConfig>> {
            match *target {
                LightingTargetConfig::Led { led } => Ok(vec![LightingTargetConfig::led(led)]),
                LightingTargetConfig::Key { key } => {
                    if !topology
                        .keys
                        .iter()
                        .any(|candidate| candidate.id == KeyId(key))
                    {
                        bail!("unknown logical key id {key}");
                    }
                    let leds = topology.resolve_leds(&KeyId(key));
                    if leds.is_empty() {
                        bail!("logical key id {key} has no associated lighting emitters");
                    }
                    Ok(leds
                        .into_iter()
                        .map(|led| LightingTargetConfig::led(led.0))
                        .collect())
                }
            }
        }

        let mut resolved = self.clone();
        resolved.scenes.clear();
        for cell in &self.scenes {
            for target in resolve(&cell.target, topology)? {
                resolved.scenes.push(SceneConfig {
                    target,
                    ..cell.clone()
                });
            }
        }
        resolved.conditional_scenes.clear();
        for cell in &self.conditional_scenes {
            for target in resolve(&cell.target, topology)? {
                resolved.conditional_scenes.push(ConditionalSceneConfig {
                    target,
                    ..cell.clone()
                });
            }
        }
        Ok(resolved)
    }

    pub fn snapshot(&self) -> Result<LightingSnapshot> {
        let mut conditional_scenes = self.conditional_scenes.clone();
        for (index, cell) in conditional_scenes.iter_mut().enumerate() {
            cell.color = normalize_color(&cell.color)?;
            validate_conditional_scene(index, cell)?;
        }
        let mut scenes = self.scenes.clone();
        for cell in &mut scenes {
            cell.color = normalize_color(&cell.color)?;
            validate_scene(cell)?;
        }
        scenes.sort();
        let duplicate = scenes
            .windows(2)
            .find(|pair| pair[0].layer == pair[1].layer && pair[0].target == pair[1].target);
        if let Some(pair) = duplicate {
            bail!(
                "duplicate scene cell for layer {} {}",
                pair[0].layer,
                pair[0].target
            );
        }
        if let Some(effects) = &self.effects {
            for (effect, table) in &effects.params {
                if effect.trim().is_empty() {
                    bail!("[lighting.effects.params] has an empty effect name");
                }
                if table.keys().any(|name| name.trim().is_empty()) {
                    bail!("effect '{effect}' has an empty parameter name");
                }
            }
        }
        let mut wake_layers = self.wake_layers.clone();
        wake_layers.sort_unstable();
        wake_layers.dedup();
        Ok(LightingSnapshot {
            brightness: self.brightness,
            output_mode: self.output_mode,
            wake_layers,
            scene_policy: self.scene_policy,
            background: self.background.clone(),
            effects: self.effects.clone(),
            params: None,
            scenes,
            conditional_scenes: Some(conditional_scenes),
        })
    }

    pub fn from_snapshot(snapshot: &LightingSnapshot) -> Self {
        Self {
            brightness: snapshot.brightness,
            output_mode: snapshot.output_mode,
            wake_layers: snapshot.wake_layers.clone(),
            scene_policy: snapshot.scene_policy,
            background: snapshot.background.clone(),
            effects: snapshot.effects.clone(),
            scenes: snapshot.scenes.clone(),
            conditional_scenes: snapshot.conditional_scenes.clone().unwrap_or_default(),
        }
    }
}

impl RuntimeConfig {
    /// Drop parameters that still hold their firmware default, so a pulled
    /// file records only what the user actually tuned.
    pub fn retain_non_default_params(&mut self, snapshot: &Snapshot) {
        let Some(effects) = self
            .lighting
            .as_mut()
            .and_then(|lighting| lighting.effects.as_mut())
        else {
            return;
        };
        let advertised = snapshot
            .lighting
            .as_ref()
            .and_then(|lighting| lighting.params.as_ref());
        let Some(advertised) = advertised else {
            effects.params.clear();
            return;
        };
        for set in advertised {
            let Some(table) = effects.params.get_mut(&set.effect) else {
                continue;
            };
            table.retain(|name, value| {
                set.params
                    .iter()
                    .find(|spec| spec.name == *name)
                    .is_none_or(|spec| spec.default != *value)
            });
        }
        effects.params.retain(|_, table| !table.is_empty());
    }
}

/// Render live parameter values as the schema's `[lighting.effects.params.…]`
/// tables. `show` prints these as-is; `pull` prunes defaults from them first.
pub fn live_param_tables(sets: Option<&[EffectParams]>) -> BTreeMap<String, BTreeMap<String, u8>> {
    sets.unwrap_or_default()
        .iter()
        .map(|set| {
            let table = set
                .params
                .iter()
                .map(|param| (param.name.clone(), param.value))
                .collect();
            (set.effect.clone(), table)
        })
        .collect()
}

/// Report every parameter a file lists whose value the keyboard does not
/// already hold. Parameters absent from the file are not compared: a file owns
/// only what it names.
pub fn param_differences(desired: &LightingSnapshot, live: &LightingSnapshot) -> Vec<String> {
    let mut result = Vec::new();
    let Some(wanted) = desired.effects.as_ref() else {
        return result;
    };
    for (effect, table) in &wanted.params {
        let set = live
            .params
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|set| set.effect == *effect);
        for (name, value) in table {
            let current = set.and_then(|set| set.params.iter().find(|param| param.name == *name));
            match current {
                Some(param) if param.value == *value => {}
                Some(param) => result.push(format!(
                    "lighting parameter {effect}.{name}: file {value} != keyboard {}",
                    param.value
                )),
                None => result.push(format!(
                    "lighting parameter {effect}.{name}: file {value} != keyboard (not advertised)"
                )),
            }
        }
    }
    result
}

/// Report slots the keyboard holds beyond what the file describes.
///
/// A file that claims a table but names fewer slots than the device has is
/// asking for the rest to be empty. Walking only as far as the file goes would
/// leave those in place and report nothing, which is how a shrinking table
/// silently keeps its tail.
fn report_surplus<T>(
    result: &mut Vec<String>,
    noun: &str,
    wanted: usize,
    present: &[T],
    is_empty: impl Fn(&T) -> bool,
) {
    for (index, slot) in present.iter().enumerate().skip(wanted) {
        if !is_empty(slot) {
            result.push(format!(
                "{noun} {index}: on the keyboard but not in the file"
            ));
        }
    }
}

/// Whether a fork slot is unprogrammed. A fork that triggers on nothing can
/// never fire, which is how the firmware spells an empty slot.
fn is_empty_fork(fork: &rynk::rmk_types::fork::Fork) -> bool {
    fork.trigger == rynk::rmk_types::action::KeyAction::No
}

pub fn differences(desired: &Snapshot, live: &Snapshot) -> Vec<String> {
    let mut result = Vec::new();
    if desired.rows != live.rows || desired.cols != live.cols {
        result.push(format!(
            "matrix: file {}x{} != keyboard {}x{}",
            desired.rows, desired.cols, live.rows, live.cols
        ));
        return result;
    }
    if desired.bluetooth_name.is_some() && desired.bluetooth_name != live.bluetooth_name {
        result.push(format!(
            "bluetooth name: file '{}' != keyboard '{}'",
            desired.bluetooth_name.as_deref().unwrap_or_default(),
            live.bluetooth_name.as_deref().unwrap_or("unsupported")
        ));
    }
    if desired.default_layer != live.default_layer {
        result.push(format!(
            "default layer: file {} != keyboard {}",
            desired.default_layer, live.default_layer
        ));
    }
    let layer_size = usize::from(desired.rows) * usize::from(desired.cols);
    for layer in 0..desired.layers.len() {
        for offset in 0..layer_size {
            let wanted = desired
                .layers
                .get(layer)
                .map_or(KeyAction::No, |keys| keys[offset]);
            let present = live
                .layers
                .get(layer)
                .map_or(KeyAction::No, |keys| keys[offset]);
            if wanted != present {
                result.push(format!(
                    "layer {layer} r{},c{}: file {} != keyboard {}",
                    offset / usize::from(desired.cols),
                    offset % usize::from(desired.cols),
                    render_key_action(wanted, &[]),
                    render_key_action(present, &[]),
                ));
            }
        }
    }
    // A table the source is silent about is left alone, so only a `Some`
    // participates in the diff.
    for (name, differs) in [
        (
            "global behavior timing",
            desired.behaviors.config.is_some() && desired.behaviors.config != live.behaviors.config,
        ),
        (
            "global behavior options",
            desired.behaviors.options.is_some()
                && desired.behaviors.options != live.behaviors.options,
        ),
        (
            "morse profiles",
            desired.behaviors.morse_profiles.is_some()
                && desired.behaviors.morse_profiles != live.behaviors.morse_profiles,
        ),
        (
            "morse hold trigger positions",
            desired.behaviors.hold_trigger_positions.is_some()
                && desired.behaviors.hold_trigger_positions
                    != live.behaviors.hold_trigger_positions,
        ),
        (
            "auto mouse layers",
            desired.behaviors.auto_mouse_layers.is_some()
                && desired.behaviors.auto_mouse_layers != live.behaviors.auto_mouse_layers,
        ),
    ] {
        if differs {
            result.push(format!("{name}: file differs from keyboard"));
        }
    }
    if let Some(wanted) = &desired.behaviors.morses {
        let present = live.behaviors.morses.as_deref().unwrap_or_default();
        for (index, morse) in wanted.iter().enumerate() {
            if present.get(index) != Some(morse) {
                result.push(format!("morse {index}: file differs from keyboard"));
            }
        }
        report_surplus(&mut result, "morse", wanted.len(), present, |morse| {
            morse.actions.is_empty()
        });
    }
    if let Some(wanted) = &desired.behaviors.combos {
        let present = live.behaviors.combos.as_deref().unwrap_or_default();
        for (index, combo) in wanted.iter().enumerate() {
            if present.get(index) != Some(combo) {
                result.push(format!("combo {index}: file differs from keyboard"));
            }
        }
        // An unprogrammed combo outputs nothing; that, not an empty trigger
        // list, is how the firmware spells a free slot.
        report_surplus(&mut result, "combo", wanted.len(), present, |combo| {
            combo.output() == rynk::rmk_types::action::KeyAction::No
        });
    }
    if let Some(wanted) = &desired.behaviors.forks {
        let present = live.behaviors.forks.as_deref().unwrap_or_default();
        for (index, fork) in wanted.iter().enumerate() {
            if present.get(index) != Some(fork) {
                result.push(format!("fork {index}: file differs from keyboard"));
            }
        }
        report_surplus(&mut result, "fork", wanted.len(), present, is_empty_fork);
    }
    if let Some(wanted) = &desired.behaviors.macros {
        let present = live.behaviors.macros.as_deref().unwrap_or_default();
        // Macro space is zero-filled past the end, so compare only as far as
        // the file describes.
        if present.len() < wanted.len() || present[..wanted.len()] != wanted[..] {
            result.push(format!("macro space: {} byte(s) differ", wanted.len()));
        }
    }
    if let Some(wanted) = &desired.pointing {
        let present = live.pointing.as_ref();
        let matches = present.is_some_and(|present| {
            wanted.devices() == present.devices() && wanted.overrides() == present.overrides()
        });
        if !matches {
            result.push("pointing configuration: file differs from keyboard".into());
        }
    }

    match (&desired.lighting, &live.lighting) {
        (Some(wanted), Some(present)) => {
            if wanted.brightness != present.brightness {
                result.push(format!(
                    "lighting brightness: file {} != keyboard {}",
                    wanted.brightness, present.brightness
                ));
            }
            if wanted.output_mode != present.output_mode {
                result.push(format!(
                    "lighting output mode: file {:?} != keyboard {:?}",
                    wanted.output_mode, present.output_mode
                ));
            }
            if wanted.wake_layers != present.wake_layers {
                result.push(format!(
                    "lighting wake layers: file {:?} != keyboard {:?}",
                    wanted.wake_layers, present.wake_layers
                ));
            }
            if wanted.scene_policy != present.scene_policy {
                result.push(format!(
                    "lighting scene policy: file {:?} != keyboard {:?}",
                    wanted.scene_policy, present.scene_policy
                ));
            }
            if wanted.background != present.background {
                result.push("lighting background differs".into());
            }
            let wanted_selection = wanted.effects.as_ref().map(EffectsConfig::selection);
            let present_selection = present.effects.as_ref().map(EffectsConfig::selection);
            if wanted_selection != present_selection {
                result.push(format!(
                    "effects state: file {wanted_selection:?} != keyboard {present_selection:?}"
                ));
            }
            result.extend(param_differences(wanted, present));
            let wanted_cells = wanted
                .scenes
                .iter()
                .map(|cell| ((cell.layer, cell.target), cell))
                .collect::<BTreeMap<_, _>>();
            let present_cells = present
                .scenes
                .iter()
                .map(|cell| ((cell.layer, cell.target), cell))
                .collect::<BTreeMap<_, _>>();
            for key in wanted_cells.keys().chain(present_cells.keys()) {
                if wanted_cells.get(key) != present_cells.get(key) {
                    result.push(format!(
                        "lighting scene layer {} {}: file {:?} != keyboard {:?}",
                        key.0,
                        key.1,
                        wanted_cells.get(key),
                        present_cells.get(key),
                    ));
                }
            }
            result.sort();
            result.dedup();

            // Reported by position, and appended after the sort, because the
            // table's order is part of its meaning: two rules that swap places
            // are a real difference even though the set is unchanged.
            if let Some(wanted_conditional) = wanted.conditional_scenes.as_ref() {
                match present.conditional_scenes.as_ref() {
                    None if wanted_conditional.is_empty() => {}
                    None => result.push(format!(
                        "lighting conditional rules: file has {} but the keyboard exposes no runtime conditional table",
                        wanted_conditional.len()
                    )),
                    Some(live) => {
                        for index in 0..wanted_conditional.len().max(live.len()) {
                            let (file, keyboard) = (wanted_conditional.get(index), live.get(index));
                            if file != keyboard {
                                result.push(format!(
                                    "lighting conditional rule {index}: file {file:?} != keyboard {keyboard:?}"
                                ));
                            }
                        }
                    }
                }
            }
        }
        (Some(_), None) => result.push("file configures lighting but keyboard exposes none".into()),
        (None, _) => {}
    }
    result
}

pub fn parse_keys(text: &str) -> Result<Vec<u16>> {
    let result = parse_keys_for_matrix(text, ROWS, COLS)?;
    for hole in HOLES {
        if result[hole] != 0 {
            bail!(
                "physical hole r{},c{} must be --",
                hole / usize::from(COLS),
                hole % usize::from(COLS)
            );
        }
    }
    Ok(result)
}

pub fn parse_keys_for_matrix(text: &str, rows_count: u8, cols_count: u8) -> Result<Vec<u16>> {
    let rows = text
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if rows.len() != usize::from(rows_count) {
        bail!(
            "keys must contain {rows_count} non-empty rows, found {}",
            rows.len()
        );
    }
    let mut result = Vec::with_capacity(usize::from(rows_count) * usize::from(cols_count));
    for (row, line) in rows.iter().enumerate() {
        let tokens = split_grid_tokens(line)?;
        if tokens.len() != usize::from(cols_count) {
            bail!(
                "row {row} must contain {cols_count} keys, found {}",
                tokens.len()
            );
        }
        for token in tokens {
            result.push(if token == "--" {
                0
            } else {
                crate::keycodes::parse_keycode(&token)?
            });
        }
    }
    Ok(result)
}

fn split_grid_tokens(line: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (index, character) in line.char_indices() {
        if character.is_whitespace() && depth == 0 {
            if let Some(token_start) = start.take() {
                tokens.push(line[token_start..index].trim().to_owned());
            }
            continue;
        }
        start.get_or_insert(index);
        match character {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1).context("unmatched ')' in key grid")?;
            }
            _ => {}
        }
    }
    if depth != 0 {
        bail!("unclosed '(' in key grid");
    }
    if let Some(token_start) = start {
        tokens.push(line[token_start..].trim().to_owned());
    }
    Ok(tokens)
}

pub fn parse_key_actions(text: &str, profile_names: &[String]) -> Result<Vec<KeyAction>> {
    let result = parse_key_actions_for_matrix(text, profile_names, ROWS, COLS)?;
    for hole in HOLES {
        if result[hole] != KeyAction::No {
            bail!(
                "physical hole r{},c{} must be --",
                hole / usize::from(COLS),
                hole % usize::from(COLS)
            );
        }
    }
    Ok(result)
}

pub fn parse_key_actions_for_matrix(
    text: &str,
    profile_names: &[String],
    rows_count: u8,
    cols_count: u8,
) -> Result<Vec<KeyAction>> {
    let rows = text
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if rows.len() != usize::from(rows_count) {
        bail!(
            "keys must contain {rows_count} non-empty rows, found {}",
            rows.len()
        );
    }
    let mut result = Vec::with_capacity(usize::from(rows_count) * usize::from(cols_count));
    for (row, line) in rows.iter().enumerate() {
        let tokens = split_grid_tokens(line)?;
        if tokens.len() != usize::from(cols_count) {
            bail!(
                "row {row} must contain {cols_count} keys, found {}",
                tokens.len()
            );
        }
        for token in tokens {
            result.push(parse_key_action(&token, profile_names)?);
        }
    }
    Ok(result)
}

fn parse_key_action(token: &str, profile_names: &[String]) -> Result<KeyAction> {
    if token == "--" {
        return Ok(KeyAction::No);
    }
    for (name, kind) in [("MT", 0u8), ("LT", 1), ("TH", 2)] {
        let Some(inner) = token
            .strip_prefix(name)
            .and_then(|rest| rest.strip_prefix('('))
            .and_then(|rest| rest.strip_suffix(')'))
        else {
            continue;
        };
        let arguments = split_call_arguments(inner)?;
        if arguments.len() != 3 {
            break;
        }
        let profile = profile_names
            .iter()
            .position(|candidate| candidate == &arguments[2])
            .with_context(|| format!("unknown morse profile '{}'", arguments[2]))?;
        let profile = u8::try_from(profile).context("more than 255 morse profiles")?;
        let (tap, hold) = match kind {
            0 => (
                single_action(&arguments[0])?,
                Action::Modifier(modifier_list_to_wire(
                    &arguments[1]
                        .split('|')
                        .map(|name| name.trim().to_owned())
                        .collect::<Vec<_>>(),
                )?),
            ),
            1 => (
                single_action(&arguments[1])?,
                Action::LayerOn(arguments[0].parse().context("invalid LT layer")?),
            ),
            _ => (single_action(&arguments[0])?, single_action(&arguments[1])?),
        };
        return Ok(KeyAction::TapHold(tap, hold, profile));
    }
    Ok(crate::rynk_keycode::from_via_keycode(
        crate::keycodes::parse_keycode(token)?,
    ))
}

fn single_action(text: &str) -> Result<Action> {
    match crate::rynk_keycode::from_via_keycode(crate::keycodes::parse_keycode(text)?) {
        KeyAction::Single(action) => Ok(action),
        _ => bail!("'{text}' is not a single action"),
    }
}

fn split_call_arguments(text: &str) -> Result<Vec<String>> {
    let mut arguments = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, character) in text.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.checked_sub(1).context("unmatched ')' in action")?,
            ',' if depth == 0 => {
                arguments.push(text[start..index].trim().to_owned());
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        bail!("unclosed '(' in action");
    }
    arguments.push(text[start..].trim().to_owned());
    Ok(arguments)
}

pub fn render_keys(keys: &[u16]) -> String {
    render_keys_for_matrix(keys, ROWS, COLS)
}

pub fn render_keys_for_matrix(keys: &[u16], rows_count: u8, cols_count: u8) -> String {
    let mut text = String::from("\n");
    for row in 0..usize::from(rows_count) {
        for col in 0..usize::from(cols_count) {
            if col > 0 {
                text.push(' ');
            }
            let offset = row * usize::from(cols_count) + col;
            if keys[offset] == 0 {
                text.push_str("--");
            } else {
                // The grid format is whitespace-delimited, so keep composite
                // keycodes as a single token even when the human formatter
                // normally inserts a space after a comma.
                text.push_str(&crate::keycodes::format_keycode(keys[offset]).replace(", ", ","));
            }
        }
        text.push('\n');
    }
    text
}

pub fn render_key_actions(keys: &[KeyAction], profile_names: &[String]) -> String {
    render_key_actions_for_matrix(keys, profile_names, ROWS, COLS)
}

pub fn render_key_actions_for_matrix(
    keys: &[KeyAction],
    profile_names: &[String],
    rows_count: u8,
    cols_count: u8,
) -> String {
    let mut text = String::from("\n");
    for row in 0..usize::from(rows_count) {
        for col in 0..usize::from(cols_count) {
            if col > 0 {
                text.push(' ');
            }
            let action = keys[row * usize::from(cols_count) + col];
            text.push_str(&render_key_action(action, profile_names).replace(", ", ","));
        }
        text.push('\n');
    }
    text
}

fn render_key_action(action: KeyAction, profile_names: &[String]) -> String {
    let KeyAction::TapHold(tap, hold, profile) = action else {
        let code = crate::rynk_keycode::to_via_keycode(action);
        return if code == 0 && action == KeyAction::No {
            "--".to_owned()
        } else {
            crate::keycodes::format_keycode(code)
        };
    };
    let suffix = profile_names
        .get(usize::from(profile))
        .map(|name| format!(", {name}"))
        .unwrap_or_default();
    match hold {
        Action::Modifier(modifiers) => format!(
            "MT({}, {}{suffix})",
            action_name(tap),
            modifier_list_from_wire(modifiers).join(" | ")
        ),
        Action::LayerOn(layer) => format!("LT({layer}, {}{suffix})", action_name(tap)),
        _ => format!("TH({}, {}{suffix})", action_name(tap), action_name(hold)),
    }
}

pub fn action_to_code(action: KeyAction, layer: usize, offset: usize) -> Result<u16> {
    let code = crate::rynk_keycode::to_via_keycode(action);
    if code == 0 && !matches!(action, KeyAction::No) {
        bail!(
            "action {action:?} at layer {layer} r{},c{} cannot be represented in runtime TOML",
            offset / usize::from(COLS),
            offset % usize::from(COLS)
        );
    }
    Ok(code)
}

pub fn normalize_color(text: &str) -> Result<String> {
    let (r, g, b) = crate::color::parse_color(text)?;
    Ok(format!("#{r:02x}{g:02x}{b:02x}"))
}

/// Structural checks only, mirroring [`validate_scene`] and adding the battery
/// bounds the firmware would otherwise reject on apply. Nothing here contacts
/// the keyboard, so `config validate` stays usable offline.
pub fn validate_conditional_scene(index: usize, cell: &ConditionalSceneConfig) -> Result<()> {
    let timings_set = cell.period_ms.is_some()
        || cell.phase_ms.is_some()
        || cell.duty.is_some()
        || cell.step_ms.is_some();
    match cell.effect {
        EffectKind::Solid if timings_set => {
            bail!(
                "solid conditional rule {index} ({}) has timing options",
                cell.target
            )
        }
        EffectKind::Solid => {}
        EffectKind::Blink => {
            if cell.period_ms.unwrap_or(0) == 0 || cell.duty.unwrap_or(101) > 100 {
                bail!(
                    "blink conditional rule {index} ({}) needs a non-zero period_ms and a duty of 0..=100",
                    cell.target
                );
            }
        }
        EffectKind::Breathe => {
            if cell.period_ms.unwrap_or(0) == 0 || cell.step_ms.unwrap_or(0) == 0 {
                bail!(
                    "breathe conditional rule {index} ({}) needs a non-zero period_ms and step_ms",
                    cell.target
                );
            }
        }
    }
    if let Some(battery) = cell.battery {
        let over = |level: Option<u8>| level.is_some_and(|value| value > 100);
        if over(battery.min_level) || over(battery.max_level) {
            bail!(
                "conditional rule {index} ({}) has a battery level above 100",
                cell.target
            );
        }
        if matches!((battery.min_level, battery.max_level), (Some(min), Some(max)) if min > max) {
            let (min, max) = (battery.min_level.unwrap(), battery.max_level.unwrap());
            bail!(
                "conditional rule {index} ({}) has battery min_level {min} above max_level {max}",
                cell.target
            );
        }
    }
    if let Some(connection) = cell.connection {
        if connection.transport.is_none()
            && connection.profile.is_none()
            && connection.ble_state.is_none()
            && connection.bonded.is_none()
            && connection.usb_connected.is_none()
        {
            bail!(
                "conditional rule {index} ({}) has a connection condition that names no gate",
                cell.target
            );
        }
        // Both bounds describe the same slot space, so they move together when
        // the board's profile count changes.
        if connection
            .profile
            .is_some_and(|profile| profile > MAX_BLE_SLOT)
        {
            bail!(
                "conditional rule {index} ({}) names a BLE profile past the board's slots (0-{MAX_BLE_SLOT})",
                cell.target
            );
        }
        if connection
            .bonded
            .is_some_and(|bonded| bonded.slot > MAX_BLE_SLOT)
        {
            bail!(
                "conditional rule {index} ({}) names a bonded slot past the board's slots (0-{MAX_BLE_SLOT})",
                cell.target
            );
        }
    }
    Ok(())
}

pub fn validate_scene(cell: &SceneConfig) -> Result<()> {
    match cell.effect {
        EffectKind::Solid => {
            if cell.period_ms.is_some()
                || cell.phase_ms.is_some()
                || cell.duty.is_some()
                || cell.step_ms.is_some()
            {
                bail!(
                    "solid scene layer {} {} has timing options",
                    cell.layer,
                    cell.target
                );
            }
        }
        EffectKind::Blink => {
            if cell.period_ms.unwrap_or(0) == 0
                || cell.duty.unwrap_or(101) > 100
                || cell.step_ms.is_some()
            {
                bail!(
                    "invalid blink scene at layer {} {}",
                    cell.layer,
                    cell.target
                );
            }
        }
        EffectKind::Breathe => {
            if cell.period_ms.unwrap_or(0) < 2
                || cell.step_ms.unwrap_or(0) == 0
                || cell.duty.is_some()
            {
                bail!(
                    "invalid breathe scene at layer {} {}",
                    cell.layer,
                    cell.target
                );
            }
        }
    }
    Ok(())
}

/// Split a wire effect into the flat fields both scene tables use.
pub fn effect_from_wire(
    effect: LightingEffect,
) -> (
    LightingRgb8,
    EffectKind,
    Option<u32>,
    Option<u32>,
    Option<u8>,
    Option<u16>,
) {
    match effect {
        LightingEffect::Solid { color } => (color, EffectKind::Solid, None, None, None, None),
        LightingEffect::Blink {
            color,
            period_ms,
            phase_ms,
            duty,
        } => (
            color,
            EffectKind::Blink,
            Some(period_ms),
            Some(phase_ms),
            Some(duty),
            None,
        ),
        LightingEffect::Breathe {
            color,
            period_ms,
            phase_ms,
            step_ms,
        } => (
            color,
            EffectKind::Breathe,
            Some(period_ms),
            Some(phase_ms),
            None,
            Some(step_ms),
        ),
    }
}

pub fn scene_from_wire(cell: LightingSceneCell) -> SceneConfig {
    let (color, effect, period_ms, phase_ms, duty, step_ms) = effect_from_wire(cell.effect);
    SceneConfig {
        layer: cell.layer,
        target: LightingTargetConfig::led(cell.led_id.0),
        color: format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b),
        effect,
        period_ms,
        phase_ms,
        duty,
        step_ms,
    }
}

pub fn conditional_scene_from_wire(
    extended: LightingExtendedConditionalSceneCell,
) -> ConditionalSceneConfig {
    let connection = extended.connection.map(|c| ConnectionConditionConfig {
        transport: c.transport.map(|transport| match transport {
            LightingActiveTransport::Usb => TransportConfig::Usb,
            LightingActiveTransport::Ble => TransportConfig::Ble,
            LightingActiveTransport::NoneActive => TransportConfig::None,
        }),
        profile: c.profile,
        ble_state: c.ble_state.map(|state| match state {
            WireBleState::Advertising => BleStateConfig::Advertising,
            WireBleState::Connected => BleStateConfig::Connected,
            WireBleState::Inactive => BleStateConfig::Inactive,
        }),
        bonded: c.bonded.map(|bonded| BondedSlotConditionConfig {
            slot: bonded.slot,
            bonded: bonded.bonded,
        }),
        usb_connected: c.usb_connected,
    });
    let effects = extended
        .effects
        .map(|c| EffectsConditionConfig { enabled: c.enabled });
    let cell = extended.cell;
    let (color, effect, period_ms, phase_ms, duty, step_ms) = effect_from_wire(cell.effect);
    ConditionalSceneConfig {
        connection,
        effects,
        target: LightingTargetConfig::led(cell.led_id.0),
        color: format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b),
        effect,
        period_ms,
        phase_ms,
        duty,
        step_ms,
        output_mode: cell.conditions.output_mode.map(output_mode_from_wire),
        layer: cell.conditions.layer.map(|c| LayerConditionConfig {
            layer: c.layer,
            active: c.active,
        }),
        battery: cell.conditions.battery.map(|c| BatteryConditionConfig {
            node: c.node.0,
            min_level: c.min_level,
            max_level: c.max_level,
            charge: match c.charge {
                LightingChargeCondition::Any => ChargeConditionConfig::Any,
                LightingChargeCondition::Charging => ChargeConditionConfig::Charging,
                LightingChargeCondition::Discharging => ChargeConditionConfig::Discharging,
                LightingChargeCondition::Unknown => ChargeConditionConfig::Unknown,
            },
        }),
    }
}

/// Build a wire effect from the flat fields both scene tables use.
pub fn effect_to_wire(
    color: &str,
    kind: EffectKind,
    period_ms: Option<u32>,
    phase_ms: Option<u32>,
    duty: Option<u8>,
    step_ms: Option<u16>,
) -> Result<LightingEffect> {
    let (r, g, b) = crate::color::parse_color(color)?;
    let color = LightingRgb8 { r, g, b };
    Ok(match kind {
        EffectKind::Solid => LightingEffect::Solid { color },
        EffectKind::Blink => LightingEffect::Blink {
            color,
            period_ms: period_ms.context("blink period_ms is required")?,
            phase_ms: phase_ms.unwrap_or(0),
            duty: duty.context("blink duty is required")?,
        },
        EffectKind::Breathe => LightingEffect::Breathe {
            color,
            period_ms: period_ms.context("breathe period_ms is required")?,
            phase_ms: phase_ms.unwrap_or(0),
            step_ms: step_ms.context("breathe step_ms is required")?,
        },
    })
}

pub fn scene_to_wire(cell: &SceneConfig) -> Result<LightingSceneCell> {
    Ok(LightingSceneCell {
        layer: cell.layer,
        led_id: LightingLedId(
            cell.target
                .led_id()
                .context("semantic scene target must be resolved before wire encoding")?,
        ),
        effect: effect_to_wire(
            &cell.color,
            cell.effect,
            cell.period_ms,
            cell.phase_ms,
            cell.duty,
            cell.step_ms,
        )?,
    })
}

pub fn conditional_scene_to_wire(
    cell: &ConditionalSceneConfig,
) -> Result<LightingExtendedConditionalSceneCell> {
    let connection = cell.connection.map(|c| LightingConnectionCondition {
        transport: c.transport.map(|transport| match transport {
            TransportConfig::Usb => LightingActiveTransport::Usb,
            TransportConfig::Ble => LightingActiveTransport::Ble,
            TransportConfig::None => LightingActiveTransport::NoneActive,
        }),
        profile: c.profile,
        ble_state: c.ble_state.map(|state| match state {
            BleStateConfig::Advertising => WireBleState::Advertising,
            BleStateConfig::Connected => WireBleState::Connected,
            BleStateConfig::Inactive => WireBleState::Inactive,
        }),
        bonded: c.bonded.map(|bonded| LightingBondedSlotCondition {
            slot: bonded.slot,
            bonded: bonded.bonded,
        }),
        usb_connected: c.usb_connected,
    });
    let base = LightingConditionalSceneCell {
        conditions: LightingConditionSet {
            output_mode: cell.output_mode.map(output_mode_to_wire),
            layer: cell.layer.map(|c| LightingLayerCondition {
                layer: c.layer,
                active: c.active,
            }),
            battery: cell.battery.map(|c| LightingBatteryCondition {
                node: LightingNodeId(c.node),
                min_level: c.min_level,
                max_level: c.max_level,
                charge: match c.charge {
                    ChargeConditionConfig::Any => LightingChargeCondition::Any,
                    ChargeConditionConfig::Charging => LightingChargeCondition::Charging,
                    ChargeConditionConfig::Discharging => LightingChargeCondition::Discharging,
                    ChargeConditionConfig::Unknown => LightingChargeCondition::Unknown,
                },
            }),
        },
        led_id: LightingLedId(
            cell.target
                .led_id()
                .context("semantic conditional target must be resolved before wire encoding")?,
        ),
        effect: effect_to_wire(
            &cell.color,
            cell.effect,
            cell.period_ms,
            cell.phase_ms,
            cell.duty,
            cell.step_ms,
        )?,
    };
    Ok(LightingExtendedConditionalSceneCell {
        cell: base,
        connection,
        effects: cell
            .effects
            .map(|c| LightingEffectsCondition { enabled: c.enabled }),
    })
}

pub fn background_from_wire(state: LightingBackgroundState) -> BackgroundConfig {
    BackgroundConfig {
        enabled: state.enabled,
        hue: state.hue,
        saturation: state.saturation,
        value: state.value,
        speed: state.speed,
        mode: match state.mode {
            LightingBackgroundMode::Solid => BackgroundModeConfig::Solid,
            LightingBackgroundMode::Breathe => BackgroundModeConfig::Breathe,
        },
    }
}

pub fn background_to_wire(state: &BackgroundConfig) -> LightingBackgroundState {
    LightingBackgroundState {
        enabled: state.enabled,
        hue: state.hue,
        saturation: state.saturation,
        value: state.value,
        speed: state.speed,
        mode: match state.mode {
            BackgroundModeConfig::Solid => LightingBackgroundMode::Solid,
            BackgroundModeConfig::Breathe => LightingBackgroundMode::Breathe,
        },
    }
}

pub fn output_mode_from_wire(mode: LightingOutputMode) -> OutputModeConfig {
    match mode {
        LightingOutputMode::AlwaysOn => OutputModeConfig::AlwaysOn,
        LightingOutputMode::AlwaysOff => OutputModeConfig::AlwaysOff,
        LightingOutputMode::PoweredOnly => OutputModeConfig::PoweredOnly,
    }
}

pub fn output_mode_to_wire(mode: OutputModeConfig) -> LightingOutputMode {
    match mode {
        OutputModeConfig::AlwaysOn => LightingOutputMode::AlwaysOn,
        OutputModeConfig::AlwaysOff => LightingOutputMode::AlwaysOff,
        OutputModeConfig::PoweredOnly => LightingOutputMode::PoweredOnly,
    }
}

pub fn scene_policy_from_wire(policy: LightingLayerPolicy) -> ScenePolicyConfig {
    match policy {
        LightingLayerPolicy::EffectiveOnly => ScenePolicyConfig::EffectiveOnly,
        LightingLayerPolicy::ActiveStack => ScenePolicyConfig::ActiveStack,
    }
}

pub fn scene_policy_to_wire(policy: ScenePolicyConfig) -> LightingLayerPolicy {
    match policy {
        ScenePolicyConfig::EffectiveOnly => LightingLayerPolicy::EffectiveOnly,
        ScenePolicyConfig::ActiveStack => LightingLayerPolicy::ActiveStack,
    }
}

/// RMK gives every layer slot its fixed capacity and initializes the unused
/// ones as transparent. `No` is intentionally not empty: an all-`No` layer
/// blocks fallback and must survive an export/reapply cycle.
pub fn trim_trailing_transparent_layers(layers: &mut Vec<Vec<KeyAction>>) {
    while layers.len() > 1
        && layers.last().is_some_and(|layer| {
            layer
                .iter()
                .all(|action| matches!(action, KeyAction::Transparent))
        })
    {
        layers.pop();
    }
}

/// The file speaks in effect and palette *names*; the protocol speaks in
/// indices into the lists a keyboard advertises. These two functions are the
/// only place that translation lives, so the CLI and the browser cannot drift.
pub fn effects_from_wire(
    state: LightingExtensionState,
    overlay: Option<u8>,
    effect_names: &[String],
    palette_names: &[String],
    params: BTreeMap<String, BTreeMap<String, u8>>,
) -> Result<EffectsConfig> {
    let name = |names: &[String], index: u8, what: &str| {
        names
            .get(usize::from(index))
            .cloned()
            .with_context(|| format!("extension {what} index is outside its advertised name list"))
    };
    Ok(EffectsConfig {
        effect: name(effect_names, state.effect, "effect")?,
        overlay: overlay
            .map(|index| name(effect_names, index, "overlay"))
            .transpose()?,
        palette: name(palette_names, state.palette, "palette")?,
        value: state.value,
        speed: state.speed,
        params,
    })
}

pub fn effects_to_wire(
    effects: &EffectsConfig,
    effect_names: &[String],
    palette_names: &[String],
) -> Result<(LightingExtensionState, Option<u8>)> {
    // An unknown name is the most common way a file and a keyboard disagree,
    // so the message lists what the keyboard does advertise rather than
    // leaving the user to go read it out of `lighting extension`.
    let index = |names: &[String], wanted: &str, what: &str| -> Result<u8> {
        let found = names
            .iter()
            .position(|name| name == wanted)
            .with_context(|| {
                format!(
                    "unknown extension {what} '{wanted}'; the keyboard advertises: {}",
                    names.join(", ")
                )
            })?;
        u8::try_from(found).with_context(|| format!("{what} index exceeds u8"))
    };
    Ok((
        LightingExtensionState {
            effect: index(effect_names, &effects.effect, "effect")?,
            palette: index(palette_names, &effects.palette, "palette")?,
            value: effects.value,
            speed: effects.speed,
        },
        effects
            .overlay
            .as_deref()
            .map(|overlay| index(effect_names, overlay, "overlay effect"))
            .transpose()?,
    ))
}

/// One parameter write addressed the way the protocol addresses it: by the
/// effect's index in the advertised effect list and the parameter's ordinal
/// within that effect's own list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamWrite {
    pub effect: u8,
    pub index: u8,
    pub value: u8,
    /// What the keyboard currently holds, so a caller can skip no-op writes.
    pub current: u8,
    /// `Effect.Parameter` as the file spells it, for error context.
    pub label: String,
}

/// Resolve the parameters a file names against what the keyboard advertises.
/// Bounds are checked here rather than left to the firmware so a host reports
/// the offending name instead of a bare protocol rejection.
pub fn params_to_writes(
    wanted: &BTreeMap<String, BTreeMap<String, u8>>,
    advertised: Option<&[EffectParams]>,
) -> Result<Vec<ParamWrite>> {
    let advertised =
        advertised.context("the keyboard does not expose per-effect extension parameters")?;
    let mut writes = Vec::new();
    for (effect, table) in wanted {
        let set = advertised
            .iter()
            .find(|set| set.effect == *effect)
            .with_context(|| format!("effect '{effect}' advertises no parameters"))?;
        for (name, value) in table {
            let index = set
                .params
                .iter()
                .position(|param| param.name == *name)
                .with_context(|| format!("effect '{effect}' has no parameter '{name}'"))?;
            let param = &set.params[index];
            if *value < param.min || *value > param.max {
                bail!(
                    "parameter '{effect}.{name}' accepts {}..={}, file requests {value}",
                    param.min,
                    param.max
                );
            }
            writes.push(ParamWrite {
                effect: set.index,
                index: u8::try_from(index).context("parameter index exceeds u8")?,
                value: *value,
                current: param.value,
                label: format!("{effect}.{name}"),
            });
        }
    }
    Ok(writes)
}

/// The inverse of [`params_to_writes`]: name the ordinals a host holds so they
/// can be written back out as `[lighting.effects.params.…]` tables.
pub fn param_tables_from_wire(
    writes: &[(u8, u8, u8)],
    advertised: &[EffectParams],
) -> Result<BTreeMap<String, BTreeMap<String, u8>>> {
    let mut tables: BTreeMap<String, BTreeMap<String, u8>> = BTreeMap::new();
    for (effect, index, value) in writes.iter().copied() {
        let set = advertised
            .iter()
            .find(|set| set.index == effect)
            .with_context(|| format!("effect index {effect} advertises no parameters"))?;
        let param = set
            .params
            .get(usize::from(index))
            .with_context(|| format!("effect '{}' has no parameter {index}", set.effect))?;
        tables
            .entry(set.effect.clone())
            .or_default()
            .insert(param.name.clone(), value);
    }
    Ok(tables)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_runtime_config(bluetooth_name: Option<&str>) -> RuntimeConfig {
        RuntimeConfig {
            rows: ROWS,
            cols: COLS,
            bluetooth_name: bluetooth_name.map(str::to_owned),
            default_layer: 0,
            layers: vec![LayerConfig {
                id: "base".into(),
                name: "Base".into(),
                keys: render_key_actions(&[KeyAction::No; LAYER_SIZE], &[]),
            }],
            morses: Vec::new(),
            combos: Vec::new(),
            macros: Vec::new(),
            forks: Vec::new(),
            behavior: None,
            pointing: None,
            lighting: None,
        }
    }

    #[test]
    fn semantic_key_targets_round_trip_and_expand_to_every_emitter() {
        use rynk::rmk_types::protocol::rynk::{LightingLed, LightingLedId, LightingMatrixPosition};

        let key_target: SceneConfig = toml::from_str(
            r##"layer = 2
key = 0
color = "#ff0000"
"##,
        )
        .unwrap();
        assert_eq!(key_target.target, LightingTargetConfig::key(0));
        assert!(toml::to_string(&key_target).unwrap().contains("key = 0"));

        let matrix = LightingMatrixPosition { row: 0, col: 0 };
        let emitter = |id| LightingLed {
            id: LightingLedId(id),
            key: Some(matrix),
            position: None,
            zone_start: 0,
            zone_len: 0,
        };
        let topology = KeyTopology::new(3, vec![matrix], vec![emitter(34), emitter(80)]).unwrap();
        let conditional_target: ConditionalSceneConfig = toml::from_str(
            r##"key = 0
color = "#0000ff"
"##,
        )
        .unwrap();
        let lighting = LightingConfig {
            brightness: 100,
            output_mode: OutputModeConfig::AlwaysOn,
            scene_policy: ScenePolicyConfig::EffectiveOnly,
            background: BackgroundConfig {
                enabled: false,
                hue: 0,
                saturation: 0,
                value: 0,
                speed: 0,
                mode: BackgroundModeConfig::Solid,
            },
            effects: None,
            scenes: vec![key_target],
            conditional_scenes: vec![conditional_target],
        };

        let mut config = minimal_runtime_config(None);
        config.lighting = Some(lighting.clone());
        let encoded = config.to_toml().unwrap();
        let decoded = RuntimeConfig::from_toml(&encoded).unwrap();
        assert!(decoded.lighting.unwrap().has_key_targets());

        let resolved = lighting.resolve_key_targets(&topology).unwrap();
        assert_eq!(
            resolved
                .scenes
                .iter()
                .map(|scene| scene.target.clone())
                .collect::<Vec<_>>(),
            vec![LightingTargetConfig::led(34), LightingTargetConfig::led(80)]
        );
        assert!(resolved
            .scenes
            .iter()
            .all(|scene| scene_to_wire(scene).is_ok()));
        assert_eq!(
            resolved
                .conditional_scenes
                .iter()
                .map(|scene| scene.target)
                .collect::<Vec<_>>(),
            vec![LightingTargetConfig::led(34), LightingTargetConfig::led(80)]
        );
        assert!(resolved
            .conditional_scenes
            .iter()
            .all(|scene| conditional_scene_to_wire(scene).is_ok()));
    }

    #[test]
    fn bluetooth_name_validates_the_advertising_limit() {
        let snapshot = minimal_runtime_config(Some("Glove80 {slot}"))
            .snapshot()
            .unwrap();
        assert_eq!(snapshot.bluetooth_name.as_deref(), Some("Glove80 {slot}"));
        let mut unsupported = snapshot.clone();
        unsupported.bluetooth_name = None;
        assert_eq!(differences(&snapshot, &unsupported).len(), 1);

        assert!(minimal_runtime_config(Some("")).snapshot().is_err());
        assert!(minimal_runtime_config(Some("12345678901234567"))
            .snapshot()
            .is_err());
    }

    #[test]
    fn existing_style_keymap_round_trips() {
        let keys = "\n-- -- KC_A KC_TRNS LT(1,KC_ESC) -- -- -- -- -- -- -- -- --\n-- -- -- -- -- -- -- -- -- -- -- -- -- --\n-- -- -- -- -- -- -- -- -- -- -- -- -- --\n-- -- -- -- -- -- -- -- -- -- -- -- -- --\n-- -- -- -- -- -- -- -- -- -- -- -- -- --\n-- -- -- -- -- -- -- -- -- -- -- -- -- --\n";
        let parsed = parse_keys(keys).unwrap();
        assert_eq!(parsed[2], 0x0004);
        assert_eq!(parsed[3], 0x0001);
        assert_eq!(parsed[4], 0x4129);
        assert_eq!(parse_keys(&render_keys(&parsed)).unwrap(), parsed);
    }

    #[test]
    fn go60_shape_and_pointing_policy_round_trip() {
        let text = r#"
rows = 5
default_layer = 0

[[layer]]
id = "base"
name = "Base"
keys = """
-- -- -- -- -- -- -- -- -- -- -- -- -- --
-- -- -- -- -- -- -- -- -- -- -- -- -- --
-- -- -- -- -- -- -- -- -- -- -- -- -- --
-- -- -- -- -- -- -- -- -- -- -- -- -- --
-- -- -- -- -- -- -- -- -- -- -- -- -- --
"""

[pointing]

[[pointing.device]]
device_id = 0
mode = "scroll"

[[pointing.device]]
device_id = 1
mode = "cursor"
"#;
        let config = RuntimeConfig::from_toml(text).unwrap();
        let snapshot = config.snapshot().unwrap();
        assert_eq!((snapshot.rows, snapshot.cols), (5, 14));
        assert_eq!(snapshot.layers[0].len(), 70);
        let pointing = snapshot.pointing.unwrap();
        assert!(matches!(
            pointing.devices()[0].mode,
            PointingMode::Scroll(_)
        ));
        assert!(matches!(
            pointing.devices()[1].mode,
            PointingMode::Cursor(_)
        ));

        let serialized = config.to_toml().unwrap();
        let rebuilt = RuntimeConfig::from_toml(&serialized)
            .unwrap()
            .snapshot()
            .unwrap();
        assert_eq!(rebuilt.rows, 5);
        assert_eq!(rebuilt.pointing.unwrap().devices(), pointing.devices());
    }

    #[test]
    fn parameterized_tap_holds_round_trip_without_via_loss() {
        let keys = "\n-- -- MT(A, LGui, hrm_pinky) LT(2, Escape, layer_hold) TH(B, LSFT, hrm_pinky) -- -- -- -- -- -- -- -- --\n-- -- -- -- -- -- -- -- -- -- -- -- -- --\n-- -- -- -- -- -- -- -- -- -- -- -- -- --\n-- -- -- -- -- -- -- -- -- -- -- -- -- --\n-- -- -- -- -- -- -- -- -- -- -- -- -- --\n-- -- -- -- -- -- -- -- -- -- -- -- -- --\n";
        let profiles = vec!["hrm_pinky".to_owned(), "layer_hold".to_owned()];
        let parsed = parse_key_actions(keys, &profiles).unwrap();
        assert!(matches!(
            parsed[2],
            KeyAction::TapHold(_, Action::Modifier(_), 0)
        ));
        assert!(matches!(
            parsed[3],
            KeyAction::TapHold(_, Action::LayerOn(2), 1)
        ));
        assert!(matches!(parsed[4], KeyAction::TapHold(_, _, 0)));
        let rendered = render_key_actions(&parsed, &profiles);
        assert_eq!(parse_key_actions(&rendered, &profiles).unwrap(), parsed);
    }

    #[test]
    fn combo_triggers_preserve_parameterized_tap_holds() {
        let profiles = vec!["autoshift".to_owned(), "thumb".to_owned()];
        let config = ComboConfig {
            name: "auto F11".to_owned(),
            keys: vec![
                "TH(KC_0, LSFT(KC_0), autoshift)".to_owned(),
                "LT(4, KC_BSPC, thumb)".to_owned(),
            ],
            positions: Vec::new(),
            output: "KC_F11".to_owned(),
            layer: Some(2),
        };

        let wire = config.to_wire(&profiles, ROWS, COLS).unwrap();
        let ComboDefinition::Actions(combo) = &wire else {
            panic!("key triggers became position triggers");
        };
        assert!(matches!(combo.actions[0], KeyAction::TapHold(_, _, 0)));
        assert!(matches!(combo.actions[1], KeyAction::TapHold(_, _, 1)));
        let rebuilt = ComboConfig::from_wire(&wire, 0, &profiles);
        assert_eq!(rebuilt.keys, config.keys);
    }

    #[test]
    fn position_combo_round_trips_without_becoming_action_triggers() {
        let config = ComboConfig {
            name: "home pair".to_owned(),
            keys: Vec::new(),
            positions: vec![[3, 1], [3, 10]],
            output: "KC_ESC".to_owned(),
            layer: Some(2),
        };

        let wire = config.to_wire(&[], ROWS, COLS).unwrap();
        let ComboDefinition::Positions(position) = &wire else {
            panic!("position triggers became action triggers");
        };
        assert_eq!(
            position.positions.as_slice(),
            [
                MatrixPosition { row: 3, col: 1 },
                MatrixPosition { row: 3, col: 10 },
            ]
        );
        let rebuilt = ComboConfig::from_wire(&wire, 0, &[]);
        assert_eq!(rebuilt.positions, config.positions);
        assert!(rebuilt.keys.is_empty());
        assert_eq!(rebuilt.output, config.output);
        assert_eq!(rebuilt.layer, config.layer);
    }

    #[test]
    fn export_trims_capacity_but_keeps_blocking_layers_and_interior_slots() {
        let config = minimal_runtime_config(None);
        let mut snapshot = config.snapshot().unwrap();
        let a = action_from_name("KC_A").unwrap();
        let b = action_from_name("KC_B").unwrap();
        let escape = action_from_name("KC_ESC").unwrap();
        let mut used_morse = Morse::default();
        let _ = used_morse.put(rynk::rmk_types::morse::TAP, a);
        snapshot.layers.push(vec![KeyAction::No; LAYER_SIZE]);
        snapshot
            .layers
            .push(vec![KeyAction::Transparent; LAYER_SIZE]);
        snapshot.behaviors.morses = Some(vec![Morse::default(), used_morse, Morse::default()]);
        snapshot.behaviors.combos = Some(vec![
            ComboDefinition::empty(),
            ComboDefinition::Actions(Combo {
                actions: [KeyAction::Single(a), KeyAction::Single(b)]
                    .into_iter()
                    .collect(),
                output: KeyAction::Single(escape),
                layer: None,
            }),
            ComboDefinition::empty(),
        ]);
        snapshot.behaviors.forks = Some(vec![
            rynk::rmk_types::fork::Fork::default(),
            rynk::rmk_types::fork::Fork {
                trigger: KeyAction::Single(a),
                positive_output: KeyAction::Single(b),
                ..Default::default()
            },
            rynk::rmk_types::fork::Fork::default(),
        ]);

        let exported = RuntimeConfig::from_snapshot(&snapshot, None);
        assert_eq!(exported.layers.len(), 2, "the all-No layer was trimmed");
        assert_eq!(exported.morses.len(), 2);
        assert_eq!(exported.combos.len(), 2);
        assert_eq!(exported.forks.len(), 2);
        RuntimeConfig::from_toml(&exported.to_toml().unwrap())
            .expect("a canonical export must validate");
    }

    #[test]
    fn hold_trigger_positions_round_trip_with_profile_names() {
        let mut config = minimal_runtime_config(None);
        let mut behavior = BehaviorConfig::default();
        behavior.morse.hold_trigger_key_positions = vec![[2, 8], [3, 9]];
        behavior.morse.profiles.insert(
            "hrm_left".to_owned(),
            MorseProfileConfig {
                hold_trigger_key_positions: vec![[2, 1], [3, 2]],
                ..MorseProfileConfig::default()
            },
        );
        config.behavior = Some(behavior);

        let snapshot = config.snapshot().unwrap();
        assert_eq!(
            snapshot.behaviors.hold_trigger_positions,
            Some(vec![
                HoldTriggerPosition {
                    profile: u8::MAX,
                    row: 2,
                    col: 8,
                },
                HoldTriggerPosition {
                    profile: u8::MAX,
                    row: 3,
                    col: 9,
                },
                HoldTriggerPosition {
                    profile: 0,
                    row: 2,
                    col: 1,
                },
                HoldTriggerPosition {
                    profile: 0,
                    row: 3,
                    col: 2,
                },
            ]),
        );

        let rebuilt = RuntimeConfig::from_snapshot(&snapshot, Some(&config));
        let morse = &rebuilt.behavior.unwrap().morse;
        assert_eq!(morse.hold_trigger_key_positions, vec![[2, 8], [3, 9]]);
        assert_eq!(
            morse.profiles["hrm_left"].hold_trigger_key_positions,
            vec![[2, 1], [3, 2]]
        );
    }

    #[test]
    fn named_sparse_profile_slots_survive_a_toml_round_trip() {
        let mut config = minimal_runtime_config(None);
        let mut behavior = BehaviorConfig::default();
        behavior.morse.profiles.insert(
            "alpha".to_owned(),
            MorseProfileConfig {
                index: Some(7),
                hold_timeout_ms: Some(170),
                opposite_hand_hold: Some(true),
                ..MorseProfileConfig::default()
            },
        );
        behavior.morse.profiles.insert(
            "zeta".to_owned(),
            MorseProfileConfig {
                index: Some(2),
                hold_timeout_ms: Some(230),
                ..MorseProfileConfig::default()
            },
        );
        config.behavior = Some(behavior);

        let snapshot = config.snapshot().unwrap();
        let entries = snapshot.behaviors.morse_profiles.as_deref().unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.index, entry.name.as_str()))
                .collect::<Vec<_>>(),
            vec![(2, "zeta"), (7, "alpha")]
        );

        let text = RuntimeConfig::from_snapshot(&snapshot, None)
            .to_toml()
            .unwrap();
        assert!(text.contains("[behavior.morse.profiles.alpha]"));
        assert!(text.contains("index = 7"));
        assert!(text.contains("opposite_hand_hold = true"));
        assert_eq!(
            RuntimeConfig::from_toml(&text).unwrap().snapshot().unwrap(),
            snapshot
        );
    }

    #[test]
    fn duplicate_profile_slots_are_rejected() {
        let mut config = minimal_runtime_config(None);
        let mut behavior = BehaviorConfig::default();
        for name in ["alpha", "zeta"] {
            behavior.morse.profiles.insert(
                name.to_owned(),
                MorseProfileConfig {
                    index: Some(3),
                    ..MorseProfileConfig::default()
                },
            );
        }
        config.behavior = Some(behavior);
        assert!(config
            .snapshot()
            .unwrap_err()
            .to_string()
            .contains("index 3 is assigned more than once"));
    }

    #[test]
    fn hand_policies_are_mutually_exclusive() {
        let mut config = minimal_runtime_config(None);
        let mut behavior = BehaviorConfig::default();
        behavior.morse.profiles.insert(
            "hrm".to_owned(),
            MorseProfileConfig {
                unilateral_tap: Some(true),
                opposite_hand_hold: Some(true),
                ..MorseProfileConfig::default()
            },
        );
        config.behavior = Some(behavior);

        assert!(config
            .snapshot()
            .unwrap_err()
            .to_string()
            .contains("unilateral_tap and opposite_hand_hold are mutually exclusive"));
    }

    #[test]
    fn scene_colors_are_canonicalized() {
        assert_eq!(normalize_color("C000C0").unwrap(), "#c000c0");
    }

    const LIGHTING_WITH_PARAMS: &str = r#"
brightness = 100
output_mode = "always-on"
scene_policy = "effective-only"

[background]
enabled = false
hue = 0
saturation = 0
value = 0
speed = 0
mode = "solid"

[effects]
effect = "Rain"
overlay = "Reactive"
palette = "Aurora"
value = 200
speed = 40

[effects.params.Rain]
Density = 6
"Trail Length" = 128
"#;

    fn params_of(config: &LightingConfig, effect: &str, name: &str) -> Option<u8> {
        config
            .effects
            .as_ref()?
            .params
            .get(effect)?
            .get(name)
            .copied()
    }

    fn param_set(effect: &str, params: &[(&str, u8, u8, u8, u8)]) -> EffectParams {
        EffectParams {
            index: 1,
            effect: effect.to_owned(),
            params: params
                .iter()
                .map(|(name, min, max, default, value)| ParamSpec {
                    name: (*name).to_owned(),
                    min: *min,
                    max: *max,
                    default: *default,
                    value: *value,
                })
                .collect(),
        }
    }

    /// The table's order is part of its meaning, so a reordering has to read as
    /// a difference even though the set of rules is identical.
    #[test]
    fn reordered_conditional_rules_are_a_difference() {
        let rule = |led: u16| ConditionalSceneConfig {
            connection: None,
            target: LightingTargetConfig::led(led),
            color: "#0040a0".into(),
            effect: EffectKind::Solid,
            period_ms: None,
            phase_ms: None,
            duty: None,
            step_ms: None,
            layer: Some(LayerConditionConfig {
                layer: 2,
                active: true,
            }),
            battery: None,
            output_mode: None,
            effects: None,
        };
        let snapshot = |cells: Vec<ConditionalSceneConfig>| {
            let mut snap = lighting_snapshot(None, None);
            snap.conditional_scenes = Some(cells);
            Snapshot {
                rows: ROWS,
                cols: COLS,
                bluetooth_name: None,
                behaviors: BehaviorSnapshot::default(),
                default_layer: 0,
                layers: Vec::new(),
                lighting: Some(snap),
                pointing: None,
            }
        };

        let forward = snapshot(vec![rule(10), rule(20)]);
        assert!(differences(&forward, &forward).is_empty());

        let reversed = snapshot(vec![rule(20), rule(10)]);
        let found = differences(&forward, &reversed);
        assert_eq!(found.len(), 2, "both positions differ: {found:?}");
        assert!(found.iter().all(|line| line.contains("conditional rule")));
    }

    /// Firmware without the runtime conditional commands reports `None`, which
    /// must stay distinct from an empty table: a file naming no rules is
    /// satisfied either way, but a file naming rules is not.
    #[test]
    fn unsupported_conditional_table_only_conflicts_when_rules_are_named() {
        let unsupported = {
            let mut snap = lighting_snapshot(None, None);
            snap.conditional_scenes = None;
            Snapshot {
                rows: ROWS,
                cols: COLS,
                bluetooth_name: None,
                behaviors: BehaviorSnapshot::default(),
                default_layer: 0,
                layers: Vec::new(),
                lighting: Some(snap),
                pointing: None,
            }
        };
        let empty_file = {
            let mut snap = lighting_snapshot(None, None);
            snap.conditional_scenes = Some(Vec::new());
            Snapshot {
                rows: ROWS,
                cols: COLS,
                bluetooth_name: None,
                behaviors: BehaviorSnapshot::default(),
                default_layer: 0,
                layers: Vec::new(),
                lighting: Some(snap),
                pointing: None,
            }
        };
        assert!(differences(&empty_file, &unsupported).is_empty());

        let with_rule = {
            let mut snap = lighting_snapshot(None, None);
            snap.conditional_scenes = Some(vec![ConditionalSceneConfig {
                connection: None,
                target: LightingTargetConfig::led(75),
                color: "#0040a0".into(),
                effect: EffectKind::Solid,
                period_ms: None,
                phase_ms: None,
                duty: None,
                step_ms: None,
                layer: None,
                battery: Some(BatteryConditionConfig {
                    node: 1,
                    min_level: Some(81),
                    max_level: None,
                    charge: ChargeConditionConfig::Charging,
                }),
                output_mode: None,
                effects: None,
            }]);
            Snapshot {
                rows: ROWS,
                cols: COLS,
                bluetooth_name: None,
                behaviors: BehaviorSnapshot::default(),
                default_layer: 0,
                layers: Vec::new(),
                lighting: Some(snap),
                pointing: None,
            }
        };
        let found = differences(&with_rule, &unsupported);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("no runtime conditional table"));
    }

    /// A table that shrank has to report its tail. Walking only as far as the
    /// file goes made a surplus slot invisible, which is how a keyboard kept
    /// firing combos that the file it was set from does not contain.
    #[test]
    fn slots_the_keyboard_holds_beyond_the_file_are_reported() {
        use rynk::rmk_types::action::KeyAction;
        use rynk::rmk_types::combo::Combo;

        let combo = |trigger: KeyAction| Combo {
            actions: Default::default(),
            output: trigger,
            layer: None,
        };
        let empty = ComboDefinition::empty();

        let mut desired = Snapshot {
            rows: ROWS,
            cols: COLS,
            bluetooth_name: None,
            default_layer: 0,
            layers: Vec::new(),
            lighting: None,
            behaviors: BehaviorSnapshot {
                // The file claims the table and names nothing in it.
                combos: Some(Vec::new()),
                ..BehaviorSnapshot::default()
            },
            pointing: None,
        };
        let live = Snapshot {
            rows: ROWS,
            cols: COLS,
            bluetooth_name: None,
            default_layer: 0,
            layers: Vec::new(),
            lighting: None,
            behaviors: BehaviorSnapshot {
                combos: Some(vec![
                    ComboDefinition::Actions(combo(KeyAction::Single(Action::LayerToggle(2)))),
                    empty.clone(),
                ]),
                ..BehaviorSnapshot::default()
            },
            pointing: None,
        };
        let report = differences(&desired, &live);
        assert_eq!(
            report,
            vec!["combo 0: on the keyboard but not in the file".to_string()],
            "only the populated surplus slot should be reported"
        );

        // Silence is still silence: a file that does not claim the table at all
        // leaves it alone, which is what keeps an older configuration safe.
        desired.behaviors.combos = None;
        assert!(differences(&desired, &live).is_empty());
    }

    #[test]
    fn conditional_rules_round_trip_through_the_wire_and_reject_bad_batteries() {
        let mut cell = ConditionalSceneConfig {
            connection: None,
            target: LightingTargetConfig::led(75),
            color: "#0040a0".into(),
            effect: EffectKind::Solid,
            period_ms: None,
            phase_ms: None,
            duty: None,
            step_ms: None,
            layer: Some(LayerConditionConfig {
                layer: 3,
                active: false,
            }),
            battery: Some(BatteryConditionConfig {
                node: 1,
                min_level: Some(20),
                max_level: Some(80),
                charge: ChargeConditionConfig::Discharging,
            }),
            output_mode: None,
            effects: None,
        };
        let wire = conditional_scene_to_wire(&cell).unwrap();
        assert_eq!(conditional_scene_from_wire(wire), cell);
        assert!(validate_conditional_scene(0, &cell).is_ok());

        cell.connection = Some(ConnectionConditionConfig {
            transport: Some(TransportConfig::Ble),
            profile: Some(2),
            ble_state: Some(BleStateConfig::Connected),
            bonded: None,
            usb_connected: None,
        });
        let wire = conditional_scene_to_wire(&cell).unwrap();
        assert_eq!(conditional_scene_from_wire(wire), cell);
        assert!(validate_conditional_scene(0, &cell).is_ok());

        cell.connection = Some(ConnectionConditionConfig {
            transport: None,
            profile: Some(4),
            ble_state: None,
            bonded: None,
            usb_connected: None,
        });
        assert!(validate_conditional_scene(0, &cell).is_err());

        cell.connection = Some(ConnectionConditionConfig {
            transport: None,
            profile: None,
            ble_state: None,
            bonded: None,
            usb_connected: None,
        });
        assert!(validate_conditional_scene(0, &cell).is_err());
        cell.connection = None;

        // The firmware would decline these on apply; catching them offline
        // means `config validate` is enough to know a file is writable.
        cell.battery = Some(BatteryConditionConfig {
            node: 1,
            min_level: Some(90),
            max_level: Some(10),
            charge: ChargeConditionConfig::Any,
        });
        assert!(validate_conditional_scene(0, &cell).is_err());

        cell.battery = Some(BatteryConditionConfig {
            node: 1,
            min_level: Some(120),
            max_level: None,
            charge: ChargeConditionConfig::Any,
        });
        assert!(validate_conditional_scene(0, &cell).is_err());
    }

    fn lighting_snapshot(
        effects: Option<EffectsConfig>,
        params: Option<Vec<EffectParams>>,
    ) -> LightingSnapshot {
        LightingSnapshot {
            brightness: 100,
            output_mode: OutputModeConfig::AlwaysOn,
            wake_layers: Vec::new(),
            scene_policy: ScenePolicyConfig::EffectiveOnly,
            background: BackgroundConfig {
                enabled: false,
                hue: 0,
                saturation: 0,
                value: 0,
                speed: 0,
                mode: BackgroundModeConfig::Solid,
            },
            effects,
            params,
            scenes: Vec::new(),
            conditional_scenes: Some(Vec::new()),
        }
    }

    fn effects_with(params: &[(&str, u8)]) -> EffectsConfig {
        EffectsConfig {
            effect: "Rain".into(),
            overlay: None,
            palette: "Aurora".into(),
            value: 200,
            speed: 40,
            params: BTreeMap::from([(
                "Rain".to_owned(),
                params
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), *value))
                    .collect(),
            )]),
        }
    }

    #[test]
    fn effect_param_tables_round_trip() {
        let config: LightingConfig = toml::from_str(LIGHTING_WITH_PARAMS).unwrap();
        assert_eq!(params_of(&config, "Rain", "Density"), Some(6));
        assert_eq!(params_of(&config, "Rain", "Trail Length"), Some(128));
        assert_eq!(
            config
                .effects
                .as_ref()
                .and_then(|effects| effects.overlay.as_deref()),
            Some("Reactive")
        );

        let text = toml::to_string_pretty(&config).unwrap();
        assert!(text.contains("[effects.params.Rain]"), "{text}");
        let reparsed: LightingConfig = toml::from_str(&text).unwrap();
        assert_eq!(reparsed.effects, config.effects);
    }

    #[test]
    fn effect_params_are_optional_and_omitted_when_empty() {
        let config: LightingConfig = toml::from_str(&LIGHTING_WITH_PARAMS.replace(
            "[effects.params.Rain]\nDensity = 6\n\"Trail Length\" = 128\n",
            "",
        ))
        .unwrap();
        assert!(config.effects.as_ref().unwrap().params.is_empty());
        assert!(!toml::to_string_pretty(&config).unwrap().contains("params"));
    }

    #[test]
    fn effect_param_names_must_not_be_empty() {
        let text = LIGHTING_WITH_PARAMS.replace("[effects.params.Rain]", "[effects.params.\"\"]");
        let error = toml::from_str::<LightingConfig>(&text)
            .unwrap()
            .snapshot()
            .unwrap_err();
        assert!(error.to_string().contains("empty effect name"), "{error}");

        let text = LIGHTING_WITH_PARAMS.replace("Density = 6", "\"\" = 6");
        let error = toml::from_str::<LightingConfig>(&text)
            .unwrap()
            .snapshot()
            .unwrap_err();
        assert!(
            error.to_string().contains("empty parameter name"),
            "{error}"
        );
    }

    #[test]
    fn effect_param_values_are_bytes() {
        let text = LIGHTING_WITH_PARAMS.replace("Density = 6", "Density = 256");
        assert!(toml::from_str::<LightingConfig>(&text).is_err());
    }

    #[test]
    fn pull_records_only_parameters_that_differ_from_their_default() {
        let snapshot = Snapshot {
            rows: ROWS,
            cols: COLS,
            bluetooth_name: None,
            behaviors: BehaviorSnapshot::default(),
            default_layer: 0,
            layers: vec![vec![KeyAction::No; LAYER_SIZE]],
            lighting: Some(lighting_snapshot(
                Some(effects_with(&[("Density", 6), ("Trail Length", 128)])),
                Some(vec![param_set(
                    "Rain",
                    &[("Density", 0, 16, 4, 6), ("Trail Length", 0, 255, 128, 128)],
                )]),
            )),
            pointing: None,
        };
        let mut config = RuntimeConfig::from_snapshot(&snapshot, None);
        config.retain_non_default_params(&snapshot);
        let lighting = config.lighting.unwrap();
        assert_eq!(params_of(&lighting, "Rain", "Density"), Some(6));
        assert_eq!(params_of(&lighting, "Rain", "Trail Length"), None);
    }

    #[test]
    fn pull_drops_parameters_when_the_keyboard_has_none() {
        let snapshot = Snapshot {
            rows: ROWS,
            cols: COLS,
            bluetooth_name: None,
            behaviors: BehaviorSnapshot::default(),
            default_layer: 0,
            layers: vec![vec![KeyAction::No; LAYER_SIZE]],
            lighting: Some(lighting_snapshot(
                Some(effects_with(&[("Density", 6)])),
                None,
            )),
            pointing: None,
        };
        let mut config = RuntimeConfig::from_snapshot(&snapshot, None);
        config.retain_non_default_params(&snapshot);
        assert!(config.lighting.unwrap().effects.unwrap().params.is_empty());
    }

    #[test]
    fn parameter_differences_only_cover_what_the_file_names() {
        let desired = lighting_snapshot(Some(effects_with(&[("Density", 6)])), None);
        let live = lighting_snapshot(
            Some(effects_with(&[("Density", 4), ("Trail Length", 200)])),
            Some(vec![param_set(
                "Rain",
                &[("Density", 0, 16, 4, 4), ("Trail Length", 0, 255, 128, 200)],
            )]),
        );
        assert_eq!(
            param_differences(&desired, &live),
            vec!["lighting parameter Rain.Density: file 6 != keyboard 4"]
        );

        let matching = lighting_snapshot(Some(effects_with(&[("Density", 4)])), None);
        assert!(param_differences(&matching, &live).is_empty());
    }

    #[test]
    fn unadvertised_parameters_are_reported_as_differences() {
        let desired = lighting_snapshot(Some(effects_with(&[("Sparkle", 3)])), None);
        let live = lighting_snapshot(Some(effects_with(&[])), None);
        assert_eq!(
            param_differences(&desired, &live),
            vec!["lighting parameter Rain.Sparkle: file 3 != keyboard (not advertised)"]
        );
    }

    #[test]
    fn parameter_tables_do_not_disturb_the_extension_selection_diff() {
        let desired = lighting_snapshot(Some(effects_with(&[("Density", 6)])), None);
        let live = lighting_snapshot(Some(effects_with(&[])), None);
        assert_eq!(
            desired.effects.as_ref().map(EffectsConfig::selection),
            live.effects.as_ref().map(EffectsConfig::selection)
        );
    }
}
