//! Saving the changes as an update appended to the file the document was opened from, leaving
//! every byte of the original where it was. A signed file keeps its signature's byte range
//! (readers show what was signed and that it was later changed), and a big file is not rewritten
//! in full.

use lopdf::IncrementalDocument;

use crate::error::{Error, Result};
use crate::Document;

impl Document {
    /// The file as it was opened with the changes appended as an incremental update. With no
    /// changes, the original bytes. The original's bytes are kept exactly at the start of the
    /// result. Documents that were opened with a password are saved in the same protection.
    pub fn save_incremental(&mut self) -> Result<Vec<u8>> {
        let (bytes, password) = self.source.clone().ok_or_else(|| Error::Unsupported("este documento no se abrió desde un archivo: no hay original al que añadir".into()))?;
        let previous = lopdf::Document::load_mem_with_options(&bytes, lopdf::LoadOptions::with_password(&password))?;
        let changed: Vec<_> = self
            .pdf
            .objects
            .iter()
            .filter(|(id, object)| previous.objects.get(*id) != Some(*object))
            .map(|(id, object)| (*id, object.clone()))
            .collect();
        if changed.is_empty() {
            return Ok(bytes);
        }
        let mut update = IncrementalDocument::create_from(bytes, previous);
        for (id, object) in changed {
            update.new_document.set_object(id, object);
        }
        update.new_document.max_id = update.new_document.max_id.max(self.pdf.max_id);
        let mut out = Vec::new();
        update.save_to(&mut out).map_err(lopdf::Error::from)?;
        // An update is only as good as the cross-reference table it points back to. A file whose own
        // table is damaged (a PDF whose line endings were converted, say) was read by scanning it, and
        // the update would point to a place that is not there: check, and say so rather than hand
        // back a file that does not open.
        let ok = Document::open_with_password(&out, &password).is_ok_and(|d| d.page_count() == self.page_count());
        if !ok {
            return Err(Error::Unsupported(
                "la tabla de referencias del archivo original está dañada: un guardado incremental no sería válido, usa save()".into(),
            ));
        }
        Ok(out)
    }
}
