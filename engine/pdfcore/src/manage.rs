//! Things a person does to what is on a page: delete it, click on it, resize it, and rebuild
//! the look of a form. Each is one undo step (or part of a batch).

use lopdf::content::{Content, Operation};
use lopdf::{Object, ObjectId};

use crate::block::{Block, BlockOptions};
use crate::error::{Error, Result};
use crate::font::resolve;
use crate::forms::FieldKind;
use crate::items::{Item, ItemKind};
use crate::placement::PagePicture;
use crate::text::{invert, multiply, TextRun};
use crate::Document;

/// What is under a point of the page.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub kind: ItemKind,
    /// Index in [`Document::page_items`] of the page.
    pub item: usize,
    /// For text: index in [`Document::blocks`], the character under the point, and the caret
    /// position (between letters, `0..=len`) nearest to it.
    pub block: Option<usize>,
    pub char_index: Option<usize>,
    pub caret: Option<usize>,
    pub display: [f32; 4],
    /// For a field: its name and which of its widgets.
    pub field: Option<(String, usize)>,
}

/// What [`Document::refresh_appearances`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Refreshed {
    /// Fields whose look was built again.
    pub rebuilt: usize,
    /// Fields left alone (they already had a look, or are signatures, push buttons...).
    pub skipped: usize,
}

fn contains(b: [f32; 4], x: f32, y: f32) -> bool {
    x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3]
}

fn area(b: [f32; 4]) -> f32 {
    (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0)
}

