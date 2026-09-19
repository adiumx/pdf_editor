"""In-memory documents, their edit history, and their lifetime.

Documents live in the server process only. Nothing is written to disk, and a
document is dropped when the browser closes it or after it has gone idle, so
running the editor never leaves copies of a PDF lying around.
"""

from __future__ import annotations

import threading
import time
import uuid
from dataclasses import dataclass, field
from typing import Any

import pymupdf

from .fonts import FontResolver

# How many steps of undo to keep. Each step is a serialized copy of the
# document, so this trades memory for history depth.
MAX_HISTORY = 30

# And how much memory those steps may take together. The step count alone is no
# guard: a step weighs what the document weighs, so thirty steps of a 100 MB
# scan would be three gigabytes. Whichever limit is reached first wins, and one
# step is always kept so that undo never stops working outright.
MAX_HISTORY_BYTES = 256 * 1024 * 1024

# Documents untouched for this long are closed by the next sweep.
IDLE_TIMEOUT_SECONDS = 2 * 60 * 60

MAX_UPLOAD_BYTES = 150 * 1024 * 1024


class DocumentError(RuntimeError):
    """The requested document is gone, or cannot be opened."""


@dataclass
class Step:
    """One undoable state of the document.

    Most edits touch one page, so most steps record one page rather than the
    whole file. A step weighs what it saves, and the history's budget is spent
    on what actually changed instead of on forty copies of the same forty
    pages. Changes to the page structure itself — adding, deleting, reordering
    — cannot be expressed that way and record everything.
    """

    pages: dict[int, bytes] | None = None
    document: bytes | None = None
    page_count: int = 0

    @property
    def size(self) -> int:
        if self.document is not None:
            return len(self.document)
        return sum(len(data) for data in (self.pages or {}).values())

    @property
    def is_whole_document(self) -> bool:
        return self.document is not None


@dataclass
class Document:
    """One open PDF plus everything the editor keeps alongside it."""

    id: str
    name: str
    doc: pymupdf.Document
    resolver: FontResolver
    undo_stack: list[Step] = field(default_factory=list)
    redo_stack: list[Step] = field(default_factory=list)
    assets: dict[str, bytes] = field(default_factory=dict)
    touched_at: float = field(default_factory=time.time)
    lock: threading.RLock = field(default_factory=threading.RLock)

    def touch(self) -> None:
        self.touched_at = time.time()

    def snapshot(self, pages: list[int] | None = None) -> None:
        """Record the current state so the next edit can be undone.

        ``pages`` names the pages the edit is about to change; everything else
        is left out of the step. Omit it when the change is to the document as
        a whole.
        """
        self.undo_stack.append(self._capture(pages))
        self._trim(self.undo_stack)
        self.redo_stack.clear()

    def _capture(self, pages: list[int] | None) -> Step:
        """Save either the named pages or the whole document."""
        if pages is None or self._has_form_fields():
            return Step(document=self._serialize(), page_count=self.doc.page_count)
        wanted = sorted({p for p in pages if 0 <= p < self.doc.page_count})
        if not wanted:
            return Step(pages={}, page_count=self.doc.page_count)
        return Step(
            pages={pno: self._serialize_page(pno) for pno in wanted},
            page_count=self.doc.page_count,
        )

    def _has_form_fields(self) -> bool:
        """Whether the document carries an interactive form.

        Form fields are listed once in the document, not on the page that
        shows them, so a page cannot be lifted out and put back on its own:
        the copy arrives while the original is still listed, the reader
        renames it to keep the two apart (``firma`` becomes ``firma [26]``),
        and the entry left behind is never collected. A few rounds of undo
        and the form no longer answers to any of its names. Pruning the list
        by hand corrupts it, so a document with a form is recorded whole.
        """
        return bool(self.doc.is_form_pdf)

    @staticmethod
    def _trim(stack: list[Step]) -> None:
        """Drop the oldest steps until the history fits both limits."""
        while len(stack) > MAX_HISTORY:
            stack.pop(0)
        total = sum(step.size for step in stack)
        while len(stack) > 1 and total > MAX_HISTORY_BYTES:
            total -= stack.pop(0).size

    @property
    def history_bytes(self) -> int:
        """What this document's history is costing."""
        return sum(step.size for step in self.undo_stack) + sum(
            step.size for step in self.redo_stack
        )

    def _serialize(self) -> bytes:
        return self.doc.tobytes(garbage=0, deflate=True)

    def _serialize_page(self, pno: int) -> bytes:
        """One page, on its own, as a small PDF."""
        with pymupdf.open() as single:
            single.insert_pdf(self.doc, from_page=pno, to_page=pno, annots=True, links=True)
            return single.tobytes(garbage=0, deflate=True)

    def _apply(self, step: Step) -> None:
        """Put the document back to what a step recorded."""
        if step.is_whole_document:
            self._restore(step.document)
            return
        for pno, data in (step.pages or {}).items():
            if not 0 <= pno < self.doc.page_count:
                continue
            with pymupdf.open(stream=data, filetype="pdf") as single:
                self.doc.insert_pdf(single, start_at=pno, annots=True, links=True)
            self.doc.delete_page(pno + 1)
        # Font resources and calibration are tied to the pages just replaced.
        self.resolver = FontResolver(self.doc)

    def _restore(self, data: bytes) -> None:
        self.doc.close()
        self.doc = pymupdf.open(stream=data, filetype="pdf")
        self.resolver = FontResolver(self.doc)

    def undo(self) -> bool:
        if not self.undo_stack:
            return False
        step = self.undo_stack.pop()
        # The step back records exactly what the step forward did, so redo costs
        # the same as undo rather than a copy of everything.
        self.redo_stack.append(self._mirror(step))
        self._trim(self.redo_stack)
        self._apply(step)
        self.touch()
        return True

    def redo(self) -> bool:
        if not self.redo_stack:
            return False
        step = self.redo_stack.pop()
        self.undo_stack.append(self._mirror(step))
        self._trim(self.undo_stack)
        self._apply(step)
        self.touch()
        return True

    def _mirror(self, step: Step) -> Step:
        """The current state of whatever a step is about to put back."""
        if step.is_whole_document or step.page_count != self.doc.page_count:
            return self._capture(None)
        return self._capture(list((step.pages or {}).keys()))

    def rollback(self, data: bytes) -> None:
        """Put the document back after an operation failed halfway."""
        self._restore(data)

    def export(self) -> bytes:
        """The finished PDF, cleaned up and compressed for saving."""
        return self.doc.tobytes(garbage=4, deflate=True, clean=True)

    @property
    def can_undo(self) -> bool:
        return bool(self.undo_stack)

    @property
    def can_redo(self) -> bool:
        return bool(self.redo_stack)

    def state(self) -> dict[str, Any]:
        """The bits of document state the client mirrors."""
        return {
            "id": self.id,
            "name": self.name,
            "page_count": self.doc.page_count,
            "can_undo": self.can_undo,
            "can_redo": self.can_redo,
        }

    def close(self) -> None:
        try:
            self.doc.close()
        except Exception:  # pragma: no cover - already closed
            pass
        self.undo_stack.clear()
        self.redo_stack.clear()
        self.assets.clear()


