#![no_main]
#![no_std]

#[allow(dead_code)]
mod trackpad;

use moergo_rmk::lighting;
use rmk::macros::rmk_peripheral;

#[rmk_peripheral(id = 0)]
mod keyboard_peripheral {
    #[register_processor(runnable)]
    fn lighting_processor() {
        crate::lighting::init_peripheral(p.SPI3, p.P0_27, p.P1_11)
    }

    #[register_processor(runnable)]
    fn trackpad_device() {
        crate::trackpad::init(
            crate::trackpad::RIGHT_DEVICE_ID,
            p.TWISPI1,
            p.P0_19,
            p.P0_21,
            p.P0_22,
            p.P0_25,
            p.P0_23,
        )
    }

    #[register_processor(runnable)]
    fn lighting_replication() {
        crate::lighting::peripheral_replication()
    }

    #[register_processor(runnable)]
    fn lighting_replication_worker() {
        crate::lighting::peripheral_lighting_worker()
    }

    #[register_processor(runnable)]
    fn lighting_power_monitor() {
        crate::lighting::power_monitor(p.PWM0, p.P1_15)
    }

    #[register_processor(event)]
    fn reactive_key_hits() {
        crate::lighting::ReactiveKeyHits::peripheral()
    }
}
