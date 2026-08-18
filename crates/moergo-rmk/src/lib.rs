#![no_std]

#[cfg(all(feature = "glove80", feature = "go60"))]
compile_error!("the `glove80` and `go60` features are mutually exclusive");
#[cfg(not(any(feature = "glove80", feature = "go60")))]
compile_error!("enable one of the `glove80` or `go60` features");

#[cfg(feature = "glove80")]
pub const BOARD_LEDS_PER_HALF: usize = 40;
#[cfg(feature = "go60")]
pub const BOARD_LEDS_PER_HALF: usize = 30;

#[cfg(feature = "glove80")]
pub const BOARD_SCENE_CAPACITY: usize = 100;
#[cfg(feature = "go60")]
pub const BOARD_SCENE_CAPACITY: usize = 80;

#[cfg(feature = "glove80")]
pub const BOARD_CHANNEL_CEILING: u8 = 230;
#[cfg(feature = "go60")]
pub const BOARD_CHANNEL_CEILING: u8 = 102;

#[cfg(feature = "glove80")]
pub const BOARD_KEEP_LED_POWER_WHILE_AWAKE: bool = false;
// GO60's canonical ZMK firmware treats WS2812_CE as an underglow-state rail,
// not a per-frame visibility optimization.
#[cfg(feature = "go60")]
pub const BOARD_KEEP_LED_POWER_WHILE_AWAKE: bool = true;

#[cfg(feature = "glove80")]
pub const BOARD_KEEP_LED_POWER_WHILE_SUSPENDED: bool = false;
// Canonical GO60 ZMK compiles without RGB_UNDERGLOW_AUTO_OFF_IDLE, so
// WS2812_CE stays asserted through idle/sleep. A dropped rail leaves this
// chain at sub-threshold voltage and randomly lit, so suspend latches a dark
// frame instead of cutting power.
#[cfg(feature = "go60")]
pub const BOARD_KEEP_LED_POWER_WHILE_SUSPENDED: bool = true;

#[cfg(feature = "glove80")]
pub const BOARD_MAINTENANCE_LED: u16 = 12;
#[cfg(feature = "go60")]
pub const BOARD_MAINTENANCE_LED: u16 = 8;

pub mod central_lighting;
pub mod lighting;
pub mod remote_boot;
pub mod split_lighting;

pub use lighting::topology_config::{
    LIGHTING_BACKGROUND, LIGHTING_CONDITIONAL_SCENE_CELLS, LIGHTING_CONTROLS,
    LIGHTING_LAYER_SCENES, LIGHTING_ROUTING, LIGHTING_TOPOLOGY, LIGHTING_TOPOLOGY_REVISION,
};
