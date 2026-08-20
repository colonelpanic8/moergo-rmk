use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn manifest(board: &str) -> toml::Value {
    let path = repo_root().join("crates").join(board).join("Cargo.toml");
    toml::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn board_source(board: &str, half: &str) -> String {
    let path = repo_root()
        .join("crates")
        .join(board)
        .join("src")
        .join(format!("{half}.rs"));
    fs::read_to_string(path).unwrap()
}

#[test]
fn board_firmware_compiles_the_same_shared_sources_and_rmk_features() {
    let glove80 = manifest("glove80-rmk");
    let go60 = manifest("go60-rmk");

    let glove80_dependencies = glove80["dependencies"].as_table().unwrap();
    let go60_dependencies = go60["dependencies"].as_table().unwrap();
    let mut glove80_features = glove80_dependencies["rmk"]["features"]
        .as_array()
        .unwrap()
        .clone();
    let mut go60_features = go60_dependencies["rmk"]["features"]
        .as_array()
        .unwrap()
        .clone();
    const GO60_FLASH_BUDGET_SWITCH: &str = "_no_split_peripheral_battery_service";
    assert!(
        !glove80_features
            .iter()
            .any(|feature| feature.as_str() == Some(GO60_FLASH_BUDGET_SWITCH))
    );
    assert!(
        go60_features
            .iter()
            .any(|feature| feature.as_str() == Some(GO60_FLASH_BUDGET_SWITCH))
    );
    glove80_features.retain(|feature| feature.as_str() != Some(GO60_FLASH_BUDGET_SWITCH));
    go60_features.retain(|feature| feature.as_str() != Some(GO60_FLASH_BUDGET_SWITCH));
    assert_eq!(
        glove80_features, go60_features,
        "board crates must enable the same RMK capabilities"
    );

    for board in ["glove80-rmk", "go60-rmk"] {
        assert!(
            !manifest(board)["dependencies"]
                .as_table()
                .unwrap()
                .contains_key("moergo-rmk"),
            "{board} must compile firmware tasks in the board binary"
        );

        let central = board_source(board, "central");
        for module in [
            "central_lighting.rs",
            "lighting.rs",
            "remote_boot.rs",
            "split_lighting.rs",
        ] {
            assert!(
                central.contains(&format!("../../moergo-rmk/src/{module}")),
                "{board} central must use shared {module}"
            );
        }

        let peripheral = board_source(board, "peripheral");
        for module in ["lighting.rs", "split_lighting.rs"] {
            assert!(
                peripheral.contains(&format!("../../moergo-rmk/src/{module}")),
                "{board} peripheral must use shared {module}"
            );
        }
    }
}

#[test]
fn board_crates_do_not_reach_into_each_others_sources() {
    for board in ["glove80-rmk", "go60-rmk"] {
        let source = repo_root().join("crates").join(board).join("src");
        for entry in fs::read_dir(source).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            let contents = fs::read_to_string(&path).unwrap();
            assert!(
                !contents.contains("../glove80-rmk/") && !contents.contains("../go60-rmk/"),
                "{} reaches into another board crate",
                path.display()
            );
        }
    }
}
