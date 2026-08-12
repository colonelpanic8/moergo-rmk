use std::env;
use std::path::PathBuf;

fn main() {
    let board = match (
        env::var_os("CARGO_FEATURE_GLOVE80").is_some(),
        env::var_os("CARGO_FEATURE_GO60").is_some(),
    ) {
        (true, false) => "glove80-rmk",
        (false, true) => "go60-rmk",
        _ => panic!("enable exactly one of the `glove80` or `go60` features"),
    };

    println!("cargo:rerun-if-env-changed=KEYBOARD_TOML_PATH");
    let config = env::var_os("KEYBOARD_TOML_PATH").map_or_else(
        || {
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
                .join("../")
                .join(board)
                .join("keyboard.toml")
        },
        PathBuf::from,
    );
    println!("cargo:rerun-if-changed={}", config.display());
    println!("cargo:rustc-env=KEYBOARD_TOML_PATH={}", config.display());
}
