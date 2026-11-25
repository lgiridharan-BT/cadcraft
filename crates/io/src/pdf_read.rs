// Added by BridgeTower GP, November 2026: PDF import (vector linework and text) and PDF page
// rendering for attaching a page as an image. See BRIDGETOWER-CHANGES.md.
// Original project: https://github.com/storytold/cadcraft
//! Reading PDF pages: their vector linework and text as drawing objects (PDFIMPORT), and a page
//! rendered to PNG to attach as an image.
//!
//! Coordinates come out in **paper inches** (a PDF point is 1/72 inch), with Y up and the page's
//! lower-left corner at the origin, so a 36 × 24 inch sheet imports 36 × 24 units wide; IMPORT's
//! `scale` then takes the sheet to real size (96 for 1/8" = 1'-0" in an inch drawing).
//!
//! A PDF from another program is untrusted input: the reader is bounded (input size, pages,
//! paths, vertices and glyphs), and anything too large is refused rather than truncated silently.

use cadcraft_color::{Color, Rgb};
use cadcraft_doc::{Circle, Common, Drawing, EntityKind, Line, LwPolyline, Space, Text};
use cadcraft_geom::{PolyVertex, Vec2, Vec3};
use craft_pdf::interpret::font::Glyph;
use craft_pdf::interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, RectExt, SoftMask,
    TransformExt,
};
use kurbo::{Affine, BezPath, PathEl, Point};

use crate::{IoError, Result};

/// Largest PDF read (bytes).
pub const MAX_INPUT: usize = 128 * 1024 * 1024;
/// Paths kept from one page.
const MAX_PATHS: usize = 400_000;
/// Vertices generated from one page (curves flattened).
const MAX_VERTICES: usize = 4_000_000;
/// Glyphs kept from one page.
const MAX_GLYPHS: usize = 400_000;
/// A PDF point in paper inches.
const PT: f64 = 1.0 / 72.0;
/// Cap height over font size: drawing text is sized by its capitals (as AutoCAD's is), PDF text
/// by its em square. 0.7 is the common figure for Helvetica/Arial-like fonts.
const CAP: f64 = 0.7;
/// Glyph-space units per em in the PDF interpreter (hayro-interpret's `UNITS_PER_EM`).
const GLYPH_EM: f64 = 1000.0;
/// Curve flattening tolerance, in PDF points (about 1/1000 inch).
const FLATTEN: f64 = 0.07;

pub const LAYER_GEOMETRY: &str = "PDF_Geometry";
pub const LAYER_FILLS: &str = "PDF_Solid Fills";
pub const LAYER_TEXT: &str = "PDF_Text";

fn err(e: impl std::fmt::Display) -> IoError {
    IoError::Format(format!("PDF: {e}"))
}

fn open(bytes: &[u8]) -> Result<craft_pdf::syntax::Pdf> {
    if bytes.len() > MAX_INPUT {
        return Err(err(format!("the file is larger than {} MB", MAX_INPUT / (1024 * 1024))));
    }
    let options = craft_pdf::OpenOptions { max_input_bytes: MAX_INPUT, ..Default::default() };
    craft_pdf::open_slice_with_options(bytes, None, &options).map_err(|e| err(format!("{e} (a password-protected or damaged PDF can't be read)")))
}

/// Whether `bytes` look like a PDF file.
pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes.len() > 5 && bytes[..bytes.len().min(1024)].windows(5).any(|w| w == b"%PDF-")
}

/// The number of pages, and each page's size in inches (width, height), as displayed.
pub fn pages(bytes: &[u8]) -> Result<Vec<(f64, f64)>> {
    let pdf = open(bytes)?;
    Ok(pdf
        .pages()
        .iter()
        .map(|p| {
            let (w, h) = p.render_dimensions();
            (f64::from(w) * PT, f64::from(h) * PT)
        })
        .collect())
}

/// What one page imported, besides the drawing.
#[derive(Debug, Default, Clone)]
pub struct ImportReport {
    pub lines: usize,
    pub polylines: usize,
    pub circles: usize,
    pub texts: usize,
    pub fills: usize,
    pub images_left_out: usize,
    pub glyphs_without_text: usize,
    /// The page's size in inches.
    pub page: (f64, f64),
}