class DocumentStore:
    """Keeps the open documents of every browser tab talking to this server."""

    def __init__(self, idle_timeout: float = IDLE_TIMEOUT_SECONDS) -> None:
        self._documents: dict[str, Document] = {}
        self._lock = threading.RLock()
        self._idle_timeout = idle_timeout

    def open(self, data: bytes, name: str, password: str | None = None) -> Document:
        """Open an uploaded PDF and register it."""
        if not data:
            raise DocumentError("El archivo está vacío.")
        if len(data) > MAX_UPLOAD_BYTES:
            raise DocumentError("El archivo supera el tamaño máximo de 150 MB.")
        try:
            doc = pymupdf.open(stream=data, filetype="pdf")
        except Exception as exc:
            raise DocumentError(f"No se pudo abrir el PDF: {exc}") from exc
        if doc.needs_pass:
            if not password or not doc.authenticate(password):
                doc.close()
                raise DocumentError("El PDF está protegido con contraseña.")
        if doc.page_count == 0:
            doc.close()
            raise DocumentError("El PDF no tiene páginas.")

        document = Document(
            id=uuid.uuid4().hex,
            name=name or "documento.pdf",
            doc=doc,
            resolver=FontResolver(doc),
        )
        with self._lock:
            self._sweep_locked()
            self._documents[document.id] = document
        return document

    def get(self, doc_id: str) -> Document:
        with self._lock:
            document = self._documents.get(doc_id)
        if document is None:
            raise DocumentError("El documento ya no está abierto. Vuelve a subirlo.")
        document.touch()
        return document

    def close(self, doc_id: str) -> None:
        with self._lock:
            document = self._documents.pop(doc_id, None)
        if document is not None:
            document.close()

    def close_all(self) -> None:
        with self._lock:
            documents = list(self._documents.values())
            self._documents.clear()
        for document in documents:
            document.close()

    def _sweep_locked(self) -> None:
        """Drop documents nobody has touched in a long time."""
        cutoff = time.time() - self._idle_timeout
        stale = [doc_id for doc_id, doc in self._documents.items() if doc.touched_at < cutoff]
        for doc_id in stale:
            self._documents.pop(doc_id).close()

    def __len__(self) -> int:
        with self._lock:
            return len(self._documents)
