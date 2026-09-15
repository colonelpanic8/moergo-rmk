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
pub struct MagicKeyActions {
    ble_clear: BleClearChord,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BleClearAction {
    Active,
    Profile(u8),
}

#[derive(Default)]
struct BleClearChord {
    held: bool,
    used: bool,
}

impl BleClearChord {
    fn update(&mut self, pressed: bool, action: Action) -> Option<BleClearAction> {
        match (pressed, action) {
            (true, Action::User(CLEAR_ACTIVE_BLE_PROFILE_ACTION)) => {
                self.held = true;
                self.used = false;
                None
            }
            (true, Action::User(profile)) if self.held && profile < rmk::ble::profile_count() => {
                self.used = true;
                Some(BleClearAction::Profile(profile))
            }
            (false, Action::User(CLEAR_ACTIVE_BLE_PROFILE_ACTION)) => {
                self.held = false;
                (!core::mem::take(&mut self.used)).then_some(BleClearAction::Active)
            }
            _ => None,
        }
    }
}

impl MagicKeyActions {
    pub const fn new() -> Self {
        Self {
            ble_clear: BleClearChord {
                held: false,
                used: false,
            },
        }
    }

    async fn on_action_event(&mut self, event: ActionEvent) {
        match self
            .ble_clear
            .update(event.keyboard_event.pressed, event.action)
        {
            Some(BleClearAction::Active) => rmk::ble::clear_active_profile().await,
            Some(BleClearAction::Profile(profile)) => {
                let _ = rmk::ble::clear_profile(profile).await;
            }
            None => {}
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_key_alone_clears_the_active_profile_on_release() {
        let mut chord = BleClearChord::default();
        assert_eq!(
            chord.update(true, Action::User(CLEAR_ACTIVE_BLE_PROFILE_ACTION)),
            None
        );
        assert_eq!(
            chord.update(false, Action::User(CLEAR_ACTIVE_BLE_PROFILE_ACTION)),
            Some(BleClearAction::Active)
        );
    }

    #[test]
    fn clear_key_plus_profile_clears_only_that_profile() {
        let mut chord = BleClearChord::default();
        chord.update(true, Action::User(CLEAR_ACTIVE_BLE_PROFILE_ACTION));
        assert_eq!(
            chord.update(true, Action::User(2)),
            Some(BleClearAction::Profile(2))
        );
        assert_eq!(chord.update(false, Action::User(2)), None);
        assert_eq!(
            chord.update(false, Action::User(CLEAR_ACTIVE_BLE_PROFILE_ACTION)),
            None
        );
    }

    #[test]
    fn non_profile_user_actions_do_not_consume_the_clear_key() {
        let mut chord = BleClearChord::default();
        chord.update(true, Action::User(CLEAR_ACTIVE_BLE_PROFILE_ACTION));
        assert_eq!(
            chord.update(true, Action::User(CLEAR_ALL_BLE_PROFILES_ACTION)),
            None
        );
        assert_eq!(
            chord.update(false, Action::User(CLEAR_ACTIVE_BLE_PROFILE_ACTION)),
            Some(BleClearAction::Active)
        );
    }
}