/// Page `index` (0-based) as a drawing: its linework on `PDF_Geometry`, solid fills' outlines on
/// `PDF_Solid Fills` and its text as TEXT on `PDF_Text`, in paper inches.
pub fn read_page(bytes: &[u8], index: usize) -> Result<(Drawing, ImportReport)> {
    let pdf = open(bytes)?;
    let pages = pdf.pages();
    let page = pages.get(index).ok_or_else(|| err(format!("there is no page {} (the file has {})", index + 1, pages.len())))?;
    let cache = InterpreterCache::new();
    let settings = InterpreterSettings::default();
    let (w, h) = page.render_dimensions();
    let (w, h) = (f64::from(w), f64::from(h));
    let crop = page.intersected_crop_box().to_kurbo();
    let initial = page.initial_transform(true).to_kurbo();
    let mut context = Context::new(initial, crop, &cache, pdf.xref(), settings);
    let mut dev = Collector::default();
    craft_pdf::interpret_page(page, &mut context, &mut dev).map_err(err)?;
    if dev.exceeded {
        return Err(err("the page has more linework or text than can be imported at once"));
    }
    let mut d = Drawing::new_imperial();
    d.read_notes.clear();
    let mut report = ImportReport { page: (w * PT, h * PT), images_left_out: dev.images, glyphs_without_text: dev.unmapped, ..Default::default() };
    // Device space has Y down from the page's top-left corner; drawings have Y up.
    let to_cad = |p: Point| Vec2::new(p.x * PT, (h - p.y) * PT);
    for layer in [LAYER_GEOMETRY, LAYER_FILLS, LAYER_TEXT] {
        d.ensure_layer(layer);
    }
    for path in &dev.paths {
        let layer = if path.filled && !path.stroked { LAYER_FILLS } else { LAYER_GEOMETRY };
        let common = Common { layer: layer.into(), color: path.color.map_or(Color::ByLayer, Color::True), ..Common::default() };
        for sub in subpaths(&path.path) {
            let kind = match shape(&sub, &to_cad) {
                Some(k) => k,
                None => continue,
            };
            match &kind {
                EntityKind::Line(_) => report.lines += 1,
                EntityKind::Circle(_) => report.circles += 1,
                _ if layer == LAYER_FILLS => report.fills += 1,
                _ => report.polylines += 1,
            }
            d.add(&Space::Model, common.clone(), kind).map_err(err)?;
        }
    }
    for run in runs(&dev.glyphs) {
        // A run that starts with spaces starts where its first visible character does.
        let lead = run.text.chars().take_while(|c| c.is_whitespace()).count();
        let text = run.text.trim();
        if text.is_empty() {
            continue;
        }
        let origin = to_cad(run.origin + run.dir * run.lead_advance(lead));
        // The baseline's direction on the page, turned into the drawing's Y-up frame.
        let rotation = (-run.dir.y).atan2(run.dir.x);
        let height = run.size * PT * CAP;
        if !(origin.x.is_finite() && origin.y.is_finite() && height.is_normal()) {
            continue;
        }
        let t = Text {
            insert: Vec3::new(origin.x, origin.y, 0.0),
            height,
            value: text.to_string(),
            rotation,
            align_pt: None,
            width_factor: 1.0,
            oblique: 0.0,
            style: "Standard".into(),
            halign: Default::default(),
            valign: Default::default(),
        };
        let common = Common { layer: LAYER_TEXT.into(), color: run.color.map_or(Color::ByLayer, Color::True), ..Common::default() };
        d.add(&Space::Model, common, EntityKind::Text(t)).map_err(err)?;
        report.texts += 1;
    }
    let mut notes = Vec::new();
    if report.images_left_out > 0 {
        notes.push(format!("PDF: {} raster image(s) on the page were left out (attach the page as an image to see them)", report.images_left_out));
    }
    if report.glyphs_without_text > 0 {
        notes.push(format!("PDF: {} character(s) had no text mapping in their font and were left out", report.glyphs_without_text));
    }
    d.read_notes = notes;
    Ok((d, report))
}

