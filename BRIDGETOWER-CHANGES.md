# BridgeTower GP changes to CADCraft

This repository is a fork of [storytold/cadcraft](https://github.com/storytold/cadcraft)
(MIT OR Apache-2.0, see `LICENSE-MIT`, `LICENSE-APACHE` and `NOTICE`). The original copyright and
licence notices are kept unchanged. Changes made by BridgeTower GP:

| File | Change |
|---|---|
| `crates/engine/src/cmd/inquiry.rs` | `ID` (and the Z lookup behind it) reports the Z of the object inside a block reference nearest the picked point, instead of the reference's own insertion Z. Civil 3D spot-elevation labels read 0 before this. A circle's or arc's centre and the inside of a solid fill count as on the object, so a Center snap or a click in the middle of a spot dot gives its Z. |
| `crates/engine/tests/it/inquiry_z.rs` | Tests for the change above. |
| `crates/ui-egui/src/dialogs.rs` | About dialog: states this is a BridgeTower build based on CADCraft; ArtCraft links removed. |
| `crates/io/src/pdf_read.rs` (new), `crates/io/src/lib.rs`, `crates/io/Cargo.toml` | PDF reading: a page's vector linework and text as drawing objects (PDFIMPORT; opening or IMPORTing a `.pdf` brings in page 1), and a page rendered to PNG. Uses the PDF reader CADCraft already ships (craft-pdf / hayro, MIT OR Apache-2.0) with its `raster` feature, and `kurbo` (MIT OR Apache-2.0, already in the build). |
| `Cargo.toml` | Pins `hayro`, `hayro-interpret`, `hayro-cmap`, `hayro-postscript` to the same reviewed craft-libs revision as craft-pdf. |
| `crates/engine/src/cmd/asked.rs` (new) and the command specs of BASE, CLAYER, LAYMCH, COLOR, LWEIGHT, LIMITS, GETVAR, LISP, DATALINK, BLOCKLIBRARY, BLOCKLIBIMPORT, IMAGEATTACH, ATTACH | Typed, these commands ask for their values on the command line (as AutoCAD does) instead of failing with "`x` is required". |
| `crates/engine/src/cmd/exchange.rs`, `crates/engine/src/cmd/image.rs`, `crates/engine/src/cmd/file.rs`, `crates/engine/src/file_access/commands.rs` | New PDFIMPORT command (Insert > PDF Import..., scale as a number or a plot scale such as `1/8"=1'-0"`); IMAGEATTACH/ATTACH take a PDF page (rendered to `<name>-page<N>.png` beside the PDF); IMAGEATTACH on the Insert menu; PDF hooks; PDFIMPORT held to the automation file roots like IMPORT. |
| `crates/ui-egui/src/clipboard.rs` | Ctrl+V: a picture on the clipboard (a screenshot) is saved as a PNG (`<drawing> images` beside the drawing, or `Pictures/CADCraft pastes`) and attached in the view; plain text goes to the command line when nothing is copied in CADCraft (it used to fail with "the clipboard is empty"). |
| `crates/ui-egui/src/lib.rs`, `crates/ui-egui/src/menus.rs`, `crates/ui-egui/src/options.rs`, `apps/cadcraft/src/main.rs`, `apps/cadcraft/src/sheet.rs`, `apps/cadcraft-cli/src/lib.rs` | File pickers for IMAGEATTACH/ATTACH (images and PDF) and PDFIMPORT; the PDF hooks installed; the About box shows the BridgeTower build number. |
| `crates/ui-egui/src/i18n/zh.tsv` | Chinese label for the new Insert > PDF Import... menu item (every menu label is translated there). |
| `crates/engine/tests/bt_pdf_and_prompts.rs`, `crates/ui-egui/tests/it/bt_paste_and_pickers.rs` (new), `crates/engine/tests/image_attach.rs`, `crates/engine/src/cmd/exchange.rs` (its ATTACH test) | Tests for the above (the old test that a PDF can't be attached now checks DWF, which still can't). |
| `.github/workflows/bridgetower-windows-build.yml` | Windows build: runs the engine tests, builds `cadcraft.exe`, publishes the `bridgetower-latest` release with the licence and notice files. |
| `docs/brand/*.png`, `docs/brand/*.svg` | Removed (ArtCraft trademarks; the brand terms require modified versions to remove them). `docs/brand/LICENSE-brand.txt` kept. |
| `README.md`, `ATTRIBUTION.md`, `NOTICE` | Fork notice, brand-file rows removed, modification notice added. |

Third-party components are unchanged. acadrust (MPL-2.0) is used unmodified from crates.io.
