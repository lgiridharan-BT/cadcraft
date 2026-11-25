// Added by BridgeTower GP, November 2026: command-line prompts for commands that had none, so
// typing them asks for their values (as AutoCAD does) instead of failing with "`x` is required".
// See BRIDGETOWER-CHANGES.md. Original project: https://github.com/storytold/cadcraft
//! A command that asks its questions on the command line, one after another, and then runs the
//! command with the answers as its parameters ([`Asked`]).

use serde_json::{Map, Value, json};

use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};
use cadcraft_geom::Vec2;

/// What a question takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A point: picked or typed `x,y`. Its parameter is `[x, y]`.
    Point,
    /// A number (or a distance picked from the previous point).
    Number,
    /// Free text, spaces included (names, file paths, expressions).
    Text,
    /// Objects, ending with Enter. Its parameter is a list of handles.
    Objects,
    /// One object, picked. Its parameter is its handle.
    Object,
}

/// One question: the parameter it fills, what it takes and the prompt.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Q {
    pub key: &'static str,
    pub kind: Kind,
    pub message: &'static str,
    /// Enter skips the question (the command's own default applies) instead of cancelling.
    pub optional: bool,
}

pub(crate) const fn q(key: &'static str, kind: Kind, message: &'static str) -> Q {
    Q { key, kind, message, optional: false }
}

pub(crate) const fn opt(key: &'static str, kind: Kind, message: &'static str) -> Q {
    Q { key, kind, message, optional: true }
}

/// How the answers become the command's result on the command line.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Show {
    /// The result's `message`, if it has one.
    Message,
    /// `NAME = value` (GETVAR).
    Variable,
    /// The whole result, as JSON.
    Result,
}

/// Asks `questions` in turn, then runs `command` with the answers.
pub(crate) struct Asked {
    title: &'static str,
    command: &'static str,
    questions: &'static [Q],
    at: usize,
    params: Map<String, Value>,
    picked: Vec<String>,
    last: Option<Vec2>,
    show: Show,
    /// Parameters always passed, whatever the answers.
    extra: &'static [(&'static str, &'static str)],
}

impl Asked {
    pub(crate) fn new(title: &'static str, command: &'static str, questions: &'static [Q]) -> Self {
        Asked { title, command, questions, at: 0, params: Map::new(), picked: Vec::new(), last: None, show: Show::Message, extra: &[] }
    }

    pub(crate) fn with(mut self, extra: &'static [(&'static str, &'static str)]) -> Self {
        self.extra = extra;
        self
    }

    pub(crate) fn showing(mut self, show: Show) -> Self {
        self.show = show;
        self
    }

    /// Box it for a command spec's `interactive`.
    pub(crate) fn boxed(self) -> Result<Box<dyn Interactive>> {
        Ok(Box::new(self))
    }

    fn question(&self) -> Option<&Q> {
        self.questions.get(self.at)
    }

    /// Record an answer and move on; after the last question run the command.
    fn answer(&mut self, s: &mut Session, value: Option<Value>) -> Result<Step> {
        let Some(q) = self.question().copied() else { return Ok(Step::Done) };
        if let Some(v) = value {
            self.params.insert(q.key.to_string(), v);
        }
        self.at += 1;
        // A PDF's page number means nothing for an image file.
        let is_pdf = self.params.get("path").and_then(Value::as_str).is_some_and(|p| p.to_ascii_lowercase().ends_with(".pdf"));
        while self.question().is_some_and(|q| q.key == "page") && !is_pdf {
            self.at += 1;
        }
        if self.at < self.questions.len() {
            return Ok(Step::Continue);
        }
        self.finish(s)
    }

    fn finish(&mut self, s: &mut Session) -> Result<Step> {
        for (k, v) in self.extra {
            self.params.entry((*k).to_string()).or_insert_with(|| json!(v));
        }
        let params = Value::Object(std::mem::take(&mut self.params));
        match s.execute(self.command, &params) {
            Ok(r) => {
                match self.show {
                    Show::Variable => {
                        let name = params.get("name").and_then(Value::as_str).unwrap_or_default().to_ascii_uppercase();
                        let v = r.get("value").cloned().unwrap_or(r);
                        let shown = v.as_str().map_or_else(|| v.to_string(), str::to_string);
                        s.echo(format!("{name} = {shown}"));
                    }
                    Show::Message => {
                        if let Some(m) = r.get("message").and_then(Value::as_str) {
                            s.echo(m.to_string());
                        }
                    }
                    Show::Result => {
                        let mut t = r.to_string();
                        if t.len() > 2000 {
                            t.truncate(t.char_indices().take_while(|(i, _)| *i < 2000).last().map_or(0, |(i, c)| i + c.len_utf8()));
                            t.push('…');
                        }
                        s.echo(t);
                    }
                }
                Ok(Step::Done)
            }
            Err(crate::EngineError::BadParams { msg, .. }) => {
                s.echo(msg);
                Ok(Step::Done)
            }
            Err(e) => Err(e),
        }
    }
}