/// Render page `index` (0-based) at `dpi` to PNG, on white. Returns the PNG and its pixel size.
pub fn render_page_png(bytes: &[u8], index: usize, dpi: f64) -> Result<(Vec<u8>, u32, u32)> {
    let pdf = open(bytes)?;
    let pages = pdf.pages();
    let page = pages.get(index).ok_or_else(|| err(format!("there is no page {} (the file has {})", index + 1, pages.len())))?;
    let (w, h) = page.render_dimensions();
    // Keep within the renderer's bounds: the longest side at most MAX_SIDE pixels, and the area
    // under MAX_PIXELS, lowering the resolution for very large sheets.
    let mut scale = (dpi / 72.0).clamp(0.1, 20.0);
    let side = f64::from(w.max(h)) * scale;
    if side > f64::from(craft_pdf::raster::MAX_SIDE) {
        scale *= f64::from(craft_pdf::raster::MAX_SIDE) / side;
    }
    let area = f64::from(w) * f64::from(h) * scale * scale;
    if area > craft_pdf::raster::MAX_PIXELS as f64 {
        scale *= (craft_pdf::raster::MAX_PIXELS as f64 / area).sqrt() * 0.999;
    }
    let options = craft_pdf::raster::Options { scale: [scale as f32, scale as f32], background: [255, 255, 255, 255], ..Default::default() };
    let cache = craft_pdf::raster::RenderCache::new();
    let raster = craft_pdf::raster::render(page, &cache, &InterpreterSettings::default(), &options).map_err(err)?;
    let mut out = Vec::new();
    // Opaque white background: premultiplied and straight RGBA are the same.
    cadcraft_render::raster::write_rgba_png(&mut out, raster.width, raster.height, raster.bytes()).ok_or_else(|| err("PNG encoding failed"))?;
    Ok((out, raster.width, raster.height))
}

// ---------- the device ----------

struct Painted {
    path: BezPath,
    stroked: bool,
    filled: bool,
    color: Option<Rgb>,
}

struct GlyphAt {
    text: String,
    /// The glyph origin on the baseline, device space (points, Y down).
    origin: Point,
    /// Unit vector along the baseline, device space.
    dir: kurbo::Vec2,
    /// Font size in points (the em along the glyph's vertical axis).
    size: f64,
    /// The pen's advance after this glyph, device space (points).
    advance: f64,
    color: Option<Rgb>,
}

#[derive(Default)]
struct Collector {
    paths: Vec<Painted>,
    glyphs: Vec<GlyphAt>,
    vertices: usize,
    images: usize,
    unmapped: usize,
    exceeded: bool,
}

fn paint_rgb(p: &Paint<'_>) -> Option<Rgb> {
    match p {
        Paint::Color(c) => {
            let [r, g, b, _] = c.to_rgba().to_rgba8();
            // Black and near-black follow the layer, as most plans are drawn in black.
            if r < 16 && g < 16 && b < 16 { None } else { Some(Rgb(r, g, b)) }
        }
        _ => None,
    }
}

impl Device<'_> for Collector {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, path: &BezPath, transform: Affine, paint: &Paint<'_>, mode: &PathDrawMode) {
        if path.is_empty() || self.exceeded {
            return;
        }
        let path = transform * path.clone();
        let filled = matches!(mode, PathDrawMode::Fill(_));
        // A fill and then a stroke of the same path (PDF's `B` operator) is one object.
        if !filled
            && let Some(last) = self.paths.last_mut()
            && last.filled
            && !last.stroked
            && last.path == path
        {
            last.stroked = true;
            return;
        }
        let count = path.elements().len();
        if self.paths.len() >= MAX_PATHS || self.vertices.saturating_add(count) > MAX_VERTICES {
            self.exceeded = true;
            return;
        }
        self.vertices += count;
        self.paths.push(Painted { path, stroked: !filled, filled, color: paint_rgb(paint) });
    }
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn draw_glyph(&mut self, glyph: &Glyph<'_>, transform: Affine, glyph_transform: Affine, paint: &Paint<'_>, _: &GlyphDrawMode) {
        if self.exceeded {
            return;
        }
        let Some(text) = glyph.as_unicode() else {
            self.unmapped += 1;
            return;
        };
        let text = match text {
            craft_pdf::interpret::hayro_cmap::BfString::Char(c) => c.to_string(),
            craft_pdf::interpret::hayro_cmap::BfString::String(s) => s,
        };
        if self.glyphs.len() >= MAX_GLYPHS {
            self.exceeded = true;
            return;
        }
        // hayro draws every font's glyphs at `GLYPH_EM` units per em and puts the font size,
        // the text matrix and the page into `transform * glyph_transform`: one em through it is
        // the font size on the page.
        let m = transform * glyph_transform;
        let origin = m * Point::ZERO;
        let x = m * Point::new(1.0, 0.0) - origin;
        let gx = x.hypot();
        let size = (m * Point::new(0.0, GLYPH_EM) - origin).hypot();
        if !(size.is_finite() && size > 1e-6 && gx.is_finite() && gx > 1e-12) {
            return;
        }
        let dir = x / gx;
        let advance = match glyph {
            Glyph::Outline(g) => g.advance_width().map(|a| f64::from(a) * gx),
            Glyph::Type3(g) => g.advance_width().map(|a| f64::from(a) * gx),
        }
        .filter(|a| a.is_finite())
        .unwrap_or(size * 0.5);
        self.glyphs.push(GlyphAt { text, origin, dir, size, advance, color: paint_rgb(paint) });
    }
    fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {
        self.images += 1;
    }
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