impl Document {
    /// The thing on top at a point of the page (as a reader shows it): a field over text over a
    /// picture, the smallest of those that overlap. `None` for empty page.
    pub fn hit_test(&self, page: usize, x: f32, y: f32) -> Result<Option<Hit>> {
        let items = self.page_items(page)?;
        let blocks = self.blocks(page)?;
        let rank = |k: &ItemKind| match k {
            ItemKind::Field => 0,
            ItemKind::Text => 1,
            ItemKind::Image => 2,
        };
        let best = items
            .iter()
            .enumerate()
            .filter(|(_, it)| contains(it.display, x, y))
            .min_by(|a, b| rank(&a.1.kind).cmp(&rank(&b.1.kind)).then(area(a.1.display).total_cmp(&area(b.1.display))));
        let Some((index, item)) = best else { return Ok(None) };
        let mut hit = Hit { kind: item.kind.clone(), item: index, block: None, char_index: None, caret: None, display: item.display, field: item.field.clone() };
        if item.kind == ItemKind::Text {
            let found = blocks.iter().position(|b| item.runs.iter().any(|r| b.runs.iter().any(|m| m.op_index == r.op_index && m.xobject == r.xobject)));
            if let Some(bi) = found {
                let b = &blocks[bi];
                // The letter nearest to the point, preferring the line it is on.
                let nearest = b
                    .char_boxes
                    .iter()
                    .enumerate()
                                        .map(|(i, bx)| {
                        let dy = if y < bx[1] { bx[1] - y } else if y > bx[3] { y - bx[3] } else { 0.0 };
                        let dx = if x < bx[0] { bx[0] - x } else if x > bx[2] { x - bx[2] } else { 0.0 };
                        (i, dy * 4.0 + dx)
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((i, _)) = nearest {
                    let bx = b.char_boxes[i];
                    hit.block = Some(bi);
                    hit.char_index = Some(i);
                    hit.caret = Some(if x > (bx[0] + bx[2]) / 2.0 { i + 1 } else { i });
                }
            }
        }
        Ok(Some(hit))
    }

    /// Take items off the page: text stops drawing, a picture's drawing is cancelled, a field's
    /// widget is removed from the page and the form (the field with its last widget). One step.
    pub fn delete_items(&mut self, page: usize, items: &[Item]) -> Result<()> {
        self.transaction(|d| d.remove_items(page, items))
    }

    fn remove_items(&mut self, page: usize, items: &[Item]) -> Result<()> {
        let id = self.page_id(page)?;
        let mut runs: Vec<&TextRun> = Vec::new();
        let mut pictures: Vec<&PagePicture> = Vec::new();
        for item in items {
            match item.kind {
                ItemKind::Text => runs.extend(item.runs.iter()),
                ItemKind::Image => pictures.extend(item.picture.iter()),
                ItemKind::Field => {}
            }
        }
        if runs.iter().any(|r| r.xobject.is_some()) {
            return Err(Error::Unsupported("borrar texto dentro de un formulario XObject aún no está soportado".into()));
        }
        // Pictures first: cancelling a drawing changes no operation numbers, blanking text neither.
        self.cancel_pictures(id, page, &pictures)?;
        self.blank_runs(id, &runs)?;
        for item in items.iter().filter(|i| i.kind == ItemKind::Field) {
            let (name, widget) = item.field.clone().ok_or_else(|| Error::Unsupported("el campo ya no está donde se leyó".into()))?;
            self.remove_widget(&name, widget)?;
        }
        Ok(())
    }

    /// Make the `Do` of these pictures draw nothing (a path-ending no-op takes its place, so no
    /// operation changes number).
    fn cancel_pictures(&mut self, id: ObjectId, page: usize, pictures: &[&PagePicture]) -> Result<()> {
        if pictures.is_empty() {
            return Ok(());
        }
        let now = self.page_pictures(page)?;
        let mut content = Content::decode(&self.pdf.get_page_content(id))?;
        for picture in pictures {
            let found = now
                .iter()
                .find(|p| p.name == picture.name && p.display.iter().zip(picture.display).all(|(a, b)| (a - b).abs() < 0.5))
                .ok_or_else(|| Error::Unsupported("la imagen ya no está donde se leyó: vuelve a leer la página".into()))?;
            content.operations[found.op_index] = Operation::new("n", vec![]);
        }
        self.set_page_content(id, &content)
    }

    /// Remove one widget of a field from its page and from the form.
    fn remove_widget(&mut self, name: &str, widget: usize) -> Result<()> {
        let field = self.field(name).ok_or_else(|| Error::NoSuchField(name.to_string()))?;
        let w = field.widgets.get(widget).ok_or_else(|| Error::NoSuchField(name.to_string()))?.clone();
        if let Some(page) = w.page {
            let page_id = self.page_id(page)?;
            self.remove_reference(page_id, b"Annots", w.id())?;
        }
        // Up the tree: out of its parent's kids, and the parent too when it has none left.
        let mut node = w.id();
        loop {
            let parent = self.pdf.get_dictionary(node).ok().and_then(|d| d.get(b"Parent").ok()).and_then(|o| o.as_reference().ok());
            match parent {
                None => {
                    self.remove_from_form(node)?;
                    break;
                }
                Some(p) => {
                    self.remove_reference(p, b"Kids", node)?;
                    let left = self.pdf.get_dictionary(p).ok().and_then(|d| d.get(b"Kids").ok()).map(|k| match resolve(&self.pdf, k) {
                        Object::Array(a) => a.len(),
                        _ => 0,
                    });
                    if left == Some(0) {
                        node = p;
                    } else {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    /// Take `target` out of the array `holder[key]` (held directly or through a reference).
    fn remove_reference(&mut self, holder: ObjectId, key: &[u8], target: ObjectId) -> Result<()> {
        let is_target = |o: &Object| o.as_reference().ok() == Some(target);
        match self.pdf.get_dictionary(holder)?.get(key).ok().cloned() {
            Some(Object::Reference(r)) => {
                if let Ok(Object::Array(a)) = self.pdf.get_object_mut(r) {
                    a.retain(|o| !is_target(o));
                }
            }
            Some(Object::Array(_)) => {
                if let Ok(Object::Array(a)) = self.pdf.get_dictionary_mut(holder)?.get_mut(key) {
                    a.retain(|o| !is_target(o));
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn remove_from_form(&mut self, field: ObjectId) -> Result<()> {
        let root = self.pdf.trailer.get(b"Root")?.as_reference()?;
        match self.pdf.get_dictionary(root)?.get(b"AcroForm").ok().cloned() {
            Some(Object::Reference(form)) => self.remove_reference(form, b"Fields", field),
            Some(Object::Dictionary(_)) => {
                // The form is inside the catalog itself.
                let fields = self.pdf.get_dictionary(root)?.get(b"AcroForm").ok().and_then(|f| f.as_dict().ok()).and_then(|d| d.get(b"Fields").ok()).cloned();
                match fields {
                    Some(Object::Reference(r)) => {
                        if let Ok(Object::Array(a)) = self.pdf.get_object_mut(r) {
                            a.retain(|o| o.as_reference().ok() != Some(field));
                        }
                    }
                    Some(Object::Array(_)) => {
                        if let Ok(Object::Dictionary(form)) = self.pdf.get_dictionary_mut(root)?.get_mut(b"AcroForm") {
                            if let Ok(Object::Array(a)) = form.get_mut(b"Fields") {
                                a.retain(|o| o.as_reference().ok() != Some(field));
                            }
                        }
                    }
                    _ => {}
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Resize an item to `display` (`[x0, y0, x1, y1]` as a reader shows the page): a picture is
    /// scaled and moved, a field gets the new box (its look is built again), text is laid out
    /// again within the new width and moved to the new corner (its height follows from the
    /// text). One step.
    pub fn resize_item(&mut self, page: usize, item: &Item, display: [f32; 4]) -> Result<()> {
        if display[2] - display[0] < 1.0 || display[3] - display[1] < 1.0 {
            return Err(Error::Rejected("el tamaño nuevo es demasiado pequeño".into()));
        }
        self.transaction(|d| d.apply_resize(page, item, display))
    }

    fn apply_resize(&mut self, page: usize, item: &Item, display: [f32; 4]) -> Result<()> {
        match item.kind {
            ItemKind::Field => {
                let (name, widget) = item.field.clone().ok_or_else(|| Error::Unsupported("el campo ya no está donde se leyó".into()))?;
                self.place_field(&name, widget, display).map(|_| ())
            }
            ItemKind::Image => {
                let picture = item.picture.clone().ok_or_else(|| Error::Unsupported("la imagen ya no está donde se leyó".into()))?;
                self.reshape_picture(page, &picture, display)
            }
            ItemKind::Text => {
                let blocks = self.blocks(page)?;
                let block: Block = blocks
                    .into_iter()
                    .find(|b| item.runs.iter().any(|r| b.runs.iter().any(|m| m.op_index == r.op_index && m.xobject == r.xobject)))
                    .ok_or_else(|| Error::Unsupported("el texto ya no está donde se leyó: vuelve a leer la página".into()))?;
                let opts = BlockOptions { width: Some(display[2] - display[0]), allow_fallback: true, ..Default::default() };
                let (top_left_x, top_left_y) = (block.display[0], block.display[1]);
                if (display[2] - display[0] - (block.display[2] - block.display[0])).abs() > 0.5 {
                    self.replace_block_with(page, &block, &block.text.clone(), opts)?;
                }
                let (dx, dy) = (display[0] - top_left_x, display[1] - top_left_y);
                if dx.abs() > 0.01 || dy.abs() > 0.01 {
                    let now = self.page_items(page)?;
                    let moved = now
                        .into_iter()
                        .find(|it| it.kind == ItemKind::Text && (it.display[0] - top_left_x).abs() < 1.0 && (it.display[1] - top_left_y).abs() < 1.0)
                        .ok_or_else(|| Error::Unsupported("no se pudo volver a encontrar el texto".into()))?;
                    self.apply_moves(page, &[(moved, (dx, dy))], None)?;
                }
                Ok(())
            }
        }
    }

    /// Scale and move a picture so that it fills `new` (display coordinates).
    fn reshape_picture(&mut self, page: usize, picture: &PagePicture, new: [f32; 4]) -> Result<()> {
        let id = self.page_id(page)?;
        let now = self.page_pictures(page)?;
        let found = now
            .iter()
            .find(|p| p.name == picture.name && p.display.iter().zip(picture.display).all(|(a, b)| (a - b).abs() < 0.5))
            .ok_or_else(|| Error::Unsupported("la imagen ya no está donde se leyó: vuelve a leer la página".into()))?
            .clone();
        let resources = self.resources(id);
        let xobjects = resources.get(b"XObject").ok().map(|o| resolve(&self.pdf, o)).and_then(|o| o.as_dict().ok().cloned());
        let mut content = Content::decode(&self.pdf.get_page_content(id))?;
        let (index, ctm) = self
            .picture_matrix(&content, &xobjects, found.op_index)
            .ok_or_else(|| Error::Unsupported("la imagen ya no está donde se leyó".into()))?;
        // Display space: x' = (x - x0) * sx + nx0, the same for y.
        let (ow, oh) = (found.display[2] - found.display[0], found.display[3] - found.display[1]);
        let (sx, sy) = ((new[2] - new[0]) / ow.max(0.01), (new[3] - new[1]) / oh.max(0.01));
        let d = [sx, 0.0, 0.0, sy, new[0] - found.display[0] * sx, new[1] - found.display[1] * sy];
        let to_display = self.display_matrix(page)?;
        let back = invert(to_display).ok_or_else(|| Error::Unsupported("la página tiene una transformación no invertible".into()))?;
        // The same change in the page's own space, then as a `cm` placed just before the drawing.
        let user = multiply(multiply(to_display, d), back);
        let inverse = invert(ctm).ok_or_else(|| Error::Unsupported("la imagen tiene una transformación no invertible".into()))?;
        let t = multiply(multiply(ctm, user), inverse);
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
        self.set_page_content(id, &content)
    }

    /// Build the look (`/AP`) of the fields again from their values. With `only_missing`, just the
    /// fields with a widget that has no look: a reader that does not build its own shows those
    /// as empty boxes. Check boxes and radio buttons get the states they lack.
    pub fn refresh_appearances(&mut self, only_missing: bool) -> Result<Refreshed> {
        self.transaction(|d| {
            let mut done = Refreshed::default();
            for field in d.fields() {
                let lacking = field.widgets.iter().any(|w| !w.has_appearance);
                if (only_missing && !lacking) || field.widgets.is_empty() {
                    done.skipped += 1;
                    continue;
                }
                match field.kind {
                    FieldKind::Text | FieldKind::Combo => d.fill_text(&field, &field.value)?,
                    FieldKind::List => {
                        let chosen: Vec<usize> = field
                            .value
                            .split('\n')
                            .filter(|v| !v.is_empty())
                            .filter_map(|v| field.options.iter().position(|(e, l)| e == v || l == v))
                            .collect();
                        d.fill_list(&field, &chosen)?;
                    }
                    FieldKind::Checkbox | FieldKind::Radio => d.rebuild_button(&field)?,
                    FieldKind::PushButton | FieldKind::Signature => {
                        done.skipped += 1;
                        continue;
                    }
                }
                done.rebuilt += 1;
            }
            Ok(done)
        })
    }
}