/// `x,y` (or `x,y,z`) typed as text.
fn parse_point(t: &str) -> Option<Vec2> {
    let mut it = t.split(',').map(|v| v.trim().parse::<f64>());
    let x = it.next()?.ok()?;
    let y = it.next()?.ok()?;
    (x.is_finite() && y.is_finite()).then_some(Vec2::new(x, y))
}

impl Interactive for Asked {
    fn name(&self) -> &'static str {
        self.title
    }

    fn prompt(&self, _: &Session) -> Prompt {
        let Some(q) = self.question() else { return Prompt::new("", Accept::TEXT) };
        let accept = match q.kind {
            Kind::Point => Accept::POINT,
            Kind::Number => Accept::NUMBER,
            Kind::Text => Accept::TEXT,
            Kind::Objects | Kind::Object => Accept::SELECT,
        };
        let mut p = Prompt::new(q.message, accept);
        if q.kind == Kind::Number {
            p.base = self.last;
        }
        p
    }

    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        let Some(q) = self.question().copied() else { return Ok(Step::Done) };
        match i {
            Input::Cancel => Ok(Step::Cancel),
            Input::Enter => {
                if q.kind == Kind::Objects && !self.picked.is_empty() {
                    let handles = json!(std::mem::take(&mut self.picked));
                    return self.answer(s, Some(handles));
                }
                if q.optional { self.answer(s, None) } else { Ok(Step::Cancel) }
            }
            Input::Pick(hs) if q.kind == Kind::Object => match hs.first() {
                Some(h) => self.answer(s, Some(json!(h.hex()))),
                None => Ok(Step::Continue),
            },
            Input::Pick(hs) if q.kind == Kind::Objects => {
                for h in hs {
                    let hex = h.hex();
                    if !self.picked.contains(&hex) {
                        self.picked.push(hex);
                    }
                }
                Ok(Step::Continue)
            }
            Input::Point(p) | Input::Deferred(crate::snap::Deferred { at: p, .. }) => match q.kind {
                Kind::Point => {
                    self.last = Some(p);
                    self.answer(s, Some(json!([p.x, p.y])))
                }
                Kind::Number => match self.last {
                    Some(b) => self.answer(s, Some(json!((p - b).len()))),
                    None => Ok(Step::Continue),
                },
                Kind::Text => self.answer(s, Some(json!(format!("{},{}", p.x, p.y)))),
                Kind::Objects | Kind::Object => Ok(Step::Continue),
            },
            Input::Text(t) | Input::Keyword(t) => {
                let t = t.trim();
                if t.is_empty() {
                    return self.input(s, Input::Enter);
                }
                match q.kind {
                    Kind::Text => {
                        // A pasted Windows path often comes quoted.
                        let v = t.trim_matches('"').to_string();
                        self.answer(s, Some(json!(v)))
                    }
                    Kind::Number => match t.parse::<f64>() {
                        Ok(n) if n.is_finite() => self.answer(s, Some(json!(n))),
                        _ => {
                            s.echo("Requires a number.".to_string());
                            Ok(Step::Continue)
                        }
                    },
                    Kind::Point => match parse_point(t) {
                        Some(p) => {
                            self.last = Some(p);
                            self.answer(s, Some(json!([p.x, p.y])))
                        }
                        None => {
                            s.echo("Requires a point (x,y) or a pick.".to_string());
                            Ok(Step::Continue)
                        }
                    },
                    Kind::Objects | Kind::Object => Ok(Step::Continue),
                }
            }
            _ => Ok(Step::Continue),
        }
    }
}

// ---------- the questions of each command ----------

pub(crate) const BASE: &[Q] = &[q("at", Kind::Point, "Enter base point")];
pub(crate) const CLAYER: &[Q] = &[q("name", Kind::Text, "Enter new value for CLAYER (a layer name)")];
pub(crate) const LAYMCH: &[Q] =
    &[q("handles", Kind::Objects, "Select objects to be changed"), q("layer", Kind::Text, "Enter the destination layer's name")];
pub(crate) const COLOR: &[Q] = &[q("color", Kind::Text, "Enter default object color (ByLayer, ByBlock, a name such as red, 1-255, or r,g,b)")];
pub(crate) const LWEIGHT: &[Q] = &[q("lineweight", Kind::Text, "Enter default lineweight for new objects (millimetres, ByLayer, ByBlock or Default)")];
pub(crate) const LIMITS: &[Q] = &[q("min", Kind::Point, "Specify lower left corner"), q("max", Kind::Point, "Specify upper right corner")];
pub(crate) const GETVAR: &[Q] = &[q("name", Kind::Text, "Enter variable name")];
pub(crate) const LISP: &[Q] = &[q("code", Kind::Text, "Enter an AutoLISP expression")];
pub(crate) const DATALINK: &[Q] =
    &[q("handle", Kind::Object, "Select a table"), q("path", Kind::Text, "Enter the CSV or tab-separated file to link")];