// ---------- paths ----------

/// The subpaths of `p`: each starts at a MoveTo.
fn subpaths(p: &BezPath) -> Vec<Vec<PathEl>> {
    let mut out: Vec<Vec<PathEl>> = Vec::new();
    for el in p.elements() {
        match el {
            PathEl::MoveTo(_) => out.push(vec![*el]),
            _ => match out.last_mut() {
                Some(cur) => cur.push(*el),
                None => out.push(vec![PathEl::MoveTo(Point::ZERO), *el]),
            },
        }
    }
    out.retain(|s| s.len() > 1);
    out
}

/// One subpath as a LINE (two points), a CIRCLE (a closed run of curves that stays a fixed
/// distance from its centre) or an LWPOLYLINE (curves flattened).
fn shape(sub: &[PathEl], to_cad: &impl Fn(Point) -> Vec2) -> Option<EntityKind> {
    let start = match sub.first() {
        Some(PathEl::MoveTo(p)) => Some(*p),
        _ => None,
    };
    let end = sub.iter().rev().find_map(|e| match e {
        PathEl::LineTo(p) | PathEl::CurveTo(_, _, p) | PathEl::QuadTo(_, p) => Some(*p),
        _ => None,
    });
    // Closed by `h`, or by ending where it started (as many PDF writers draw circles).
    let ends_at_start = matches!((start, end), (Some(a), Some(b)) if (a - b).hypot() <= 1e-3);
    let closed = matches!(sub.last(), Some(PathEl::ClosePath)) || ends_at_start && sub.len() > 3;
    // Curves, or (where the interpreter hands them over already flattened) many short segments.
    let has_curves = sub.iter().any(|e| matches!(e, PathEl::CurveTo(..) | PathEl::QuadTo(..))) || sub.len() >= 24;
    if closed
        && has_curves
        && let Some(c) = circle(sub, to_cad)
    {
        return Some(c);
    }
    let mut pts: Vec<Vec2> = Vec::new();
    let mut path = BezPath::new();
    for e in sub {
        path.push(*e);
    }
    kurbo::flatten(path.iter(), FLATTEN, |el| match el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => {
            let q = to_cad(p);
            if pts.last().is_none_or(|l: &Vec2| (*l - q).len() > 1e-7) {
                pts.push(q);
            }
        }
        _ => {}
    });
    if closed && pts.len() > 2 && (pts[0] - pts[pts.len() - 1]).len() <= 1e-5 {
        pts.pop();
    }
    if pts.iter().any(|p| !(p.x.is_finite() && p.y.is_finite())) {
        return None;
    }
    match pts.len() {
        0 | 1 => None,
        2 if !closed => Some(EntityKind::Line(Line { a: Vec3::new(pts[0].x, pts[0].y, 0.0), b: Vec3::new(pts[1].x, pts[1].y, 0.0) })),
        _ => Some(EntityKind::LwPolyline(LwPolyline {
            vertices: pts.into_iter().map(PolyVertex::new).collect(),
            closed,
            const_width: 0.0,
            elevation: 0.0,
            plinegen: false,
        })),
    }
}

