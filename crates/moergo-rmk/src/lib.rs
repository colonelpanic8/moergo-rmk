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
pub const BOARD_CHANNEL_CEILING: u8 = 230;
#[cfg(feature = "go60")]
pub const BOARD_CHANNEL_CEILING: u8 = 102;

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
