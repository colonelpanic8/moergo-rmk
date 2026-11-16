use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use bluest::{Adapter, DeviceId, Uuid};

pub fn usb_serial(path: &Path) -> Result<String> {
    let node = path.file_name().context("HID path has no filename")?;
    let uevent = std::fs::read_to_string(
        Path::new("/sys/class/hidraw")
            .join(node)
            .join("device/uevent"),
    )
    .with_context(|| format!("could not read hardware identity for {}", path.display()))?;
    serial_from_uevent(&uevent)
}

fn serial_from_uevent(uevent: &str) -> Result<String> {
    uevent
        .lines()
        .find_map(|line| line.strip_prefix("HID_UNIQ="))
        .filter(|serial| !serial.is_empty())
        .map(str::to_owned)
        .context("USB device has no hardware serial; refusing to identify it by name or path")
}

pub async fn ble_serial(id: DeviceId) -> Result<String> {
    tokio::time::timeout(Duration::from_secs(10), read_ble_serial(id))
        .await
        .context("timed out reading the BLE hardware serial")?
}

async fn read_ble_serial(id: DeviceId) -> Result<String> {
    let adapter = Adapter::default().await.context("no BLE adapter")?;
    let device = adapter
        .connected_devices()
        .await?
        .into_iter()
        .find(|device| device.id() == id)
        .ok_or_else(|| anyhow!("selected BLE device disconnected before reading its serial"))?;
    let services = device
        .discover_services_with_uuid(Uuid::from_u128(0x0000180a_0000_1000_8000_00805f9b34fb))
        .await?;
    let mut serials = Vec::new();
    for service in services {
        for characteristic in service
            .discover_characteristics_with_uuid(Uuid::from_u128(
                0x00002a25_0000_1000_8000_00805f9b34fb,
            ))
            .await?
        {
            serials.push(String::from_utf8(characteristic.read().await?)?);
        }
    }
    if serials.len() != 1 || serials[0].is_empty() {
        bail!("expected one nonempty BLE Device Information serial, found {serials:?}");
    }
    Ok(serials.remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_identity_uses_the_serial_not_the_product_or_port() {
        assert_eq!(
            serial_from_uevent(
                "HID_NAME=MoErgo Glove80\nHID_PHYS=usb-1/input2\nHID_UNIQ=rynk:bcbd6d\n"
            )
            .unwrap(),
            "rynk:bcbd6d"
        );
        assert!(serial_from_uevent("HID_NAME=WhiteIG80\nHID_UNIQ=\n").is_err());
        assert!(serial_from_uevent("HID_NAME=WhiteIG80\n").is_err());
    }
}
