//! `connection` verb: read and drive the keyboard's transports and BLE slots.
//!
//! Everything here rides the management protocol, so it works regardless of
//! which transport currently carries typing — including switching the typing
//! target itself.

use anyhow::Result;
use clap::{Subcommand, ValueEnum};
use rynk::rmk_types::ble::BleState;
use rynk::rmk_types::connection::{ConnectionStatus, ConnectionType, UsbState};
use rynk::rmk_types::protocol::rynk::{SplitTransportForce, SplitTransportState};

use crate::transport::Selector;

#[derive(Subcommand)]
pub enum ConnectionCommand {
    /// Show the transports, the selected BLE slot, and where typing goes.
    Status,
    /// Select a BLE slot; an empty slot starts advertising for pairing.
    Switch { slot: u8 },
    /// Forget the bond stored in a BLE slot.
    Clear { slot: u8 },
    /// Read or replace the BLE advertising-name template.
    Name {
        #[command(subcommand)]
        command: NameCommand,
    },
    /// Show the wired/BLE transport between the halves, or force one.
    Split {
        /// Force the split transport (volatile until the next force or
        /// reboot); `auto` returns control to cable detect.
        #[arg(long, value_enum)]
        force: Option<SplitForceMode>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum SplitForceMode {
    Auto,
    Wired,
    Ble,
}

impl From<SplitForceMode> for SplitTransportForce {
    fn from(mode: SplitForceMode) -> Self {
        match mode {
            SplitForceMode::Auto => SplitTransportForce::Auto,
            SplitForceMode::Wired => SplitTransportForce::Wired,
            SplitForceMode::Ble => SplitTransportForce::Ble,
        }
    }
}

#[derive(Subcommand)]
pub enum NameCommand {
    /// Show the persistent name template.
    Get,
    /// Replace the template; `{slot}` expands to the one-based active slot.
    Set { template: String },
}

pub fn run(selector: &Selector, command: &ConnectionCommand) -> Result<()> {
    crate::rynk_client::run_connection(selector, command)
}

pub(crate) fn render_split(state: &SplitTransportState) -> String {
    if !state.auto {
        return "split transport: fixed (this board has no automatic wired/BLE policy)\n".into();
    }
    let force = match state.forced {
        SplitTransportForce::Auto => "auto (following cable detect)",
        SplitTransportForce::Wired => "wired",
        SplitTransportForce::Ble => "ble",
    };
    format!(
        "cable detected: {}\n\
         force: {force}\n\
         split transport: {}\n",
        if state.cable_detected { "yes" } else { "no" },
        if state.wired_active { "wired" } else { "ble" },
    )
}

pub(crate) fn render(status: &ConnectionStatus) -> String {
    let usb = match status.usb {
        UsbState::Disabled => "disabled",
        UsbState::Enabled => "enabled (not configured)",
        UsbState::Configured => "connected",
        UsbState::Suspended => "suspended",
    };
    let ble = match status.ble.state {
        BleState::Advertising => "advertising (pairable)",
        BleState::Connected => "connected",
        BleState::Inactive => "inactive",
    };
    let preferred = match status.preferred {
        ConnectionType::Usb => "usb",
        ConnectionType::Ble => "ble",
    };
    let active = match status.decide_active() {
        Some(ConnectionType::Usb) => "usb",
        Some(ConnectionType::Ble) => "ble",
        None => "none",
    };
    format!(
        "usb: {usb}\n\
         ble: slot {} {ble}\n\
         preferred: {preferred}\n\
         typing goes to: {active}\n",
        status.ble.profile,
    )
}
