//! The things on a page a person moves around as units, the way a design tool
//! does: blocks of text, pictures and form fields. They can be moved together,
//! lined up, spread out evenly and spaced at an even leading, as one step.

use crate::add::Align;
use crate::error::{Error, Result};
use crate::layout::{arranged, Arrange};
use crate::lines::build_lines;
use crate::paragraph::paragraph_runs;
use crate::placement::PagePicture;
use crate::text::TextRun;
use crate::Document;

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    /// A paragraph, or a line, of text.
    Text,
    Image,
    Field,
}

/// One movable thing. Read again after any edit: like text runs, items go stale.
#[derive(Debug, Clone)]
pub struct Item {
    pub kind: ItemKind,
    /// `[x0, y0, x1, y1]` as a reader shows the page.
    pub display: [f32; 4],
    /// For text: its text, with lines joined by spaces.
    pub text: String,
    /// For text: the runs it is made of (one per line of a paragraph).
    pub runs: Vec<TextRun>,
    /// For a picture.
    pub picture: Option<PagePicture>,
    /// For a field: its name and which of its widgets.
    pub field: Option<(String, usize)>,
}

fn union(boxes: impl Iterator<Item = [f32; 4]>) -> [f32; 4] {
    boxes.fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, d| [b[0].min(d[0]), b[1].min(d[1]), b[2].max(d[2]), b[3].max(d[3])])
}

impl Document {
    /// What can be moved on a page: text (paragraphs where there are some, otherwise
    /// lines), pictures and form fields, top to bottom.
    pub fn page_items(&self, page: usize) -> Result<Vec<Item>> {
        let runs = self.text_runs(page)?;
        let lines = build_lines(&runs);
        let mut taken = vec![false; runs.len()];
        let mut items: Vec<Item> = Vec::new();
        for line in &lines {
            // Text inside a form XObject cannot be moved, so a line with some is left out.
            if line.invisible || line.runs.iter().any(|&r| taken[r] || runs[r].xobject.is_some()) {
                continue;
            }
            let group: Vec<usize> = match line.runs.first().and_then(|&r| paragraph_runs(&runs, &lines, r)) {
                Some(chain) if chain.iter().all(|&r| !taken[r]) => chain,
                _ => line.runs.clone(),
            };
            for &r in &group {
                taken[r] = true;
            }
            let members: Vec<TextRun> = group.iter().map(|&r| runs[r].clone()).collect();
            items.push(Item {
                kind: ItemKind::Text,
                display: union(members.iter().map(|m| m.display)),
                text: members.iter().map(|m| m.text.trim_end()).collect::<Vec<_>>().join(" "),
                runs: members,
                picture: None,
                field: None,
            });
        }
        for picture in self.page_pictures(page)? {
            items.push(Item { kind: ItemKind::Image, display: picture.display, text: String::new(), runs: Vec::new(), picture: Some(picture), field: None });
        }
        for field in self.fields() {
            for (index, widget) in field.widgets.iter().enumerate().filter(|(_, w)| w.page == Some(page)) {
                items.push(Item {
                    kind: ItemKind::Field,
                    display: widget.display,
                    text: field.value.clone(),
                    runs: Vec::new(),
                    picture: None,
                    field: Some((field.name.clone(), index)),
                });
            }
        }
        items.sort_by(|a, b| a.display[1].total_cmp(&b.display[1]).then(a.display[0].total_cmp(&b.display[0])));
        Ok(items)
    }

    /// Move items by `(dx, dy)` each (points, as a reader shows the page), as one step.
    pub fn move_items(&mut self, page: usize, moves: &[(Item, (f32, f32))]) -> Result<()> {
        self.transaction(|d| d.apply_moves(page, moves, None))
    }

