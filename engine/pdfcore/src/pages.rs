//! Page operations: rotate, delete, reorder, extract and bring in pages from
//! another document.
//!
//! The page tree is rewritten flat (one `/Pages` node holding every page) after
//! copying down what pages inherit (resources, boxes, rotation), so a page
//! keeps looking the same wherever it ends up. Every change goes through the
//! tracked table, so each operation is one step `undo` takes back.

use lopdf::{Dictionary, Object, ObjectId};

use crate::error::{Error, Result};
use crate::font::resolve;
use crate::Document;

/// What a page takes from its ancestors when it has no value of its own.
const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

impl Document {
    fn pages_root(&self) -> Result<ObjectId> {
        let root = self.pdf.trailer.get(b"Root")?.as_reference()?;
        Ok(self.pdf.get_dictionary(root)?.get(b"Pages")?.as_reference()?)
    }

    /// The value an ancestor gives a page for `key`, as stored (a reference stays one).
    fn inherited_raw(&self, mut id: ObjectId, key: &[u8]) -> Option<Object> {
        for _ in 0..32 {
            let dict = self.pdf.get_dictionary(id).ok()?;
            if let Some(value) = dict.get(key).ok().filter(|v| !matches!(v, Object::Null)) {
                return Some(value.clone());
            }
            id = dict.get(b"Parent").ok()?.as_reference().ok()?;
        }
        None
    }

    /// Make the document's pages exactly `order`, flat under the root.
    pub(crate) fn set_page_order(&mut self, order: &[ObjectId]) -> Result<()> {
        let root = self.pages_root()?;
        for &id in order {
            for key in INHERITED {
                if self.pdf.get_dictionary(id)?.get(key).is_ok_and(|v| !matches!(v, Object::Null)) {
                    continue;
                }
                if let Some(value) = self.inherited_raw(id, key) {
                    self.pdf.get_dictionary_mut(id)?.set(key, value);
                }
            }
        }
        for &id in order {
            self.pdf.get_dictionary_mut(id)?.set("Parent", Object::Reference(root));
        }
        let node = self.pdf.get_dictionary_mut(root)?;
        node.set("Kids", Object::Array(order.iter().map(|&id| Object::Reference(id)).collect()));
        node.set("Count", Object::Integer(order.len() as i64));
        self.pages = order.to_vec();
        Ok(())
    }

    /// Rotate a page by `degrees` (a multiple of 90, clockwise as a reader shows it).
    pub fn rotate_page(&mut self, page: usize, degrees: i32) -> Result<()> {
        if degrees % 90 != 0 {
            return Err(Error::Rejected("la rotación debe ser múltiplo de 90 grados".into()));
        }
        let id = self.page_id(page)?;
        self.transaction(|d| {
            let current = d.page_size(page)?.rotation;
            let turned = (current + degrees).rem_euclid(360) as i64;
            d.pdf.get_dictionary_mut(id)?.set("Rotate", Object::Integer(turned));
            Ok(())
        })
    }

    /// Remove a page, and the form fields that lived only on it. A document keeps at least one page.
    pub fn delete_page(&mut self, page: usize) -> Result<()> {
        let id = self.page_id(page)?;
        if self.pages.len() <= 1 {
            return Err(Error::Rejected("un documento necesita al menos una página".into()));
        }
        self.transaction(|d| {
            let doomed = d.annotation_ids(id);
            let order: Vec<ObjectId> = d.pages.iter().copied().filter(|p| *p != id).collect();
            d.set_page_order(&order)?;
            d.drop_fields_on(&doomed)
        })
    }

    /// Move the page at `from` so that it ends up at index `to`.
    pub fn move_page(&mut self, from: usize, to: usize) -> Result<()> {
        self.page_id(from)?;
        self.page_id(to)?;
        if from == to {
            return Ok(());
        }
        self.transaction(|d| {
            let mut order = d.pages.clone();
            let page = order.remove(from);
            order.insert(to, page);
            d.set_page_order(&order)
        })
    }

