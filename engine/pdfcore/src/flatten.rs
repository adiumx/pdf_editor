//! Flattening: draw annotations and form fields into the page itself and remove them, so what
//! they show can no longer be changed (the copy to send, to print, to archive).

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId};

use crate::error::{Error, Result};
use crate::font::resolve;
use crate::forms::FieldKind;
use crate::text::transform_box;
use crate::Document;

/// What to flatten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlattenOptions {
    /// Highlights, notes, free text, drawings... (links are kept: they are navigation).
    pub annotations: bool,
    /// Form fields. Signature fields are left alone.
    pub forms: bool,
}

impl Default for FlattenOptions {
    fn default() -> Self {
        FlattenOptions { annotations: true, forms: true }
    }
}

/// What [`Document::flatten`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flattened {
    /// Marks now drawn into their pages.
    pub drawn: usize,
    /// Annotations taken off the pages (the drawn ones, hidden ones, pop-ups).
    pub removed: usize,
    /// Left as they were: no drawing to take (an empty field), signatures.
    pub skipped: usize,
}

impl Document {
    /// Draw the annotations and fields of `pages` (all when `None`) into their pages and remove
    /// them. One step. A signed document stops being valid when flattened; check
    /// [`signatures`](Self::signatures) first.
    pub fn flatten(&mut self, pages: Option<&[usize]>, options: FlattenOptions) -> Result<Flattened> {
        let all: Vec<usize> = (0..self.page_count()).collect();
        let wanted: Vec<usize> = pages.map(<[usize]>::to_vec).unwrap_or(all);
        if let Some(bad) = wanted.iter().find(|p| **p >= self.page_count()) {
            return Err(Error::NoSuchPage(*bad));
        }
        self.transaction(|d| {
            if options.forms {
                d.refresh_appearances(true)?;
            }
            let mut done = Flattened::default();
            for page in wanted {
                d.flatten_page(page, options, &mut done)?;
            }
            Ok(done)
        })
    }

