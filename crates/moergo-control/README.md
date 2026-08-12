# moergo-control

Native Glove80 control CLI using RMK's Rynk protocol over USB HID or BLE. It
controls current firmware only; the retired Glove80 product protocol
is intentionally not supported.

Run from the repository's Nix development shell:

```bash
cargo run -p moergo-control -- --help
cargo run -p moergo-control -- --usb version
```

The top-level commands are:

- `config validate|diff|apply|pull|show`
- `connection status|switch|clear|name`
- `device-data` (namespaced static metadata and live state as JSON)
- `keymap read|set|default|monitor|find`
- `lighting ping|caps|set|unset|clear|read|frame|replica-status|replace|brightness`
- `lighting scene-read|scene-set|scene-unset|scene-policy|params`
- `version`
- `bootloader [--peripheral] [--yes]`
- `maintenance`

Device selection defaults to USB with BLE fallback. Use `--usb` or `--ble` to
require one transport, and `--device` to select a `/dev/hidraw*` or BLE address
when multiple keyboards are available.

`keymap set` accepts `LAYER KEY KEYCODE` triples. A key may be a flat index or
`row,col`; keycodes use familiar names such as `KC_A`, `MO(2)`, and
`LT(1,KC_ESC)`.

Lighting commands operate on RMK's topology-aware overlay and revisioned
state. `lighting replace` accepts one cell per line:

```text
12 ff0000
40 00ff00 blink period=750 duty=30
```

Overlay cells are transient. Per-layer scene cells are stored by the keyboard
and survive a reboot. For example, this makes LED 29 blue whenever layer 1 is
active and composes it with the other active layers:

```bash
cargo run -p moergo-control -- lighting scene-set 1 29 blue
cargo run -p moergo-control -- lighting scene-policy active-stack
cargo run -p moergo-control -- lighting scene-read
```

The maintenance lock covers all remote mutations, matrix monitoring, storage
reset, and bootloader entry. Hold Magic and tap R to toggle it: R glows green
while the lock is off and unattended automation is allowed, and red while the
lock is engaged. Use `moergo-control maintenance` to read both the live and
compiled-default state.

The `config` commands provide a bidirectional TOML snapshot of managed runtime
state. `config diff FILE` compares the file with a live keyboard;
`config apply FILE` writes only differences and verifies readback; `config
pull FILE` writes the keyboard state to disk; and `config show` prints it.
Set `bluetooth_name = "Glove80 {slot}"` at the top level to give each active
BLE profile a distinct one-based advertising name. The same persistent value
can be inspected or changed directly with `connection name get` and
`connection name set TEMPLATE`; names are limited to 16 UTF-8 bytes.
Lighting extensions remain generic: effect and palette names come from Rynk's
extension descriptor, regardless of the firmware-side effect provider. Effects
may also advertise tunable parameters, which the file addresses by name:

```toml
[lighting.effects.params.Rain]
Density = 6
"Trail Length" = 128
```

A file owns only the parameters it lists; the rest keep whatever value the
keyboard holds. `config pull` records only parameters that differ from their
firmware default. Firmware that predates the parameter commands simply
advertises none, so reads degrade quietly and only a file that names a
parameter fails.

`lighting params` lists every effect that advertises parameters, `lighting
params EFFECT` lists one effect's, and `lighting params EFFECT NAME VALUE`
writes one.

Conditional lighting rules — the runtime counterpart of the ones a board
compiles in — are managed the same way. A rule applies when every condition it
names holds; naming none makes it unconditional:

```toml
[[lighting.conditional_scene]]
led = 75
color = "#0040a0"
layer = { layer = 2, active = true }
battery = { node = 1, min_level = 81, charge = "charging" }
```

Use `key = N` instead of `led = N` in either scene table to target the logical
key at index `N` in the keyboard's canonical layout. `config diff` and `config
apply` resolve it through the topology advertised by the connected keyboard,
including every emitter associated with that key. Raw LED targets remain
available for underglow, indicators, and other emitters that are not keys.

The same selectors address key bindings. A layer's `keys` grid is the fastest
way to read a whole layer and the worst way to say "every key in this zone", so
`[[layer.bind]]` entries layer over the grid:

```toml
[[layer]]
id = "magic"
name = "Magic"
keys = """…"""

[[layer.bind]]
key = [3, 0]
action = "QK_BOOT"

[[layer.bind]]
zone = 1
action = "KC_NO"
```

Binds apply after the grid in file order, and the last one covering a key wins
it, so a broad selector can be written first and corrected afterwards. A layer
with no `keys` at all starts transparent. `led = N` binds the key that owns
that emitter, and `all = true` binds every key the topology advertises.

Only `key = [row, col]` is a question about the matrix; the rest are questions
about the board, so `config validate` checks that they are well formed and
names them as resolving against a connected keyboard, while `config diff` and
`config apply` resolve them for real. The keyboard stores a grid rather than
the selectors a file used to describe one, so `config pull` writes the whole
layer back as `keys` — the same flattening that turns pulled lighting cells
back into `led` selectors.

Unlike `[[lighting.scene]]`, this table is ordered: matching rules compose in
table order and later ones win the slots they share, so `config diff` reports
by position and reordering two rules is a real difference. The table is written
as one atomic replacement, and a runtime rule outranks a compiled one on the
same LED, which is what lets a host replace a board's built-in status lighting
rather than only add to it. Firmware without the runtime conditional commands
reports no table at all, so a file that names no rules still applies cleanly
and only one that names them fails.