    /// `targets`, when given, are the boxes fields end up in (so they can be resized as well).
    fn apply_moves(&mut self, page: usize, moves: &[(Item, (f32, f32))], targets: Option<&[[f32; 4]]>) -> Result<()> {
        let mut text: Vec<(TextRun, (f32, f32))> = Vec::new();
        let mut pictures: Vec<(PagePicture, (f32, f32))> = Vec::new();
        for (n, (item, delta)) in moves.iter().enumerate() {
            match item.kind {
                ItemKind::Text => text.extend(item.runs.iter().cloned().map(|r| (r, *delta))),
                ItemKind::Image => pictures.push((item.picture.clone().expect("a picture item has its picture"), *delta)),
                ItemKind::Field => {
                    let (name, widget) = item.field.clone().expect("a field item has its field");
                    let target = match targets {
                        Some(t) => t[n],
                        None => [item.display[0] + delta.0, item.display[1] + delta.1, item.display[2] + delta.0, item.display[3] + delta.1],
                    };
                    self.place_field(&name, widget, target)?;
                }
            }
        }
        self.shift_runs(page, &text)?;
        self.shift_pictures(page, &pictures)
    }

    /// Line items up or space them evenly, as one step.
    pub fn arrange_items(&mut self, page: usize, items: &[Item], how: Arrange) -> Result<()> {
        let distributing = matches!(how, Arrange::DistributeHorizontally | Arrange::DistributeVertically);
        if items.len() < if distributing { 3 } else { 2 } {
            return Err(Error::Rejected(
                if distributing { "para repartir hacen falta al menos tres elementos" } else { "hacen falta al menos dos elementos" }.into(),
            ));
        }
        let resizing = matches!(how, Arrange::SameWidth | Arrange::SameHeight);
        if resizing && items.iter().any(|i| i.kind != ItemKind::Field) {
            return Err(Error::Rejected("igualar el tamaño solo se puede con campos de formulario".into()));
        }
        let boxes: Vec<[f32; 4]> = items.iter().map(|i| i.display).collect();
        let targets = arranged(&boxes, how);
        let moves: Vec<(Item, (f32, f32))> = items
            .iter()
            .cloned()
            .zip(&targets)
            .zip(&boxes)
            .map(|((item, t), b)| (item, (t[0] - b[0], t[1] - b[1])))
            .collect();
        self.transaction(|d| d.apply_moves(page, &moves, Some(&targets)))
    }

    /// Give text items an even leading: the first stays, and each next baseline
    /// is `leading` points below the one before (the average of the gaps there
    /// are now when `leading` is `None`).
    pub fn space_lines(&mut self, page: usize, items: &[Item], leading: Option<f32>) -> Result<()> {
        if items.len() < 2 || items.iter().any(|i| i.kind != ItemKind::Text || i.runs.is_empty()) {
            return Err(Error::Rejected("el interlineado necesita al menos dos textos".into()));
        }
        let baseline = |i: &Item| -> Result<f32> { Ok(self.to_display_point(page, i.runs[0].origin)?.1) };
        let mut order: Vec<(f32, usize)> = Vec::new();
        for (n, item) in items.iter().enumerate() {
            order.push((baseline(item)?, n));
        }
        order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let leading = leading.unwrap_or_else(|| (order[order.len() - 1].0 - order[0].0) / (order.len() - 1) as f32);
        if !(leading > 0.0) {
            return Err(Error::Rejected("el interlineado debe ser mayor que cero".into()));
        }
        let first = order[0].0;
        let moves: Vec<(Item, (f32, f32))> =
            order.iter().enumerate().map(|(rank, (at, n))| (items[*n].clone(), (0.0, first + leading * rank as f32 - at))).collect();
        self.transaction(|d| d.apply_moves(page, &moves, None))
    }

    /// Align the lines of a paragraph to its left edge, centre or right edge.
    pub fn align_paragraph(&mut self, page: usize, run: &TextRun, align: Align) -> Result<()> {
        let paragraph = self
            .paragraph_of(page, run)?
            .ok_or_else(|| Error::Unsupported("ese texto no es un párrafo de varias líneas".into()))?;
        let runs = self.text_runs(page)?;
        let members: Vec<TextRun> = paragraph.runs.iter().map(|&i| runs[i].clone()).collect();
        let [left, _, right, _] = paragraph.display;
        let moves: Vec<(TextRun, (f32, f32))> = members
            .into_iter()
            .map(|m| {
                let d = m.display;
                let dx = match align {
                    Align::Left => left - d[0],
                    Align::Right => right - d[2],
                    Align::Center => (left + right) / 2.0 - (d[0] + d[2]) / 2.0,
                };
                (m, (dx, 0.0))
            })
            .collect();
        self.transaction(|d| d.shift_runs(page, &moves))
    }
}
