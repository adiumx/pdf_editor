//! Moving what is already on a page: text and pictures.
//!
//! A picture moves by wrapping its `Do` in a `q`, a translation and a `Q`. Text is
//! harder, because where a string lands depends on what came before it: an
//! absolute `Tm`, the pen left after the previous string, a relative `Td`. So a
//! run is moved by inserting a `Td` ahead of it, then reading the page again to
//! see where everything went and correcting what moved by accident, which is
//! what the line reflow does for the words after an edited one.

use lopdf::content::{Content, Operation};
use lopdf::Object;

use crate::error::{Error, Result};
use crate::font::resolve;
use crate::text::{invert, multiply, transform_box, TextRun, IDENTITY};
use crate::Document;

/// Closer than this (points) counts as in place.
const EPSILON: f32 = 0.02;
/// Corrections after which the page is left as it was.
const MAX_CORRECTIONS: usize = 400;

/// A picture drawn on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct PagePicture {
    /// Name of the image in the page's resources.
    pub name: String,
    /// Where its drawing operation is in the page's content.
    pub op_index: usize,
    /// `[x0, y0, x1, y1]` as a reader shows the page.
    pub display: [f32; 4],
}

impl Document {
    /// A displacement as a reader shows the page, in the page's own space.
    pub(crate) fn user_delta(&self, page: usize, dx: f32, dy: f32) -> Result<(f32, f32)> {
        let inverse = invert(self.display_matrix(page)?)
            .ok_or_else(|| Error::Unsupported("la página tiene una transformación no invertible".into()))?;
        Ok((dx * inverse[0] + dy * inverse[2], dx * inverse[1] + dy * inverse[3]))
    }

    /// The pictures a page draws (not the ones inside form XObjects).
    pub fn page_pictures(&self, page: usize) -> Result<Vec<PagePicture>> {
        let id = self.page_id(page)?;
        let to_display = self.display_matrix(page)?;
        let resources = self.resources(id);
        let xobjects = resources.get(b"XObject").ok().map(|o| resolve(&self.pdf, o)).and_then(|o| o.as_dict().ok().cloned());
        let content = Content::decode(&self.pdf.get_page_content(id))?;
        let mut out = Vec::new();
        for (index, ctm, name) in self.draws(&content, &xobjects) {
            let unit = transform_box(multiply(ctm, to_display), [0.0, 0.0, 1.0, 1.0]);
            out.push(PagePicture { name, op_index: index, display: unit });
        }
        Ok(out)
    }

