use rmk::types::protocol::rynk::{
    DeviceDataDescriptor, DeviceDataRecord, DeviceDataValue, DeviceDataVolatility,
};

const RECORD_COUNT: u8 = 11;

pub fn descriptor() -> DeviceDataDescriptor {
    DeviceDataDescriptor {
        namespace: "com.moergo.go60".try_into().unwrap(),
        schema_version: 1,
        record_count: RECORD_COUNT,
    }
}

fn text(value: &str) -> DeviceDataValue {
    DeviceDataValue::Text(value.try_into().unwrap())
}

fn record(key: &str, volatility: DeviceDataVolatility, value: DeviceDataValue) -> DeviceDataRecord {
    DeviceDataRecord {
        key: key.try_into().unwrap(),
        volatility,
        value,
    }
}

fn peripheral_trace_text() -> heapless::String<64> {
    use core::fmt::Write as _;
    let mut out = heapless::String::new();
    match crate::central_lighting::peripheral_debug() {
        None => {
            let _ = out.push_str("none");
        }
        Some(d) => {
            let _ = write!(
                out,
                "stage={} boots={} rr={:#x},{:#x} cause={},{}",
                d.stage, d.boots, d.rr[0], d.rr[1], d.cause[0], d.cause[1]
            );
        }
    }
    out
}

pub fn record_at(index: u8) -> Option<DeviceDataRecord> {
    let wired = rmk::split::selector::wired_selected();
    match index {
        0 => Some(record(
            "device.model",
            DeviceDataVolatility::Static,
            text("go60"),
        )),
        1 => Some(record(
            "split.policy",
            DeviceDataVolatility::Static,
            text("auto"),
        )),
        2 => Some(record(
            "split.activeTransport",
            DeviceDataVolatility::Live,
            text(if wired { "wired" } else { "ble" }),
        )),
        3 => Some(record(
            "split.cableDetected",
            DeviceDataVolatility::Live,
            DeviceDataValue::Bool(wired),
        )),
        4 => Some(record(
            "split.peripheral.activeTransport",
            DeviceDataVolatility::Live,
            text(match crate::central_lighting::peripheral_transport() {
                None => "unknown",
                Some(report) if !report.auto => "fixed",
                Some(report) if report.wired => "wired",
                Some(_) => "ble",
            }),
        )),
        5 => Some(record(
            "split.force",
            DeviceDataVolatility::Live,
            text(match rmk::split::selector::forced_mode() {
                rmk::split::selector::FORCE_WIRED => "wired",
                rmk::split::selector::FORCE_BLE => "ble",
                _ => "auto",
            }),
        )),
        6 => Some(record(
            "debug.lastPanicLoc",
            DeviceDataVolatility::Static,
            text(
                crate::panic_store::last_panic()
                    .as_ref()
                    .map_or("none", |p| p.loc.as_str()),
            ),
        )),
        7 => Some(record(
            "debug.lastPanicMsg",
            DeviceDataVolatility::Static,
            text(
                crate::panic_store::last_panic()
                    .as_ref()
                    .map_or("none", |p| p.msg.as_str()),
            ),
        )),
        8 => Some(record(
            "debug.bootTrace",
            DeviceDataVolatility::Live,
            text(crate::panic_store::boot_trace().as_str()),
        )),
        9 => Some(record(
            "debug.peripheral.trace",
            DeviceDataVolatility::Live,
            text(peripheral_trace_text().as_str()),
        )),
        10 => Some(record(
            "debug.peripheral.panicLoc",
            DeviceDataVolatility::Live,
            text(
                crate::central_lighting::peripheral_debug()
                    .and_then(|d| d.panic_loc)
                    .as_deref()
                    .unwrap_or("none"),
            ),
        )),
        _ => None,
    }
}
