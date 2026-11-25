// Added by BridgeTower GP, November 2026. See BRIDGETOWER-CHANGES.md.
//! Ctrl+V with plain text puts it on the command line; Ctrl+V with a picture (a screenshot)
//! attaches it as an image; IMAGEATTACH, ATTACH and PDFIMPORT typed open a file picker.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use cadcraft_engine::Session;
use cadcraft_engine::doc::{EntityKind, Raster};
use cadcraft_ui_egui::{CadApp, Pick, Services};
use egui::{Event, vec2};
use egui_kittest::Harness;
use serde_json::json;

fn harness(app: CadApp) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

#[test]
fn pasted_text_goes_to_the_command_line() {
    let mut h = harness(CadApp::new(Session::new(), Services::default()));
    h.event(Event::Paste("100,200".into()));
    h.run_steps(2);
    assert_eq!(h.state().cmd.buffer, "100,200");
    // Several lines become one, spaces between.
    h.state_mut().cmd.buffer.clear();
    h.event(Event::Paste("LINE\r\n0,0\n".into()));
    h.run_steps(2);
    assert_eq!(h.state().cmd.buffer, "LINE 0,0");
    assert!(h.state().session.log.iter().all(|l| !l.contains("clipboard is empty")), "no PASTECLIP error");
}

#[test]
fn a_pasted_picture_is_saved_and_attached_in_the_view() {
    // The app installs the real image decoder; here any file decodes to a 4 × 2 picture.
    cadcraft_engine::cmd::file::set_image_decoder(|_| Ok(Raster::new(4, 2, vec![200; 4 * 2 * 4]).expect("pixels")));
    let dir = std::env::temp_dir().join(format!("cadcraft-bt-paste-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = CadApp::new(Session::new(), Services::default());
    // A saved drawing: the picture goes in "<drawing> images" beside it.
    app.session.state_mut().unwrap().path = Some(dir.join("site plan.dwg").to_string_lossy().to_string());
    let mut h = harness(app);
    let picture = egui::ColorImage::new([40, 20], vec![egui::Color32::from_rgb(10, 120, 200); 40 * 20]);
    h.event(Event::PasteImage(Arc::new(picture)));
    h.run_steps(2);
    let images: Vec<_> = h
        .state()
        .session
        .doc()
        .unwrap()
        .model
        .iter()
        .filter_map(|e| if let EntityKind::Image(i) = &e.kind { Some(i.clone()) } else { None })
        .collect();
    assert_eq!(images.len(), 1, "{:?}", h.state().session.log.last());
    let saved = std::path::Path::new(&images[0].path);
    assert!(saved.starts_with(dir.join("site plan images")) && saved.is_file(), "{saved:?}");
    assert!(std::fs::read(saved).unwrap().starts_with(b"\x89PNG"));
    assert!(h.state().session.log.iter().any(|l| l.starts_with("Pasted the picture")), "{:?}", h.state().session.log.last());
    let _ = std::fs::remove_dir_all(&dir);
}

/// An app whose extra file dialog records what it was asked and answers `answer`.
fn picking(answer: Option<String>) -> (CadApp, Rc<RefCell<Vec<Pick>>>) {
    let asked = Rc::new(RefCell::new(Vec::new()));
    let a = asked.clone();
    let services = Services {
        pick_file: Some(Box::new(move |p: &Pick| {
            a.borrow_mut().push(p.clone());
            answer.clone()
        })),
        ..Services::default()
    };
    (CadApp::new(Session::new(), services), asked)
}

#[test]
fn attach_and_pdf_import_typed_open_a_file_picker() {
    let (mut app, asked) = picking(None);
    app.cmdline("iat");
    app.cmdline("attach");
    app.cmdline("pdfimport");
    assert!(matches!(asked.borrow().as_slice(), [Pick::Attach(_), Pick::Attach(_), Pick::Pdf(_)]), "{:?}", asked.borrow());
    assert!(!app.session.prompting(), "a cancelled picker starts nothing");
    // A chosen file starts the command with it: the next question is asked on the command line.
    let (mut app, _) = picking(Some("C:\\Plans\\A101 floor plan.pdf".into()));
    app.cmdline("pdfimport");
    assert!(app.session.prompting());
    let prompt = app.session.current_prompt().map(|p| p.message).unwrap_or_default();
    assert!(prompt.contains("page number"), "{prompt}");
    // JSON calls never open a picker.
    let (mut app, asked) = picking(None);
    let _ = app.run("imageattach", json!({ "path": "missing.png" }));
    assert!(asked.borrow().is_empty());
}