    fn flatten_page(&mut self, page: usize, options: FlattenOptions, done: &mut Flattened) -> Result<()> {
        let id = self.page_id(page)?;
        let widgets: Vec<(ObjectId, String, usize, FieldKind)> = self
            .fields()
            .into_iter()
            .flat_map(|f| f.widgets.iter().enumerate().map(|(i, w)| (w.id(), f.name.clone(), i, f.kind)).collect::<Vec<_>>())
            .collect();
        let mut ops: Vec<Operation> = Vec::new();
        let mut to_remove: Vec<ObjectId> = Vec::new();
        let mut widget_removals: Vec<(String, usize)> = Vec::new();
        for (_, object) in self.annotation_refs(id) {
            let Ok(dict) = self.pdf.get_dictionary(object) else { continue };
            let subtype = dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()).map(|n| String::from_utf8_lossy(n).into_owned()).unwrap_or_default();
            let widget = widgets.iter().find(|w| w.0 == object);
            let wanted = match (subtype.as_str(), widget) {
                ("Link", _) => false,
                ("Popup", _) => options.annotations,
                (_, Some((_, _, _, FieldKind::Signature))) => {
                    done.skipped += 1;
                    false
                }
                ("Widget", _) => options.forms,
                _ => options.annotations,
            };
            if !wanted {
                continue;
            }
            let flags = dict.get(b"F").ok().and_then(|o| o.as_i64().ok()).unwrap_or(0);
            let rect: Vec<f32> = dict
                .get(b"Rect")
                .ok()
                .map(|o| resolve(&self.pdf, o))
                .and_then(|o| o.as_array().ok())
                .map(|a| a.iter().filter_map(|n| crate::font::num(&self.pdf, n)).collect())
                .unwrap_or_default();
            let drawing = if subtype == "Popup" || flags & (2 | 32) != 0 || rect.len() != 4 { None } else { self.normal_appearance(dict) };
            match drawing {
                Some(stream) => {
                    let rect = [rect[0].min(rect[2]), rect[1].min(rect[3]), rect[0].max(rect[2]), rect[1].max(rect[3])];
                    let form = self.pdf.get_object(stream).ok().and_then(|o| o.as_stream().ok()).map(|s| s.dict.clone()).unwrap_or_default();
                    let numbers = |key: &[u8]| -> Vec<f32> {
                        form.get(key)
                            .ok()
                            .map(|o| resolve(&self.pdf, o))
                            .and_then(|o| o.as_array().ok())
                            .map(|a| a.iter().filter_map(|n| crate::font::num(&self.pdf, n)).collect())
                            .unwrap_or_default()
                    };
                    let bbox = match numbers(b"BBox").as_slice() {
                        [a, b, c, d] => [*a, *b, *c, *d],
                        _ => rect,
                    };
                    let matrix = match numbers(b"Matrix").as_slice() {
                        [a, b, c, d, e, f] => [*a, *b, *c, *d, *e, *f],
                        _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                    };
                    let t = transform_box(matrix, bbox);
                    let (sx, sy) = ((rect[2] - rect[0]) / (t[2] - t[0]).max(1e-6), (rect[3] - rect[1]) / (t[3] - t[1]).max(1e-6));
                    let a = [sx, 0.0, 0.0, sy, rect[0] - t[0] * sx, rect[1] - t[1] * sy];
                    let name = format!("PcFl{}_{}", id.0, object.0);
                    self.add_xobject_resource(id, &name, stream)?;
                    ops.push(Operation::new("q", vec![]));
                    ops.push(Operation::new("cm", a.iter().map(|v| Object::Real(*v)).collect()));
                    ops.push(Operation::new("Do", vec![Object::Name(name.into_bytes())]));
                    ops.push(Operation::new("Q", vec![]));
                    done.drawn += 1;
                }
                None if subtype == "Widget" && flags & (2 | 32) == 0 => {
                    // A field with nothing to show (an empty one) just goes.
                }
                None => {}
            }
            done.removed += 1;
            match widget {
                Some((_, name, index, _)) => widget_removals.push((name.clone(), *index)),
                None => to_remove.push(object),
            }
        }
        if !ops.is_empty() {
            self.append_to_page(id, &Content { operations: ops })?;
        }
        for object in to_remove {
            self.remove_reference(id, b"Annots", object)?;
        }
        // Several widgets of one field go from the last, so the others keep their numbers.
        widget_removals.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        for (name, index) in widget_removals {
            self.remove_widget(&name, index)?;
        }
        Ok(())
    }

    /// The stream that is the annotation's normal look (the state it shows, if there are several).
    fn normal_appearance(&self, annot: &Dictionary) -> Option<ObjectId> {
        let ap = resolve(&self.pdf, annot.get(b"AP").ok()?).as_dict().ok()?;
        let normal = ap.get(b"N").ok()?;
        let reference = normal.as_reference().ok();
        match reference.map(|r| (r, self.pdf.get_object(r).ok())) {
            Some((r, Some(Object::Stream(_)))) => Some(r),
            _ => {
                let states = resolve(&self.pdf, normal).as_dict().ok()?;
                let state = annot.get(b"AS").ok()?.as_name().ok()?;
                states.get(state).ok()?.as_reference().ok().filter(|r| matches!(self.pdf.get_object(*r), Ok(Object::Stream(_))))
            }
        }
    }

    /// Make a form XObject usable by name in one page's content.
    pub(crate) fn add_xobject_resource(&mut self, page: ObjectId, resource: &str, xobject: ObjectId) -> Result<()> {
        let mut resources = self.resources(page);
        let mut table = resources
            .get(b"XObject")
            .ok()
            .map(|o| resolve(&self.pdf, o))
            .and_then(|o| o.as_dict().ok().cloned())
            .unwrap_or_default();
        table.set(resource, Object::Reference(xobject));
        resources.set("XObject", Object::Dictionary(table));
        self.pdf.get_dictionary_mut(page)?.set("Resources", Object::Dictionary(resources));
        Ok(())
    }
}
