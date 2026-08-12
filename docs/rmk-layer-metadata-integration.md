# RMK persistent layer metadata integration

This repository is qualified against generated RMK commit
`71b81a07dd5a65253b5d9abb25382ceb732219c4`.

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

The generated qualification tree contains those resolutions and the complete
30-entry carried stack. Its parent `c01e8602975d` contains the protocol and
configuration resolutions; the final generated patch commit adds the retained
pointing WASM endpoints.
