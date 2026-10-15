//! Moving, resizing and aligning form fields.
//!
//! Boxes are in the page as a reader shows it (origin top-left, y down), the
//! same as every box the engine returns, so what a finger or a grid decides on
//! screen goes in unchanged. A field's drawing is built again at its new size
//! (text is not stretched), and aligning several fields is one step to undo.

use lopdf::Object;

use crate::error::{Error, Result};
use crate::forms::{Field, FieldKind};
use crate::text::{invert, transform_box};
use crate::Document;

/// Smallest a field may be made, in points.
const MIN_SIDE: f32 = 6.0;

/// How [`Document::arrange_fields`] lines fields up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrange {
    AlignLeft,
    AlignRight,
    AlignTop,
    AlignBottom,
    /// Centres share a vertical line.
    CenterHorizontally,
    /// Centres share a horizontal line.
    CenterVertically,
    /// Equal gaps between fields from left to right (three or more).
    DistributeHorizontally,
    /// Equal gaps from top to bottom (three or more).
    DistributeVertically,
    SameWidth,
    SameHeight,
}

/// One widget of one field, as `Document::fields` names them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldRef {
    pub name: String,
    /// Index in the field's `widgets` (a field repeated on several places has more than one).
    pub widget: usize,
}

impl Document {
    /// Put widget `widget` of the field called `name` in `display` (`[x0, y0,
    /// x1, y1]`). The box is kept inside the page and at least a few points
    /// each way; what it ended up as is returned. A signed signature field
    /// cannot be moved.
    pub fn move_field(&mut self, name: &str, widget: usize, display: [f32; 4]) -> Result<[f32; 4]> {
        self.transaction(|d| d.place_field(name, widget, display))
    }

    fn place_field(&mut self, name: &str, widget: usize, display: [f32; 4]) -> Result<[f32; 4]> {
        let field = self.field(name).ok_or_else(|| Error::NoSuchField(name.to_string()))?;
        if field.signed {
            return Err(Error::Rejected(format!("«{name}» es una firma ya firmada: moverla la invalida")));
        }
        let target = field
            .widgets
            .get(widget)
            .ok_or_else(|| Error::Rejected(format!("«{name}» no tiene un widget número {widget}")))?
            .clone();
        let page = target.page.ok_or_else(|| Error::Unsupported(format!("«{name}» no está en ninguna página")))?;
        let size = self.page_size(page)?;
        let clamped = clamp(display, size.width, size.height);

        let to_user = invert(self.display_matrix(page)?)
            .ok_or_else(|| Error::Unsupported("la página tiene una transformación no invertible".into()))?;
        let rect = transform_box(to_user, clamped);
        self.pdf
            .get_dictionary_mut(target.id())?
            .set("Rect", Object::Array(rect.iter().map(|v| Object::Real(*v)).collect()));

        // Draw the field again at its new size, with the value it holds.
        let fresh = self.field(name).ok_or_else(|| Error::NoSuchField(name.to_string()))?;
        self.redraw(&fresh)?;
        Ok(clamped)
    }

    fn redraw(&mut self, field: &Field) -> Result<()> {
        match field.kind {
            FieldKind::Text | FieldKind::Combo => {
                // Nothing typed, nothing to redraw: the frame of an empty box stretches fine.
                if !field.value.is_empty() {
                    self.fill_text(field, &field.value.clone())?;
                }
            }
            FieldKind::List => {
                let chosen: Vec<usize> = field
                    .value
                    .split('\n')
                    .filter(|v| !v.is_empty())
                    .filter_map(|v| field.options.iter().position(|(e, l)| e == v || l == v))
                    .collect();
                self.fill_list(field, &chosen)?;
            }
            // A tick or a dot scales with its box; a button or a signature has nothing to redraw.
            FieldKind::Checkbox | FieldKind::Radio | FieldKind::PushButton | FieldKind::Signature => {}
        }
        Ok(())
    }

