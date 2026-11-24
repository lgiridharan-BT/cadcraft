# BridgeTower GP changes to CADCraft

This repository is a fork of [storytold/cadcraft](https://github.com/storytold/cadcraft)
(MIT OR Apache-2.0, see `LICENSE-MIT`, `LICENSE-APACHE` and `NOTICE`). The original copyright and
licence notices are kept unchanged. Changes made by BridgeTower GP:

| File | Change |
|---|---|
| `crates/engine/src/cmd/inquiry.rs` | `ID` (and the Z lookup behind it) reports the Z of the object inside a block reference nearest the picked point, instead of the reference's own insertion Z. Civil 3D spot-elevation labels read 0 before this. |
| `crates/engine/tests/it/inquiry_z.rs` | Test for the change above. |
| `crates/ui-egui/src/dialogs.rs` | About dialog: states this is a BridgeTower build based on CADCraft; ArtCraft links removed. |
| `.github/workflows/bridgetower-windows-build.yml` | Windows build: runs the engine tests, builds `cadcraft.exe`, publishes the `bridgetower-latest` release with the licence and notice files. |
| `docs/brand/*.png`, `docs/brand/*.svg` | Removed (ArtCraft trademarks; the brand terms require modified versions to remove them). `docs/brand/LICENSE-brand.txt` kept. |
| `README.md`, `ATTRIBUTION.md`, `NOTICE` | Fork notice, brand-file rows removed, modification notice added. |

Third-party components are unchanged. acadrust (MPL-2.0) is used unmodified from crates.io.
