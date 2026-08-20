//! Read RMK's standard BLE Battery Service instances.

use std::time::Duration;

use anyhow::{anyhow, bail, ensure, Context, Result};
use bluest::{Adapter, Characteristic, Device, Uuid};
use rynk::rmk_types::battery::{BatteryStatus, ChargeState};
use serde::Serialize;

use crate::transport::{Preference, Selector};

const RYNK_SERVICE_UUID: Uuid = Uuid::from_u128(rynk::rmk_types::protocol::rynk::RYNK_SERVICE_UUID);
const BATTERY_SERVICE_UUID: Uuid = Uuid::from_u128(0x0000180f_0000_1000_8000_00805f9b34fb);
const BATTERY_LEVEL_UUID: Uuid = Uuid::from_u128(0x00002a19_0000_1000_8000_00805f9b34fb);
const USER_DESCRIPTION_UUID: Uuid = Uuid::from_u128(0x00002901_0000_1000_8000_00805f9b34fb);
const PRESENTATION_FORMAT_UUID: Uuid = Uuid::from_u128(0x00002904_0000_1000_8000_00805f9b34fb);
const MAIN_BATTERY_DESCRIPTION: u16 = 0x0106;
const GATT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct BatteryReading {
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) level: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) charge_state: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) connected: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct BatteryReport {
    pub(crate) transport: &'static str,
    pub(crate) device: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    pub(crate) batteries: Vec<BatteryReading>,
}

struct OrderedReading {
    order: u16,
    reading: BatteryReading,
}

pub fn run(selector: &Selector, json: bool) -> Result<()> {
    match selector.preference {
        Preference::Usb => return crate::rynk_client::run_battery(selector, json),
        Preference::Ble => {}
        Preference::Auto
            if !selector
                .device
                .as_deref()
                .is_some_and(crate::transport::is_ble_address) =>
        {
            return crate::rynk_client::run_battery(selector, json);
        }
        Preference::Auto => {}
    }

    let runtime =
        tokio::runtime::Runtime::new().context("could not create the BLE async runtime")?;
    runtime.block_on(async {
        tokio::time::timeout(GATT_TIMEOUT, query(selector.device.as_deref(), json))
            .await
            .map_err(|_| anyhow!("timed out reading BLE battery services"))?
    })
}

async fn query(requested: Option<&str>, json: bool) -> Result<()> {
    let adapter = Adapter::default()
        .await
        .ok_or_else(|| anyhow!("no BLE adapter found"))?;
    adapter
        .wait_available()
        .await
        .context("BLE adapter is unavailable")?;
    let device = select_device(
        adapter
            .connected_devices_with_services(&[RYNK_SERVICE_UUID])
            .await
            .context("Rynk BLE discovery failed")?,
        requested,
    )?;
    let device_id = device.id().to_string();
    let device_name = device.name_async().await.ok();
    let readings = read_batteries(&device).await?;
    let report = BatteryReport {
        transport: "ble-gatt",
        device: device_id,
        name: device_name,
        batteries: readings,
    };
    emit(&report, json)
}

fn select_device(mut devices: Vec<Device>, requested: Option<&str>) -> Result<Device> {
    if let Some(address) = requested {
        let needle = normalized_device_id(address);
        return devices
            .into_iter()
            .find(|device| normalized_device_id(&device.id().to_string()) == needle)
            .ok_or_else(|| anyhow!("no connected Rynk BLE device matches {address}"));
    }
    match devices.len() {
        0 => bail!("no connected Rynk BLE device found; connect the keyboard over BLE first"),
        1 => Ok(devices.pop().expect("length checked")),
        count => bail!("found {count} connected Rynk BLE devices; pass --device to select one"),
    }
}

fn normalized_device_id(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .flat_map(char::to_lowercase)
        .collect()
}

async fn read_batteries(device: &Device) -> Result<Vec<BatteryReading>> {
    let services = device
        .discover_services_with_uuid(BATTERY_SERVICE_UUID)
        .await
        .context("could not discover BLE Battery Services")?;
    if services.is_empty() {
        bail!("connected Rynk device does not expose a BLE Battery Service");
    }

    let mut readings = Vec::with_capacity(services.len());
    for service in services {
        let characteristic = service
            .discover_characteristics_with_uuid(BATTERY_LEVEL_UUID)
            .await
            .context("could not discover a Battery Level characteristic")?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("Battery Service has no Battery Level characteristic"))?;
        readings.push(read_characteristic(characteristic).await?);
    }
    readings.sort_by_key(|reading| reading.order);
    Ok(readings
        .into_iter()
        .map(|reading| reading.reading)
        .collect())
}

