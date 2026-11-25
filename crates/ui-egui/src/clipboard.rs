// Modified by BridgeTower GP, November 2026 (PDF import/attach, typed-command prompts, paste fixes).
// See BRIDGETOWER-CHANGES.md. Original: https://github.com/storytold/cadcraft
//! The system clipboard: Cut/Copy/Paste key presses, and the drawing clipboard mirrored as DXF
//! text so it can be pasted into another CADCraft window.
//!
//! egui turns Cmd/Ctrl+C, X and V into `Copy`, `Cut` and `Paste(text)` events instead of key
//! presses, so the clipboard shortcuts are read from those. A `Paste` arrives only when the system
//! clipboard holds text; every copy therefore puts its DXF text there.

use std::hash::{DefaultHasher, Hash, Hasher};

use cadcraft_engine::cmd::clipboard;

use crate::CadApp;

/// egui memory: the clipboard copy last mirrored, and the hash of the text it became.
fn serial_id() -> egui::Id {
    egui::Id::unique("cc_clipboard_serial")
}
fn text_id() -> egui::Id {
    egui::Id::unique("cc_clipboard_text")
}

/// Line endings normalised (pasted text arrives with `\n`), so our own text is recognised.
fn hash(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.replace("\r\n", "\n").hash(&mut h);
    h.finish()
}

/// Put a new copy (COPYCLIP, CUTCLIP, COPYBASE from anywhere) on the system clipboard, once.
pub fn mirror(app: &CadApp, ctx: &egui::Context) {
    let Some(src) = &app.session.clipboard_source else { return };
    if src.uid.is_none() || ctx.data(|d| d.get_temp::<u64>(serial_id())) == Some(src.serial) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(serial_id(), src.serial));
    if let Some(text) = clipboard::system_text(&app.session) {
        ctx.data_mut(|d| d.insert_temp(text_id(), hash(&text)));
        ctx.copy_text(text);
    }
}

/// The commands the Cut/Copy/Paste events of this frame ask for. Pasted DXF text that isn't our
/// own last copy (another CADCraft's) becomes the clipboard first.
pub fn shortcut_commands(app: &mut CadApp, ctx: &egui::Context) -> Vec<&'static str> {
    let (events, shift) = ctx.input(|i| {
        let ev: Vec<egui::Event> =
            i.events.iter().filter(|e| matches!(e, egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_) | egui::Event::PasteImage(_))).cloned().collect();
        (ev, i.modifiers.shift)
    });
    let mut out = Vec::new();
    for e in events {
        match e {
            egui::Event::Copy => out.push(if shift { "copybase" } else { "copyclip" }),
            egui::Event::Cut => out.push("cutclip"),
            egui::Event::Paste(text) => {
                let h = hash(&text);
                let ours = ctx.data(|d| d.get_temp::<u64>(text_id())) == Some(h);
                if ours {
                    out.push(if shift { "pasteblock" } else { "pasteclip" });
                } else if clipboard::load_system_text(&mut app.session, &text) {
                    ctx.data_mut(|d| d.insert_temp(text_id(), h));
                    out.push(if shift { "pasteblock" } else { "pasteclip" });
                } else if !app.session.clipboard.is_empty() {
                    // Objects copied in CADCraft are pasted, whatever other text is about.
                    out.push(if shift { "pasteblock" } else { "pasteclip" });
                } else {
                    // BridgeTower GP: with nothing copied in CADCraft, plain text (a command,
                    // coordinates, a name) goes to the command line, as when typed; line breaks
                    // become spaces. It used to fail with "the clipboard is empty".
                    paste_text(app, &text);
                }
            }
            // BridgeTower GP: a picture on the clipboard (a screenshot) is saved as a PNG and
            // attached in the middle of the view, as AutoCAD pastes one.
            egui::Event::PasteImage(image) => {
                let msg = match paste_image(app, &image) {
                    Ok(m) => m,
                    Err(e) => format!("The picture couldn't be pasted: {e}"),
                };
                app.session.echo(msg.clone());
                app.set_status(msg);
            }
            _ => {}
        }
    }
    out
}

/// BridgeTower GP: plain pasted text, one line of it, added to what is typed at the command line.
pub fn paste_text(app: &mut CadApp, text: &str) {
    let line = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return;
    }
    // Keep the command line to a sensible length: a pasted document is not a command.
    let line: String = line.chars().take(4000).collect();
    app.cmd.buffer.push_str(&line);
}

/// BridgeTower GP: the folder a pasted picture is saved in: `<drawing> images` beside a saved
/// drawing, else `Pictures/CADCraft pastes` (or the home folder) for an unsaved one.
fn paste_folder(app: &CadApp) -> std::path::PathBuf {
    if let Some(path) = app.session.state().ok().and_then(|st| st.path.clone()) {
        let p = std::path::Path::new(&path);
        if let (Some(dir), Some(stem)) = (p.parent(), p.file_stem()) {
            return dir.join(format!("{} images", stem.to_string_lossy()));
        }
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let pictures = home.join("Pictures");
    let base = if pictures.is_dir() { pictures } else { home };
    base.join("CADCraft pastes")
}

/// BridgeTower GP: save `image` as a PNG ([`paste_folder`]) and attach it centred in the view,
/// half the view wide. Returns the message for the command line.
pub fn paste_image(app: &mut CadApp, image: &egui::ColorImage) -> Result<String, String> {
    let [w, h] = image.size;
    if w == 0 || h == 0 {
        return Err("the clipboard picture is empty".into());
    }
    let (w32, h32) = (u32::try_from(w).map_err(|e| e.to_string())?, u32::try_from(h).map_err(|e| e.to_string())?);
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    let mut png = Vec::new();
    cadcraft_render::raster::write_rgba_png(&mut png, w32, h32, &rgba).ok_or("PNG encoding failed")?;
    let folder = paste_folder(app);
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or_default();
    let file = folder.join(format!("pasted-{stamp}.png"));
    std::fs::write(&file, &png).map_err(|e| format!("{}: {e}", file.display()))?;
    let path = file.to_string_lossy().to_string();
    let view = app.session.state().map_err(|e| e.to_string())?.view();
    let (vw, vh) = app.session.viewport_px;
    let view_width = if vh > 0.0 { view.height * vw / vh } else { view.height };
    let width = if view_width.is_normal() { view_width * 0.5 } else { 1.0 };
    let height = width * h as f64 / w as f64;
    let at = [view.center.x - width / 2.0, view.center.y - height / 2.0];
    app.run("imageattach", serde_json::json!({ "path": path, "at": at, "width": width }))?;
    Ok(format!("Pasted the picture as an image ({w} x {h} pixels), saved as {path}"))
}
