//! Magic-layer system controls and physical right-half bootloader routing.
//!
//! Split key actions are resolved on the central, so binding the right-half
//! key directly to RMK's `Bootloader` action would reboot the left half. This
//! processor also maps board-reserved user actions onto the standard lighting
//! action path, keeping the runtime keymap's 16-bit representation sufficient.

use rmk::event::ActionEvent;
use rmk::types::action::{Action, LightAction};

/// User action reserved for clearing the active BLE host profile.
pub const CLEAR_ACTIVE_BLE_PROFILE_ACTION: u8 = 10;
/// User action reserved for clearing every BLE host profile.
pub const CLEAR_ALL_BLE_PROFILES_ACTION: u8 = 11;
/// User action reserved for the right-half physical bootloader key.
pub const PERIPHERAL_BOOTLOADER_ACTION: u8 = 12;
/// User action reserved for the Magic-layer split-transport toggle:
/// force-BLE ↔ auto. Forced-wired stays host-only — with no cable present
/// it would strand the halves mid-toggle.
pub const SPLIT_TRANSPORT_TOGGLE_ACTION: u8 = 13;
#[rmk::macros::processor(subscribe = [ActionEvent])]
pub struct MagicKeyActions;

impl MagicKeyActions {
    async fn on_action_event(&mut self, event: ActionEvent) {
        match (event.keyboard_event.pressed, event.action) {
            (false, Action::User(action))
                if crate::LIGHTING_CONTROLS.output_toggle_user_action == Some(action) =>
            {
                rmk::lighting::send_light_action(LightAction::BacklightToggle).await;
            }
            (false, Action::User(action))
                if crate::LIGHTING_CONTROLS.output_mode_cycle_user_action == Some(action) =>
            {
                rmk::lighting::send_light_action(LightAction::OutputModeCycle).await;
            }
            (false, Action::User(CLEAR_ACTIVE_BLE_PROFILE_ACTION)) => {
                rmk::ble::clear_active_profile().await;
            }
            (false, Action::User(CLEAR_ALL_BLE_PROFILES_ACTION)) => {
                rmk::ble::clear_all_profiles().await;
            }
            (false, Action::User(PERIPHERAL_BOOTLOADER_ACTION)) => {
                // A second release while one request is pending is equivalent
                // to the first. Never block the keyboard task on split traffic.
                let _ = crate::central_lighting::REMOTE_BOOT_REQUESTS.try_send(());
            }
            (false, Action::User(SPLIT_TRANSPORT_TOGGLE_ACTION)) => {
                use rmk::split::selector;
                if selector::auto_enabled() {
                    let mode = if selector::forced_mode() == selector::FORCE_BLE {
                        selector::FORCE_AUTO
                    } else {
                        selector::FORCE_BLE
                    };
                    let _ = rmk::split::request_transport_force(mode);
                }
            }
            _ => {}
        }
    }
}
