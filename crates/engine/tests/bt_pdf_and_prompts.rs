// Added by BridgeTower GP, November 2026. See BRIDGETOWER-CHANGES.md.
//! PDFIMPORT, PDF pages attached as images, and the command-line prompts of commands that used to
//! fail with "`x` is required" when typed.

use std::path::PathBuf;

use cadcraft_engine::cmd::file::{IoHooks, PdfHooks, set_image_decoder, set_io, set_pdf};
use cadcraft_engine::doc::EntityKind;
use cadcraft_engine::{Input, Session};
use serde_json::json;

fn hooks() {
    set_io(IoHooks {
        read: |b, name| cadcraft_io::read(b, name).map_err(|e| e.to_string()),
        write: |d, name| cadcraft_io::write(d, name).map_err(|e| e.to_string()),
        plot: None,
    });
    set_image_decoder(|b| cadcraft_io::raster::decode(b).map_err(|e| e.to_string()));
    set_pdf(PdfHooks {
        pages: |b| cadcraft_io::pdf_read::pages(b).map_err(|e| e.to_string()),
        read_page: |b, i| cadcraft_io::pdf_read::read_page(b, i).map(|(d, _)| d).map_err(|e| e.to_string()),
        render_png: |b, i, dpi| cadcraft_io::pdf_read::render_page_png(b, i, dpi).map_err(|e| e.to_string()),
    });
}

fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cadcraft-bt-pdf-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A one-page US Letter PDF with `content` and Helvetica as `/F1`.
fn pdf(content: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

/// A plan sheet: a 1" wall line at (1", 1") and a room label at (1", 2").
fn plan(dir: &std::path::Path) -> String {
    let p = dir.join("plan.pdf");
    std::fs::write(&p, pdf("72 72 m 144 72 l S\nBT /F1 10 Tf 72 144 Td (BEDROOM 2) Tj ET")).unwrap();
    p.to_string_lossy().to_string()
}

fn lines(s: &Session) -> Vec<cadcraft_engine::doc::Line> {
    s.doc().unwrap().model.iter().filter_map(|e| if let EntityKind::Line(l) = &e.kind { Some(l.clone()) } else { None }).collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6
}

#[test]
fn pdfimport_brings_in_linework_and_text_at_a_plot_scale() {
    hooks();
    let dir = folder("import");
    let path = plan(&dir);
    let mut s = Session::new();
    // An inch drawing at 1/8" = 1'-0": a paper inch is 96 inches.
    let r = s.execute("pdfimport", &json!({ "path": path, "scale": "1/8\"=1'-0\"", "at": [1000, 0] })).unwrap();
    assert_eq!(r["lines"], 1, "{r}");
    assert_eq!(r["texts"], 1, "{r}");
    assert_eq!(r["page"], json!([8.5, 11.0]));
    let l = &lines(&s)[0];
    assert!(close(l.a.x, 1000.0 + 96.0) && close(l.a.y, 96.0) && close(l.b.x, 1000.0 + 192.0), "{l:?}");
    let t = s.doc().unwrap().model.iter().find_map(|e| if let EntityKind::Text(t) = &e.kind { Some(t.clone()) } else { None }).unwrap();
    assert_eq!(t.value, "BEDROOM 2");
    assert!(close(t.insert.x, 1000.0 + 96.0) && close(t.insert.y, 192.0), "{t:?}");
    assert!(s.doc().unwrap().layers.iter().any(|l| l.name == "PDF_Geometry") && s.doc().unwrap().layers.iter().any(|l| l.name == "PDF_Text"));
    // A feet drawing at 1" = 20': a paper inch is 20 feet.
    let mut s = Session::new();
    s.execute("units", &json!({ "insunits": 2 })).unwrap();
    s.execute("pdfimport", &json!({ "path": path, "scale": "1\"=20'" })).unwrap();
    let l = &lines(&s)[0];
    assert!(close(l.a.x, 20.0) && close(l.b.x, 40.0), "{l:?}");
    // Bad input is refused, not guessed.
    assert!(s.execute("pdfimport", &json!({ "path": path, "scale": "huge" })).is_err());
    assert!(s.execute("pdfimport", &json!({ "path": path, "page": 2 })).is_err());
}

#[test]
fn pdfimport_typed_asks_for_the_file_page_point_and_scale() {
    hooks();
    let dir = folder("typed");
    let path = plan(&dir);
    let mut s = Session::new();
    s.cmdline("pdfimport").unwrap();
    assert!(s.prompting());
    s.cmdline(&path).unwrap();
    s.cmdline("").unwrap(); // page: the first
    s.cmdline("10,20").unwrap();
    s.cmdline("96").unwrap();
    assert!(!s.prompting(), "done after the scale");
    let l = &lines(&s)[0];
    assert!(close(l.a.x, 10.0 + 96.0) && close(l.a.y, 20.0 + 96.0), "{l:?}");
    // A path with spaces, typed as one answer.
    let spaced = dir.join("my plan set.pdf");
    std::fs::copy(&path, &spaced).unwrap();
    let mut s = Session::new();
    s.cmdline("pdfimport").unwrap();
    s.cmdline(&spaced.to_string_lossy()).unwrap();
    for _ in 0..3 {
        s.cmdline("").unwrap();
    }
    assert_eq!(lines(&s).len(), 1);
}

#[test]
fn a_pdf_page_attaches_as_an_image_at_paper_size() {
    hooks();
    let dir = folder("attach");
    let path = plan(&dir);
    let mut s = Session::new();
    let r = s.execute("imageattach", &json!({ "path": path, "size": "native", "dpi": 72 })).unwrap();
    let png = dir.join("plan-page1.png");
    assert!(png.is_file(), "the page is saved as a PNG beside the PDF");
    assert_eq!(r["path"], png.to_string_lossy().to_string());
    assert_eq!(r["pixels"], json!([612.0, 792.0]));
    // An inch drawing: the page is 8.5" wide.
    assert!(close(r["size"][0].as_f64().unwrap(), 8.5), "{r}");
    // ATTACH takes a PDF too, and typed IMAGEATTACH asks for the page.
    let mut s = Session::new();
    s.cmdline("iat").unwrap();
    s.cmdline(&path).unwrap();
    s.cmdline("1").unwrap();
    s.cmdline("0,0").unwrap();
    s.cmdline("").unwrap();
    assert!(!s.prompting());
    assert_eq!(s.doc().unwrap().model.iter().filter(|e| matches!(e.kind, EntityKind::Image(_))).count(), 1);
    assert!(s.execute("attach", &json!({ "path": path })).is_ok());
}

#[test]
fn typed_commands_ask_on_the_command_line_instead_of_failing() {
    hooks();
    let mut s = Session::new();
    s.execute("line", &json!({ "points": [[0, 0], [10, 0]] })).unwrap();
    s.cmdline("base").unwrap();
    s.cmdline("5,7").unwrap();
    let base = s.doc().unwrap().header.point("INSBASE").unwrap();
    assert!(close(base.x, 5.0) && close(base.y, 7.0), "{base:?}");

    s.cmdline("limits").unwrap();
    s.cmdline("0,0").unwrap();
    s.cmdline("120,90").unwrap();
    let max = s.doc().unwrap().header.point("LIMMAX").unwrap();
    assert!(close(max.x, 120.0) && close(max.y, 90.0), "{max:?}");

    s.cmdline("getvar").unwrap();
    s.cmdline("LIMMAX").unwrap();
    assert!(s.log.iter().any(|l| l.starts_with("LIMMAX = ")), "{:?}", s.log.last());

    s.cmdline("color").unwrap();
    s.cmdline("red").unwrap();
    s.cmdline("lweight").unwrap();
    s.cmdline("0.35").unwrap();
    s.execute("layer.new", &json!({ "name": "Walls" })).unwrap();
    s.cmdline("clayer").unwrap();
    s.cmdline("Walls").unwrap();
    assert_eq!(s.doc().unwrap().header.str("CLAYER", ""), "Walls");

    // LAYMCH: objects, then the destination layer.
    let h = s.doc().unwrap().model.iter().next().unwrap().handle;
    s.cmdline("laymch").unwrap();
    s.input(Input::Pick(vec![h])).unwrap();
    s.input(Input::Enter).unwrap();
    s.cmdline("Walls").unwrap();
    assert_eq!(s.doc().unwrap().entity(h).unwrap().common.layer, "Walls");

    // Esc at any question leaves things as they were.
    s.cmdline("base").unwrap();
    s.input(Input::Cancel).unwrap();
    assert!(!s.prompting());
}

#[test]
fn no_typed_command_fails_for_a_missing_value_any_more() {
    hooks();
    for name in [
        "base",
        "blocklibrary",
        "blocklibimport",
        "clayer",
        "laymch",
        "attach",
        "imageattach",
        "iat",
        "color",
        "col",
        "colour",
        "lweight",
        "lw",
        "lineweight",
        "limits",
        "getvar",
        "datalink",
        "lisp",
        "pdfimport",
    ] {
        let mut s = Session::new();
        s.execute("line", &json!({ "points": [[0, 0], [10, 0]] })).unwrap();
        let r = s.cmdline(name);
        assert!(r.is_ok(), "{name}: {r:?}");
        assert!(s.prompting(), "{name} asks for its value");
        s.input(Input::Cancel).unwrap();
    }
}