async fn read_characteristic(characteristic: Characteristic) -> Result<OrderedReading> {
    let value = characteristic
        .read()
        .await
        .context("could not read a Battery Level characteristic")?;
    let level = decode_level(&value)?;
    let mut user_description = None;
    let mut presentation_description = None;
    for descriptor in characteristic
        .discover_descriptors()
        .await
        .context("could not discover Battery Level descriptors")?
    {
        let Ok(uuid) = descriptor.uuid_async().await else {
            continue;
        };
        if uuid == USER_DESCRIPTION_UUID {
            if let Ok(value) = descriptor.read().await {
                user_description = decode_user_description(&value);
            }
        } else if uuid == PRESENTATION_FORMAT_UUID {
            if let Ok(value) = descriptor.read().await {
                presentation_description = decode_presentation_description(&value);
            }
        }
    }

    let (fallback, order) = presentation_identity(presentation_description);
    Ok(OrderedReading {
        order,
        reading: BatteryReading {
            name: user_description.unwrap_or(fallback),
            level: Some(level),
            charge_state: None,
            connected: None,
        },
    })
}

impl BatteryReading {
    pub(crate) fn from_status(
        name: String,
        status: BatteryStatus,
        connected: Option<bool>,
    ) -> Self {
        let (level, charge_state) = match status {
            BatteryStatus::Unavailable => (None, None),
            BatteryStatus::Available {
                charge_state,
                level,
            } => (level, Some(render_charge_state(charge_state))),
        };
        Self {
            name,
            level,
            charge_state,
            connected,
        }
    }
}

fn render_charge_state(state: ChargeState) -> &'static str {
    match state {
        ChargeState::Charging => "charging",
        ChargeState::Discharging => "discharging",
        ChargeState::Unknown => "unknown",
    }
}

fn decode_level(value: &[u8]) -> Result<u8> {
    ensure!(
        value.len() == 1,
        "Battery Level must contain exactly one byte"
    );
    ensure!(
        value[0] <= 100,
        "Battery Level {} is outside 0..=100",
        value[0]
    );
    Ok(value[0])
}

fn decode_user_description(value: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(value)
        .ok()?
        .trim_end_matches('\0')
        .trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn decode_presentation_description(value: &[u8]) -> Option<u16> {
    (value.len() == 7).then(|| u16::from_le_bytes([value[5], value[6]]))
}

fn presentation_identity(description: Option<u16>) -> (String, u16) {
    match description {
        Some(MAIN_BATTERY_DESCRIPTION) => ("Central".into(), 0),
        Some(id @ 1..) => (format!("Peripheral {}", id - 1), id),
        _ => ("Battery".into(), u16::MAX),
    }
}

pub(crate) fn render(readings: &[BatteryReading]) -> String {
    readings
        .iter()
        .map(|reading| {
            let value = match reading.level {
                Some(level) => format!("{level}%"),
                None => "unavailable".into(),
            };
            let charge = reading
                .charge_state
                .map(|state| format!(" ({state})"))
                .unwrap_or_default();
            let connection = match reading.connected {
                Some(false) => " (disconnected)",
                _ => "",
            };
            format!("{}: {value}{charge}{connection}\n", reading.name)
        })
        .collect()
}

pub(crate) fn emit(report: &BatteryReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
    } else {
        print!("{}", render(&report.batteries));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_rmk_battery_descriptors() {
        assert_eq!(decode_user_description(b"Right\0"), Some("Right".into()));
        assert_eq!(decode_user_description(b"  "), None);
        assert_eq!(
            decode_presentation_description(&[0x04, 0x00, 0xad, 0x27, 0x01, 0x06, 0x01]),
            Some(MAIN_BATTERY_DESCRIPTION)
        );
        assert_eq!(
            presentation_identity(Some(MAIN_BATTERY_DESCRIPTION)),
            ("Central".into(), 0)
        );
        assert_eq!(presentation_identity(Some(1)), ("Peripheral 0".into(), 1));
    }

    #[test]
    fn rejects_malformed_battery_levels() {
        assert!(decode_level(&[]).is_err());
        assert!(decode_level(&[50, 60]).is_err());
        assert!(decode_level(&[101]).is_err());
        assert_eq!(decode_level(&[73]).unwrap(), 73);
    }

    #[test]
    fn renders_each_half_on_its_own_line() {
        assert_eq!(
            render(&[
                BatteryReading {
                    name: "Central".into(),
                    level: Some(73),
                    charge_state: Some("discharging"),
                    connected: None,
                },
                BatteryReading {
                    name: "Peripheral 0".into(),
                    level: None,
                    charge_state: None,
                    connected: Some(false),
                },
            ]),
            "Central: 73% (discharging)\nPeripheral 0: unavailable (disconnected)\n"
        );
    }

    #[test]
    fn converts_rynk_battery_status() {
        assert_eq!(
            BatteryReading::from_status(
                "Central".into(),
                BatteryStatus::Available {
                    charge_state: ChargeState::Charging,
                    level: Some(91),
                },
                None,
            ),
            BatteryReading {
                name: "Central".into(),
                level: Some(91),
                charge_state: Some("charging"),
                connected: None,
            }
        );
    }

    #[test]
    fn normalizes_linux_ble_addresses() {
        assert_eq!(normalized_device_id("AA:BB:CC:DD:EE:FF"), "aabbccddeeff");
        assert_eq!(normalized_device_id("aa-bb-cc-dd-ee-ff"), "aabbccddeeff");
    }
}
