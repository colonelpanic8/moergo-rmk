# Shared MoErgo firmware

This crate owns firmware services that should behave identically on the
Glove80 and Go60:

- the lighting engine and physical LED driver;
- Rynk lighting control and split-state replication;
- Magic-layer lighting actions; and
- cross-half bootloader routing.

`glove80-rmk` and `go60-rmk` are thin board crates. Each selects one feature
on this crate, supplies its RMK-generated hardware/configuration statics, and
registers only processors that are specific to that board. Shared behavior
must be implemented here rather than copied into a board crate.

The two feature sets are intentionally mutually exclusive. The embedded board
crates are separate Cargo workspaces, so each build gets one set of constants
and one `KEYBOARD_TOML_PATH`-derived topology:

| Feature | LEDs per half | Channel ceiling | Maintenance LED |
| --- | ---: | ---: | ---: |
| `glove80` | 40 | 230 | 12 |
| `go60` | 30 | 102 | 8 |

The remaining differences—matrix wiring, GPIOs, Go60 trackpads, and device
data—stay in their board crates. `crates/xtask/tests/board_parity.rs` prevents
either board from reaching into the other's source tree and keeps their RMK
capability sets aligned.