    /// A new PDF holding only `pages`, in the order given.
    pub fn extract_pages(&self, pages: &[usize]) -> Result<Vec<u8>> {
        if pages.is_empty() {
            return Err(Error::Rejected("no hay páginas que extraer".into()));
        }
        let ids = pages.iter().map(|&p| self.page_id(p)).collect::<Result<Vec<_>>>()?;
        let mut copy = Document::from_raw(self.pdf.clone_inner());
        copy.set_page_order(&ids)?;
        // Fields whose widgets are on pages left out go too.
        let kept: std::collections::HashSet<ObjectId> = ids.iter().flat_map(|p| copy.annotation_ids(*p)).collect();
        let gone: Vec<ObjectId> = self
            .pages
            .iter()
            .filter(|p| !ids.contains(p))
            .flat_map(|p| copy.annotation_ids(*p))
            .filter(|a| !kept.contains(a))
            .collect();
        copy.drop_fields_on(&gone)?;
        copy.save_to_bytes()
    }

    /// Bring pages of another document in, before the page at `at` (`page_count()` appends).
    /// `source_pages` picks some of them (all when `None`). Returns how many came in.
    ///
    /// The fields of the pages come along. Field names are not made unique: two
    /// forms with a field of the same name will share it when it is filled.
    pub fn insert_pages(&mut self, source: &[u8], password: &str, source_pages: Option<&[usize]>, at: usize) -> Result<usize> {
        if at > self.pages.len() {
            return Err(Error::NoSuchPage(at));
        }
        let mut other = Document::open_with_password(source, password)?;
        let wanted: Vec<usize> = match source_pages {
            Some(list) => list.to_vec(),
            None => (0..other.page_count()).collect(),
        };
        if wanted.is_empty() {
            return Err(Error::Rejected("no hay páginas que traer".into()));
        }
        let ids = wanted.iter().map(|&p| other.page_id(p)).collect::<Result<Vec<_>>>()?;
        other.set_page_order(&ids)?;
        let mut raw = other.pdf.into_inner();
        raw.prune_objects();

        self.transaction(|d| {
            raw.renumber_objects_with(d.pdf.max_id + 1);
            let new_pages: Vec<ObjectId> = raw.get_pages().values().copied().collect();
            // Read the form after renumbering, so its references are the new ones.
            let fields = Document::fields_of_raw(&raw);
            let dr = Document::resources_of_raw(&raw);
            d.pdf.absorb(raw.objects.clone(), raw.max_id);
            let mut order = d.pages.clone();
            for (offset, id) in new_pages.iter().enumerate() {
                order.insert(at + offset, *id);
            }
            d.set_page_order(&order)?;
            d.adopt_fields(fields, dr)?;
            Ok(new_pages.len())
        })
    }

    /// The annotation objects a page lists.
    fn annotation_ids(&self, page: ObjectId) -> Vec<ObjectId> {
        let Ok(dict) = self.pdf.get_dictionary(page) else { return Vec::new() };
        let Some(Object::Array(list)) = dict.get(b"Annots").ok().map(|o| resolve(&self.pdf, o)) else { return Vec::new() };
        list.iter().filter_map(|o| o.as_reference().ok()).collect()
    }

    /// Take widgets (and fields whose every widget is among them) out of the form.
    fn drop_fields_on(&mut self, widgets: &[ObjectId]) -> Result<()> {
        if widgets.is_empty() {
            return Ok(());
        }
        let Ok(root) = self.pdf.trailer.get(b"Root").and_then(|o| o.as_reference()) else { return Ok(()) };
        let Some(form_id) = self.acroform_id(root) else { return Ok(()) };
        let roots: Vec<ObjectId> = self
            .pdf
            .get_dictionary(form_id)
            .ok()
            .and_then(|d| d.get(b"Fields").ok())
            .and_then(|o| resolve(&self.pdf, o).as_array().ok().map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect()))
            .unwrap_or_default();
        let mut keep = Vec::new();
        for id in roots {
            if self.prune_field(id, widgets, 0)? {
                keep.push(Object::Reference(id));
            }
        }
        self.pdf.get_dictionary_mut(form_id)?.set("Fields", Object::Array(keep));
        Ok(())
    }

    /// Remove `gone` from this field's subtree; `false` if nothing of it is left.
    fn prune_field(&mut self, id: ObjectId, gone: &[ObjectId], depth: usize) -> Result<bool> {
        if gone.contains(&id) {
            return Ok(false);
        }
        if depth > 16 {
            return Ok(true);
        }
        let kids: Option<Vec<ObjectId>> = self
            .pdf
            .get_dictionary(id)
            .ok()
            .and_then(|d| d.get(b"Kids").ok())
            .and_then(|o| resolve(&self.pdf, o).as_array().ok().map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect()));
        let Some(kids) = kids else { return Ok(true) };
        let mut kept = Vec::new();
        for kid in &kids {
            if self.prune_field(*kid, gone, depth + 1)? {
                kept.push(Object::Reference(*kid));
            }
        }
        if kept.len() != kids.len() {
            self.pdf.get_dictionary_mut(id)?.set("Kids", Object::Array(kept.clone()));
        }
        Ok(!kept.is_empty())
    }

    fn acroform_id(&self, catalog: ObjectId) -> Option<ObjectId> {
        self.pdf.get_dictionary(catalog).ok()?.get(b"AcroForm").ok()?.as_reference().ok()
    }
}