/// A closed run of curves whose samples all sit within 0.5% of one radius from their centre.
fn circle(sub: &[PathEl], to_cad: &impl Fn(Point) -> Vec2) -> Option<EntityKind> {
    let mut path = BezPath::new();
    for e in sub {
        path.push(*e);
    }
    let mut pts = Vec::new();
    kurbo::flatten(path.iter(), FLATTEN / 4.0, |el| {
        if let PathEl::MoveTo(p) | PathEl::LineTo(p) = el {
            pts.push(to_cad(p));
        }
    });
    // The closing point repeats the first; counted twice it would pull the centre towards it.
    if pts.len() > 1 && (pts[0] - pts[pts.len() - 1]).len() <= 1e-5 {
        pts.pop();
    }
    if pts.len() < 8 {
        return None;
    }
    let n = pts.len() as f64;
    let c = pts.iter().fold(Vec2::ZERO, |a, p| a + *p) / n;
    let r = pts.iter().map(|p| (*p - c).len()).sum::<f64>() / n;
    if !(r.is_normal() && c.x.is_finite() && c.y.is_finite()) {
        return None;
    }
    if pts.iter().all(|p| ((*p - c).len() - r).abs() <= r * 0.005) {
        Some(EntityKind::Circle(Circle { center: Vec3::new(c.x, c.y, 0.0), radius: r }))
    } else {
        None
    }
}

// ---------- text ----------

struct Run {
    text: String,
    origin: Point,
    dir: kurbo::Vec2,
    size: f64,
    color: Option<Rgb>,
    /// The pen's advance after each glyph, in order (to skip leading spaces).
    advances: Vec<f64>,
}

impl Run {
    /// How far the first `n` glyphs move the pen.
    fn lead_advance(&self, n: usize) -> f64 {
        self.advances.iter().take(n).sum()
    }
}

