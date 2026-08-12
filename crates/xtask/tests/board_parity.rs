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

#[test]
fn board_firmware_uses_the_same_shared_service_and_rmk_features() {
    let glove80 = manifest("glove80-rmk");
    let go60 = manifest("go60-rmk");

    let glove80_dependencies = glove80["dependencies"].as_table().unwrap();
    let go60_dependencies = go60["dependencies"].as_table().unwrap();
    assert_eq!(
        glove80_dependencies["rmk"]["features"], go60_dependencies["rmk"]["features"],
        "board crates must enable the same RMK capabilities"
    );

    for (board, feature, dependencies) in [
        ("glove80-rmk", "glove80", glove80_dependencies),
        ("go60-rmk", "go60", go60_dependencies),
    ] {
        let shared = &dependencies["moergo-rmk"];
        assert_eq!(shared["path"].as_str(), Some("../moergo-rmk"));
        assert_eq!(
            shared["features"].as_array().unwrap(),
            &[toml::Value::String(feature.into())],
            "{board} must select only its own board constants"
        );
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