impl Document {
    /// A lopdf document's form dictionary, wherever the catalog keeps it.
    fn raw_form(raw: &lopdf::Document) -> Option<Dictionary> {
        let held = raw.catalog().ok()?.get(b"AcroForm").ok()?;
        match held {
            Object::Reference(id) => raw.get_dictionary(*id).ok().cloned(),
            Object::Dictionary(dict) => Some(dict.clone()),
            _ => None,
        }
    }

    /// The field references of a lopdf document's form.
    fn fields_of_raw(raw: &lopdf::Document) -> Vec<ObjectId> {
        let Some(form) = Self::raw_form(raw) else { return Vec::new() };
        let list = match form.get(b"Fields") {
            Ok(Object::Reference(id)) => raw.get_object(*id).ok(),
            Ok(other) => Some(other),
            Err(_) => None,
        };
        match list {
            Some(Object::Array(list)) => list.iter().filter_map(|o| o.as_reference().ok()).collect(),
            _ => Vec::new(),
        }
    }

    /// The form's default resources (the fonts its fields are drawn with).
    fn resources_of_raw(raw: &lopdf::Document) -> Option<Object> {
        let form = Self::raw_form(raw)?;
        match form.get(b"DR").ok()? {
            Object::Reference(id) => raw.get_object(*id).ok().cloned(),
            other => Some(other.clone()),
        }
    }

    /// Add `fields` to this document's form, creating the form if it has none,
    /// and any font of `dr` the form does not already have.
    fn adopt_fields(&mut self, fields: Vec<ObjectId>, dr: Option<Object>) -> Result<()> {
        if fields.is_empty() {
            return Ok(());
        }
        let root = self.pdf.trailer.get(b"Root")?.as_reference()?;
        let form_id = match self.acroform_id(root) {
            Some(id) => id,
            None => {
                let mut form = Dictionary::new();
                form.set("Fields", Object::Array(Vec::new()));
                let id = self.pdf.add_object(Object::Dictionary(form));
                self.pdf.get_dictionary_mut(root)?.set("AcroForm", Object::Reference(id));
                id
            }
        };
        let mut list: Vec<Object> = self
            .pdf
            .get_dictionary(form_id)?
            .get(b"Fields")
            .ok()
            .and_then(|o| resolve(&self.pdf, o).as_array().ok().cloned())
            .unwrap_or_default();
        list.extend(fields.into_iter().map(Object::Reference));
        let own_dr = self.pdf.get_dictionary(form_id)?.get(b"DR").ok().map(|o| resolve(&self.pdf, o).clone());
        let merged = merge_resources(own_dr, dr);
        let form = self.pdf.get_dictionary_mut(form_id)?;
        form.set("Fields", Object::Array(list));
        if let Some(dr) = merged {
            form.set("DR", dr);
        }
        Ok(())
    }
}

/// Default resources with the fonts of `extra` the `base` lacks added.
fn merge_resources(base: Option<Object>, extra: Option<Object>) -> Option<Object> {
    match (base, extra) {
        (Some(Object::Dictionary(mut base)), Some(Object::Dictionary(extra))) => {
            let mut fonts = base.get(b"Font").ok().and_then(|o| o.as_dict().ok().cloned()).unwrap_or_default();
            if let Ok(Object::Dictionary(more)) = extra.get(b"Font") {
                for (name, value) in more.iter() {
                    if !fonts.has(name) {
                        fonts.set(name.clone(), value.clone());
                    }
                }
            }
            base.set("Font", Object::Dictionary(fonts));
            Some(Object::Dictionary(base))
        }
        (None, extra) => extra,
        (base, _) => base,
    }
}