    /// Every `Do` of an image, with the transformation in force and the image's name.
    fn draws(&self, content: &Content, xobjects: &Option<lopdf::Dictionary>) -> Vec<(usize, [f32; 6], String)> {
        let (mut ctm, mut stack, mut found) = (IDENTITY, Vec::<[f32; 6]>::new(), Vec::new());
        for (index, op) in content.operations.iter().enumerate() {
            let n = |i: usize| op.operands.get(i).and_then(|o| crate::font::num(&self.pdf, o)).unwrap_or(0.0);
            match op.operator.as_str() {
                "q" => stack.push(ctm),
                "Q" => ctm = stack.pop().unwrap_or(ctm),
                "cm" => ctm = multiply([n(0), n(1), n(2), n(3), n(4), n(5)], ctm),
                "Do" => {
                    let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) else { continue };
                    let is_image = xobjects
                        .as_ref()
                        .and_then(|d| d.get(name).ok())
                        .and_then(|o| resolve(&self.pdf, o).as_stream().ok())
                        .is_some_and(|s| s.dict.get(b"Subtype").and_then(|o| o.as_name()).is_ok_and(|n| n == b"Image"));
                    if is_image {
                        found.push((index, ctm, String::from_utf8_lossy(name).into_owned()));
                    }
                }
                _ => {}
            }
        }
        found
    }

    /// Move pictures by `(dx, dy)` each, as a reader shows the page.
    pub(crate) fn shift_pictures(&mut self, page: usize, moves: &[(PagePicture, (f32, f32))]) -> Result<()> {
        if moves.is_empty() {
            return Ok(());
        }
        let id = self.page_id(page)?;
        // The page may have been rewritten since these were read: find each again by name and place.
        let now = self.page_pictures(page)?;
        let resources = self.resources(id);
        let xobjects = resources.get(b"XObject").ok().map(|o| resolve(&self.pdf, o)).and_then(|o| o.as_dict().ok().cloned());
        let mut content = Content::decode(&self.pdf.get_page_content(id))?;
        let draws = self.draws(&content, &xobjects);
        let mut jobs: Vec<(usize, [f32; 6], (f32, f32))> = Vec::new();
        for (picture, delta) in moves {
            let found = now
                .iter()
                .position(|p| {
                    p.name == picture.name && p.display.iter().zip(picture.display).all(|(a, b)| (a - b).abs() < 0.5)
                })
                .ok_or_else(|| Error::Unsupported("la imagen ya no está donde se leyó: vuelve a leer la página".into()))?;
            let (index, ctm, _) = draws.iter().find(|(i, _, _)| *i == now[found].op_index).cloned().expect("listed above");
            jobs.push((index, ctm, self.user_delta(page, delta.0, delta.1)?));
        }
        // Last first, so earlier operation numbers stay where they were.
        jobs.sort_by_key(|j| std::cmp::Reverse(j.0));
        for (index, ctm, (ux, uy)) in jobs {
            let inverse = invert(ctm).ok_or_else(|| Error::Unsupported("la imagen tiene una transformación no invertible".into()))?;
            // Final = local x ctm; moved final = local x ctm x translate(u); the `cm` that does it is ctm x translate(u) x ctm^-1.
            let t = multiply(multiply(ctm, [1.0, 0.0, 0.0, 1.0, ux, uy]), inverse);
            let draw = content.operations[index].clone();
            content.operations.splice(
                index..=index,
                [
                    Operation::new("q", vec![]),
                    Operation::new("cm", t.iter().map(|v| Object::Real(*v)).collect()),
                    draw,
                    Operation::new("Q", vec![]),
                ],
            );
        }
        self.set_page_content(id, &content)
    }

    /// Move text runs by `(dx, dy)` each (as a reader shows the page) and keep every
    /// other run on the page exactly where it was.
    pub(crate) fn shift_runs(&mut self, page: usize, moves: &[(TextRun, (f32, f32))]) -> Result<()> {
        if moves.is_empty() {
            return Ok(());
        }
        let id = self.page_id(page)?;
        let mut content = Content::decode(&self.pdf.get_page_content(id))?;
        let before = self.text_runs(page)?;
        // What each run to move should end up at, in the page's own space.
        let mut wanted: Vec<(usize, (f32, f32))> = Vec::new();
        for (run, (dx, dy)) in moves {
            if run.xobject.is_some() {
                return Err(Error::Unsupported("mover texto dentro de un formulario XObject aún no está soportado".into()));
            }
            let old = before
                .iter()
                .find(|r| r.op_index == run.op_index && r.xobject.is_none())
                .ok_or_else(|| Error::Unsupported("el texto ya no está donde se leyó: vuelve a leer la página".into()))?;
            let (ux, uy) = self.user_delta(page, *dx, *dy)?;
            wanted.push((old.op_index, (old.origin.0 + ux, old.origin.1 + uy)));
        }
        let first = wanted.iter().map(|w| w.0).min().expect("not empty");
        // Everything after the first change stays where it is.
        let mut items = wanted.clone();
        for r in before.iter().filter(|r| r.xobject.is_none() && r.op_index > first && !wanted.iter().any(|w| w.0 == r.op_index)) {
            items.push((r.op_index, r.origin));
        }
        items.sort_by_key(|i| i.0);

        let original = content.operations.clone();
        let (mut corrections, mut i) = (0usize, 0usize);
        let mut runs = self.runs_in(page, id, &content)?;
        while i < items.len() {
            let Some(run) = runs.iter().find(|x| x.op_index == items[i].0 && x.xobject.is_none()) else {
                i += 1;
                continue;
            };
            let error = (items[i].1 .0 - run.origin.0, items[i].1 .1 - run.origin.1);
            if (error.0 * error.0 + error.1 * error.1).sqrt() <= EPSILON {
                i += 1;
                continue;
            }
            let [a, b, c, d] = run.text_linear;
            let det = a * d - b * c;
            if det.abs() < 1e-9 || corrections >= MAX_CORRECTIONS {
                content.operations = original;
                return Err(Error::Unsupported("no se pudo mover ese texto sin descolocar el resto de la página".into()));
            }
            let (tx, ty) = ((error.0 * d - error.1 * c) / det, (-error.0 * b + error.1 * a) / det);
            content.operations.insert(items[i].0, Operation::new("Td", vec![Object::Real(tx), Object::Real(ty)]));
            corrections += 1;
            for t in &mut items[i..] {
                t.0 += 1;
            }
            // Only a changed page needs reading again.
            runs = self.runs_in(page, id, &content)?;
        }
        self.set_page_content(id, &content)
    }
}
