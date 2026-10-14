//! Undo and redo without copying the document.
//!
//! Every mutation of the object table goes through [`Tracked`], which, while a
//! transaction is open, remembers the *previous* value of each object the first
//! time it is touched. Objects created during the transaction need no copy:
//! they are exactly the ones numbered above the table's size when it began.
//!
//! An edit mostly adds objects (a new content stream, a new appearance) and
//! changes a few small dictionaries (the page's `/Contents`, a field's `/V`), so
//! a step costs a few hundred bytes however large the document is. The same
//! patch that undoes a step, applied, yields the patch that redoes it.

use std::collections::BTreeMap;
use std::ops::Deref;

use lopdf::{Dictionary, Object, ObjectId};

use crate::error::Result;

/// A set of changes to the object table, and what it transforms back into
/// when applied.
#[derive(Default, Debug)]
pub(crate) struct Patch {
    /// Objects to put back (or put in) as they should be.
    set: BTreeMap<ObjectId, Object>,
    /// Objects numbered above this are removed first.
    top: u32,
}

pub(crate) struct Tracked {
    doc: lopdf::Document,
    open: Option<Patch>,
}

impl Deref for Tracked {
    type Target = lopdf::Document;
    fn deref(&self) -> &lopdf::Document {
        &self.doc
    }
}

impl Tracked {
    pub fn new(doc: lopdf::Document) -> Tracked {
        Tracked { doc, open: None }
    }

    pub fn begin(&mut self) {
        self.open = Some(Patch { set: BTreeMap::new(), top: self.doc.max_id });
    }

    /// Close the transaction, handing back the patch that undoes it.
    pub fn commit(&mut self) -> Patch {
        self.open.take().unwrap_or_default()
    }

    /// Undo whatever the open transaction did.
    pub fn rollback(&mut self) {
        if let Some(patch) = self.open.take() {
            self.apply(patch);
        }
    }

    /// Make the table what `patch` says; returns the patch that reverses it.
    pub fn apply(&mut self, patch: Patch) -> Patch {
        let mut inverse = Patch { set: BTreeMap::new(), top: self.doc.max_id };
        for id in patch.set.keys() {
            if let Some(now) = self.doc.objects.get(id) {
                inverse.set.insert(*id, now.clone());
            }
        }
        let doomed: Vec<ObjectId> = self.doc.objects.keys().filter(|id| id.0 > patch.top).copied().collect();
        for id in doomed {
            if let Some(object) = self.doc.objects.remove(&id) {
                inverse.set.insert(id, object);
            }
        }
        for (id, object) in patch.set {
            self.doc.objects.insert(id, object);
        }
        self.doc.max_id = patch.top;
        inverse
    }

    fn remember(&mut self, id: ObjectId) {
        if let Some(open) = &mut self.open {
            if id.0 <= open.top && !open.set.contains_key(&id) {
                if let Some(old) = self.doc.objects.get(&id) {
                    open.set.insert(id, old.clone());
                }
            }
        }
    }

    // The same names lopdf's Document has, so every call site is tracked
    // without being rewritten.

    pub fn get_dictionary_mut(&mut self, id: ObjectId) -> Result<&mut Dictionary> {
        self.remember(id);
        Ok(self.doc.get_dictionary_mut(id)?)
    }

    pub fn get_object_mut(&mut self, id: ObjectId) -> Result<&mut Object> {
        self.remember(id);
        Ok(self.doc.get_object_mut(id)?)
    }

    /// Take in objects numbered above the current top (another document's,
    /// renumbered to fit): they need no copy to be undone, like any new object.
    pub fn absorb(&mut self, objects: BTreeMap<ObjectId, Object>, max_id: u32) {
        self.doc.objects.extend(objects);
        self.doc.max_id = self.doc.max_id.max(max_id);
    }

    pub fn into_inner(self) -> lopdf::Document {
        self.doc
    }

    pub fn clone_inner(&self) -> lopdf::Document {
        self.doc.clone()
    }

    pub fn add_object<T: Into<Object>>(&mut self, object: T) -> ObjectId {
        self.doc.add_object(object)
    }

    /// The document as bytes, without the objects nothing points to. Done on a
    /// copy: the live table keeps everything, because undoing a step can bring
    /// back a reference to an object the saved file would have dropped.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut copy = self.doc.clone();
        copy.prune_objects();
        let mut out = Vec::new();
        copy.save_to(&mut out).map_err(lopdf::Error::from)?;
        Ok(out)
    }
}