    /// Line several fields up, or space them evenly, as one step. Needs at
    /// least two (three to distribute), all on the same page.
    pub fn arrange_fields(&mut self, items: &[FieldRef], how: Arrange) -> Result<()> {
        let distributing = matches!(how, Arrange::DistributeHorizontally | Arrange::DistributeVertically);
        if items.len() < if distributing { 3 } else { 2 } {
            return Err(Error::Rejected(
                if distributing { "para repartir hacen falta al menos tres campos" } else { "hacen falta al menos dos campos" }.into(),
            ));
        }
        let mut boxes: Vec<[f32; 4]> = Vec::new();
        let mut page = None;
        for item in items {
            let field = self.field(&item.name).ok_or_else(|| Error::NoSuchField(item.name.clone()))?;
            let widget = field
                .widgets
                .get(item.widget)
                .ok_or_else(|| Error::Rejected(format!("«{}» no tiene un widget número {}", item.name, item.widget)))?;
            if page.is_some_and(|p| Some(p) != widget.page) {
                return Err(Error::Rejected("los campos están en páginas distintas".into()));
            }
            page = widget.page;
            boxes.push(widget.display);
        }
        let targets = arranged(&boxes, how);
        self.transaction(|d| {
            for (item, target) in items.iter().zip(targets) {
                d.place_field(&item.name, item.widget, target)?;
            }
            Ok(())
        })
    }
}

fn clamp(b: [f32; 4], width: f32, height: f32) -> [f32; 4] {
    let (w, h) = ((b[2] - b[0]).abs().max(MIN_SIDE).min(width), (b[3] - b[1]).abs().max(MIN_SIDE).min(height));
    let x0 = b[0].min(b[2]).clamp(0.0, (width - w).max(0.0));
    let y0 = b[1].min(b[3]).clamp(0.0, (height - h).max(0.0));
    [x0, y0, x0 + w, y0 + h]
}

/// Where each box goes for `how`.
fn arranged(boxes: &[[f32; 4]], how: Arrange) -> Vec<[f32; 4]> {
    let left = boxes.iter().map(|b| b[0]).fold(f32::MAX, f32::min);
    let right = boxes.iter().map(|b| b[2]).fold(f32::MIN, f32::max);
    let top = boxes.iter().map(|b| b[1]).fold(f32::MAX, f32::min);
    let bottom = boxes.iter().map(|b| b[3]).fold(f32::MIN, f32::max);
    let (cx, cy) = ((left + right) / 2.0, (top + bottom) / 2.0);
    let shift = |b: &[f32; 4], dx: f32, dy: f32| [b[0] + dx, b[1] + dy, b[2] + dx, b[3] + dy];
    match how {
        Arrange::AlignLeft => boxes.iter().map(|b| shift(b, left - b[0], 0.0)).collect(),
        Arrange::AlignRight => boxes.iter().map(|b| shift(b, right - b[2], 0.0)).collect(),
        Arrange::AlignTop => boxes.iter().map(|b| shift(b, 0.0, top - b[1])).collect(),
        Arrange::AlignBottom => boxes.iter().map(|b| shift(b, 0.0, bottom - b[3])).collect(),
        Arrange::CenterHorizontally => boxes.iter().map(|b| shift(b, cx - (b[0] + b[2]) / 2.0, 0.0)).collect(),
        Arrange::CenterVertically => boxes.iter().map(|b| shift(b, 0.0, cy - (b[1] + b[3]) / 2.0)).collect(),
        Arrange::SameWidth => {
            let w = boxes.iter().map(|b| b[2] - b[0]).fold(0.0, f32::max);
            boxes.iter().map(|b| [b[0], b[1], b[0] + w, b[3]]).collect()
        }
        Arrange::SameHeight => {
            let h = boxes.iter().map(|b| b[3] - b[1]).fold(0.0, f32::max);
            boxes.iter().map(|b| [b[0], b[1], b[2], b[1] + h]).collect()
        }
        Arrange::DistributeHorizontally | Arrange::DistributeVertically => {
            let horizontal = how == Arrange::DistributeHorizontally;
            let (lo, hi) = if horizontal { (0, 2) } else { (1, 3) };
            let mut order: Vec<usize> = (0..boxes.len()).collect();
            order.sort_by(|&a, &b| boxes[a][lo].total_cmp(&boxes[b][lo]).then(a.cmp(&b)));
            let span = boxes[*order.last().unwrap()][hi] - boxes[order[0]][lo];
            let used: f32 = boxes.iter().map(|b| b[hi] - b[lo]).sum();
            let gap = (span - used) / (boxes.len() - 1) as f32;
            let mut out = boxes.to_vec();
            let mut at = boxes[order[0]][lo];
            for &i in &order {
                let delta = at - boxes[i][lo];
                out[i] = if horizontal { shift(&boxes[i], delta, 0.0) } else { shift(&boxes[i], 0.0, delta) };
                at += boxes[i][hi] - boxes[i][lo] + gap;
            }
            out
        }
    }
}
