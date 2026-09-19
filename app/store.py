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

# Documents untouched for this long are closed by the next sweep.
IDLE_TIMEOUT_SECONDS = 2 * 60 * 60

MAX_UPLOAD_BYTES = 150 * 1024 * 1024


class DocumentError(RuntimeError):
    """The requested document is gone, or cannot be opened."""


@dataclass
class Document:
    """One open PDF plus everything the editor keeps alongside it."""

    id: str
    name: str
    doc: pymupdf.Document
    resolver: FontResolver
    undo_stack: list[bytes] = field(default_factory=list)
    redo_stack: list[bytes] = field(default_factory=list)
    assets: dict[str, bytes] = field(default_factory=dict)
    touched_at: float = field(default_factory=time.time)
    lock: threading.RLock = field(default_factory=threading.RLock)

    def touch(self) -> None:
        self.touched_at = time.time()

    def snapshot(self) -> None:
        """Record the current state so the next edit can be undone."""
        self.undo_stack.append(self._serialize())
        if len(self.undo_stack) > MAX_HISTORY:
            self.undo_stack.pop(0)
        self.redo_stack.clear()

    def _serialize(self) -> bytes:
        return self.doc.tobytes(garbage=0, deflate=True)

    def _restore(self, data: bytes) -> None:
        self.doc.close()
        self.doc = pymupdf.open(stream=data, filetype="pdf")
        self.resolver = FontResolver(self.doc)

    def undo(self) -> bool:
        if not self.undo_stack:
            return False
        self.redo_stack.append(self._serialize())
        self._restore(self.undo_stack.pop())
        self.touch()
        return True

    def redo(self) -> bool:
        if not self.redo_stack:
            return False
        self.undo_stack.append(self._serialize())
        self._restore(self.redo_stack.pop())
        self.touch()
        return True

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
