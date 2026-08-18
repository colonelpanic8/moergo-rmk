use rmk::types::protocol::rynk::{
    DeviceDataDescriptor, DeviceDataRecord, DeviceDataValue, DeviceDataVolatility,
};

const RECORD_COUNT: u8 = 12;

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

fn peripheral_trace_text() -> heapless::String<96> {
    use core::fmt::Write as _;
    let mut out = heapless::String::new();
    match crate::central_lighting::peripheral_debug() {
        None => {
            let _ = out.push_str("none");
        }
        Some(d) => {
            // cause[0] is the peripheral's RX byte count; cause[1] packs
            // wired-entry, frame-ok, frame-bad and TX counts one byte each.
            let packed = d.cause[1];
            let _ = write!(
                out,
                "stage={} boots={} rr={:#x} wired={} rx={} ok={} bad={} tx={}",
                d.stage,
                d.boots,
                d.rr[0],
                packed >> 24,
                d.cause[0],
                (packed >> 16) & 0xff,
                (packed >> 8) & 0xff,
                packed & 0xff
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
        11 => Some(record(
            "split.wired.counters",
            DeviceDataVolatility::Live,
            text(wired_counter_text().as_str()),
        )),
        _ => None,
    }
}

/// This half's wired-link traffic, so a dead cable link can be read straight
/// off the central over USB while the split link itself is down.
fn wired_counter_text() -> heapless::String<64> {
    use core::fmt::Write as _;
    let (rx, ok, bad, tx) = rmk::split::serial::counters::snapshot();
    let mut out = heapless::String::new();
    let _ = write!(
        out,
        "wired={} rx={} ok={} bad={} tx={}",
        rmk::split::selector::wired_entries(),
        rx,
        ok,
        bad,
        tx
    );
    out
}