/// Glyphs joined into lines of text: a glyph continues the run when it has the run's size and
/// direction and starts near where the previous glyph's advance left the pen; a gap wider than a
/// quarter of the size is a space, and anything else starts a new run.
fn runs(glyphs: &[GlyphAt]) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    let mut pen: Option<Point> = None;
    for g in glyphs {
        let mut joined = false;
        if let (Some(run), Some(p)) = (out.last_mut(), pen) {
            let same_size = (g.size - run.size).abs() <= run.size * 0.05;
            let same_dir = run.dir.dot(g.dir) > 0.9999;
            if same_size && same_dir {
                let d = g.origin - p;
                let along = d.dot(run.dir);
                let across = d.cross(run.dir).abs();
                if across <= run.size * 0.2 && along >= -run.size * 0.3 && along <= run.size * 3.0 {
                    if along > run.size * 0.25 && !run.text.ends_with(' ') && g.text != " " {
                        run.text.push(' ');
                    }
                    run.text.push_str(&g.text);
                    run.advances.push(g.advance);
                    joined = true;
                }
            }
        }
        if !joined {
            out.push(Run { text: g.text.clone(), origin: g.origin, dir: g.dir, size: g.size, color: g.color, advances: vec![g.advance] });
        }
        pen = Some(g.origin + g.dir * g.advance);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page PDF (US Letter, 612 × 792 pt) with `content` as its page content and Helvetica
    /// as `/F1`, with a correct cross-reference table.
    pub(crate) fn pdf(content: &str) -> Vec<u8> {
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

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn lines_rectangles_circles_and_text_come_in_at_paper_inches() {
        // A line 1" long at (1", 1"); a 2" × 1" rectangle at (3", 1"); a 0.5"-radius circle drawn
        // as four Béziers centred at (6", 2"); "HELLO WORLD" in 12 pt Helvetica at (1", 9").
        let k = 0.552_284_75 * 36.0;
        let c = format!(
            "1 w 72 72 m 144 72 l S\n216 72 144 72 re S\n\
             468 144 m 468 {a} {b} 180 432 180 c {d} 180 396 {a} 396 144 c 396 {e} {d} 108 432 108 c {b} 108 468 {e} 468 144 c S\n\
             BT /F1 12 Tf 72 648 Td (HELLO WORLD) Tj ET\n\
             BT /F1 12 Tf 0 1 -1 0 300 300 Tm (UP) Tj ET",
            a = 144.0 + k,
            b = 432.0 + k,
            d = 432.0 - k,
            e = 144.0 - k
        );
        let (d, r) = read_page(&pdf(&c), 0).unwrap();
        assert!(close(r.page.0, 8.5, 1e-9) && close(r.page.1, 11.0, 1e-9), "{:?}", r.page);
        let model: Vec<_> = d.model.iter().collect();
        let line = model.iter().find_map(|e| if let EntityKind::Line(l) = &e.kind { Some(l.clone()) } else { None }).expect("a line");
        assert!(close(line.a.x, 1.0, 1e-6) && close(line.a.y, 1.0, 1e-6) && close(line.b.x, 2.0, 1e-6) && close(line.b.y, 1.0, 1e-6), "{line:?}");
        let rect = model
            .iter()
            .find_map(|e| if let EntityKind::LwPolyline(p) = &e.kind { Some(p.clone()) } else { None })
            .expect("the rectangle as a closed polyline");
        assert!(rect.closed && rect.vertices.len() == 4, "{rect:?}");
        let xs: Vec<f64> = rect.vertices.iter().map(|v| v.p.x).collect();
        assert!(xs.iter().any(|x| close(*x, 3.0, 1e-6)) && xs.iter().any(|x| close(*x, 5.0, 1e-6)), "{xs:?}");
        let circle = model
            .iter()
            .find_map(|e| if let EntityKind::Circle(c) = &e.kind { Some(c.clone()) } else { None })
            .unwrap_or_else(|| panic!("a circle: {:#?}", model.iter().map(|e| &e.kind).collect::<Vec<_>>()));
        assert!(close(circle.center.x, 6.0, 1e-3) && close(circle.center.y, 2.0, 1e-3) && close(circle.radius, 0.5, 2e-3), "{circle:?}");
        let texts: Vec<_> = model.iter().filter_map(|e| if let EntityKind::Text(t) = &e.kind { Some(t.clone()) } else { None }).collect();
        let hello = texts.iter().find(|t| t.value == "HELLO WORLD").unwrap_or_else(|| panic!("one run with its space: {texts:?}"));
        assert!(close(hello.insert.x, 1.0, 1e-6) && close(hello.insert.y, 9.0, 1e-6), "{hello:?}");
        assert!(close(hello.height, 12.0 / 72.0 * CAP, 1e-6) && close(hello.rotation, 0.0, 1e-9), "{hello:?}");
        let up = texts.iter().find(|t| t.value == "UP").expect("the rotated text");
        assert!(close(up.rotation, std::f64::consts::FRAC_PI_2, 1e-9), "rotated 90° counter-clockwise: {up:?}");
        assert!(close(up.insert.x, 300.0 / 72.0, 1e-6) && close(up.insert.y, 300.0 / 72.0, 1e-6), "{up:?}");
        assert!(d.model.iter().all(|e| e.common.layer.starts_with("PDF_")));
    }

    #[test]
    fn leading_spaces_are_dropped_and_the_text_starts_at_its_first_letter() {
        let (d, _) = read_page(&pdf("BT /F1 10 Tf 72 72 Td (  APPROX. 8.36 AC) Tj ET"), 0).unwrap();
        let t = d.model.iter().find_map(|e| if let EntityKind::Text(t) = &e.kind { Some(t.clone()) } else { None }).unwrap();
        assert_eq!(t.value, "APPROX. 8.36 AC");
        // Helvetica's space is 278/1000 em: two of them at 10 pt.
        assert!(close(t.insert.x, 1.0 + 2.0 * 2.78 / 72.0, 1e-4), "{t:?}");
    }

    #[test]
    fn a_filled_and_stroked_path_is_one_object_and_fills_have_their_own_layer() {
        let c = "72 72 72 72 re B\n300 300 36 36 re f";
        let (d, _) = read_page(&pdf(c), 0).unwrap();
        let on = |layer: &str| d.model.iter().filter(|e| e.common.layer == layer).count();
        assert_eq!(on(LAYER_GEOMETRY), 1, "B (fill and stroke) is one outline");
        assert_eq!(on(LAYER_FILLS), 1, "f (fill only)");
    }

    #[test]
    fn pages_render_and_bad_input_is_refused() {
        let bytes = pdf("72 72 m 540 720 l S");
        assert_eq!(pages(&bytes).unwrap(), vec![(8.5, 11.0)]);
        let (png, w, h) = render_page_png(&bytes, 0, 72.0).unwrap();
        assert_eq!((w, h), (612, 792));
        assert!(png.starts_with(b"\x89PNG"));
        assert!(read_page(&bytes, 1).is_err(), "no second page");
        assert!(read_page(b"%PDF-1.4 garbage", 0).is_err());
        assert!(is_pdf(&bytes) && !is_pdf(b"0\nSECTION"));
    }
}
