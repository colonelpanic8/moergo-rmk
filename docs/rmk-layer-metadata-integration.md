# RMK persistent layer metadata integration

This repository is qualified against generated RMK commit
`79a8f38d6082c98abf8b140875e3347b222bd62d`.

That commit was generated locally by `fork-assembler` from the RMK assembly
base `1a411da55bca` with the following first entry:

- branch: `feat/persistent-layer-metadata`
- commit: `05d4327ee0cc`
- subject: `feat(rynk): persist logical layer metadata`

The topic is intentionally not pushed by this change. Before publishing the
MoErgo pin, push the topic and replace the assembly manifest's temporary local
remote with the writable fork remote. Rebuild the assembly; do not hand-commit
to `assembled`.

The assembly resolution must retain all of these coherence changes:

- the layer metadata rows in the final Rynk protocol tests, documentation, and
  regenerated wire snapshots;
- `layer_names` initialization in the standalone lighting `Keymap` constructor
  introduced later in the stack;
- the existing pointing-config endpoints in `rynk-wasm`, which layer
  structural rewrites use to preserve pointing layer overrides.
- the wake-layer mask in `StandardState` and `StandardReplicaState`, including
  snapshot capture/application, so split renderers preserve runtime rewrites.

The generated qualification tree contains those resolutions and the complete
carried stack. The final coherence fixup is captured on `feat/device-data`,
then the generated runtime-lighting resolution retains split wake state and
the final generated patch commit retains the pointing WASM endpoints.