pub(crate) const BLOCKLIBRARY: &[Q] = &[q("path", Kind::Text, "Enter a folder of drawings, or one drawing file")];
pub(crate) const BLOCKLIBIMPORT: &[Q] =
    &[q("path", Kind::Text, "Enter the drawing file to take a block from"), opt("block", Kind::Text, "Enter the block's name <the whole drawing>")];
pub(crate) const IMAGEATTACH: &[Q] = &[
    q("path", Kind::Text, "Enter the image (PNG, JPEG, BMP, TIFF) or PDF file"),
    opt("page", Kind::Number, "Enter the PDF page number <1>"),
    opt("at", Kind::Point, "Specify insertion point <0,0>"),
    opt("width", Kind::Number, "Specify the width in drawing units <the image's own size>"),
];
pub(crate) const PDFIMPORT: &[Q] = &[
    q("path", Kind::Text, "Enter the PDF file to import"),
    opt("page", Kind::Number, "Enter the page number <1>"),
    opt("at", Kind::Point, "Specify insertion point <0,0>"),
    opt("scale", Kind::Text, "Enter scale: drawing units per paper inch, or a plot scale such as 1/8\"=1'-0\" or 1\"=20' <1>"),
];

// ---------- units and plot scales ----------

/// Inches in one drawing unit, from INSUNITS (unitless drawings count as inches).
pub(crate) fn inches_per_unit(d: &cadcraft_doc::Drawing) -> f64 {
    match d.header.i64("INSUNITS", 1) {
        2 => 12.0,
        3 => 63360.0,
        4 => 1.0 / 25.4,
        5 => 1.0 / 2.54,
        6 => 1000.0 / 25.4,
        7 => 1_000_000.0 / 25.4,
        10 => 36.0,
        14 => 100.0 / 25.4,
        15 => 10_000.0 / 25.4,
        _ => 1.0,
    }
}

/// A length in inches: `1/8"`, `3"`, `1'`, `1'-6"`, `20'`, `1 1/2"`; a bare number is inches.
fn length_inches(t: &str) -> Option<f64> {
    let t = t.trim();
    let number = |s: &str| -> Option<f64> {
        let s = s.trim().trim_end_matches('"').trim();
        if s.is_empty() {
            return Some(0.0);
        }
        // `1 1/2`, `1-1/2` or `3/16`.
        let s = s.replace('-', " ");
        let mut total = 0.0;
        for part in s.split_whitespace() {
            total += match part.split_once('/') {
                Some((a, b)) => a.parse::<f64>().ok()? / b.parse::<f64>().ok().filter(|b| *b != 0.0)?,
                None => part.parse::<f64>().ok()?,
            };
        }
        Some(total)
    };
    let v = match t.split_once('\'') {
        Some((feet, inches)) => number(feet)? * 12.0 + number(inches.trim_start_matches('-'))?,
        None => number(t)?,
    };
    (v.is_finite() && v > 0.0).then_some(v)
}

/// Drawing units per paper inch for `t`: a number as given, a ratio `1:100`, or a plot scale
/// `paper=real` such as `1/8"=1'-0"` or `1"=20'` (real inches per paper inch, in this drawing's
/// units).
pub(crate) fn units_per_paper_inch(t: &str, d: &cadcraft_doc::Drawing) -> Option<f64> {
    let t = t.trim();
    if let Ok(n) = t.parse::<f64>() {
        return (n.is_finite() && n > 0.0).then_some(n);
    }
    let per_unit = inches_per_unit(d);
    if let Some((a, b)) = t.split_once(':') {
        let (a, b) = (a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?);
        return (a > 0.0 && b > 0.0).then(|| b / a / per_unit);
    }
    let (paper, real) = t.split_once('=')?;
    let (paper, real) = (length_inches(paper)?, length_inches(real)?);
    Some(real / paper / per_unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plot_scales_in_an_inch_and_a_foot_drawing() {
        let inch = cadcraft_doc::Drawing::new_imperial();
        let mut foot = cadcraft_doc::Drawing::new_imperial();
        foot.header.set_i64("INSUNITS", 2);
        assert_eq!(units_per_paper_inch("1/8\"=1'-0\"", &inch), Some(96.0));
        assert_eq!(units_per_paper_inch("1/8\"=1'-0\"", &foot), Some(8.0));
        assert_eq!(units_per_paper_inch("1/4\"=1'", &inch), Some(48.0));
        assert_eq!(units_per_paper_inch("1\"=20'", &foot), Some(20.0));
        assert_eq!(units_per_paper_inch("3/16\" = 1'-0\"", &inch), Some(64.0));
        assert_eq!(units_per_paper_inch("1 1/2\"=1'-0\"", &inch), Some(8.0));
        assert_eq!(units_per_paper_inch("1:100", &inch), Some(100.0));
        assert_eq!(units_per_paper_inch("2.5", &inch), Some(2.5));
        assert_eq!(units_per_paper_inch("abc", &inch), None);
        assert_eq!(units_per_paper_inch("0", &inch), None);
    }
}
