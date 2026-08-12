use anyhow::Result;
use rynk::rmk_types::protocol::rynk::{
    DeviceDataDescriptor, DeviceDataRecord, DeviceDataValue, DeviceDataVolatility,
};
use serde_json::{Map, Number, Value};

use crate::transport::Selector;

pub fn run(selector: &Selector) -> Result<()> {
    crate::rynk_client::run_device_data(selector)
}

pub(crate) fn render(
    descriptor: &DeviceDataDescriptor,
    records: &[DeviceDataRecord],
) -> Result<String> {
    let mut static_values = Map::new();
    let mut state_values = Map::new();
    for record in records {
        let value = match &record.value {
            DeviceDataValue::Bool(value) => Value::Bool(*value),
            DeviceDataValue::Unsigned(value) => Value::Number(Number::from(*value)),
            DeviceDataValue::Signed(value) => Value::Number(Number::from(*value)),
            DeviceDataValue::Text(value) => Value::String(value.to_string()),
        };
        let target = match record.volatility {
            DeviceDataVolatility::Static => &mut static_values,
            DeviceDataVolatility::Live => &mut state_values,
        };
        target.insert(record.key.to_string(), value);
    }

    let mut root = Map::new();
    root.insert(
        "namespace".into(),
        Value::String(descriptor.namespace.to_string()),
    );
    root.insert(
        "schemaVersion".into(),
        Value::Number(Number::from(descriptor.schema_version)),
    );
    root.insert("static".into(), Value::Object(static_values));
    root.insert("state".into(), Value::Object(state_values));
    Ok(serde_json::to_string_pretty(&Value::Object(root))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_typed_static_and_live_records_as_json() {
        let descriptor = DeviceDataDescriptor {
            namespace: "com.moergo.go60".try_into().unwrap(),
            schema_version: 1,
            record_count: 2,
        };
        let records = [
            DeviceDataRecord {
                key: "split.policy".try_into().unwrap(),
                volatility: DeviceDataVolatility::Static,
                value: DeviceDataValue::Text("auto".try_into().unwrap()),
            },
            DeviceDataRecord {
                key: "split.cableDetected".try_into().unwrap(),
                volatility: DeviceDataVolatility::Live,
                value: DeviceDataValue::Bool(true),
            },
        ];

        let value: Value = serde_json::from_str(&render(&descriptor, &records).unwrap()).unwrap();
        assert_eq!(value["namespace"], "com.moergo.go60");
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["static"]["split.policy"], "auto");
        assert_eq!(value["state"]["split.cableDetected"], true);
    }
}
