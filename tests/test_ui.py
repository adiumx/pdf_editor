"""The editor driven through a real browser, with real mouse input.

These exist because the unit tests could not catch a whole class of bug: a
format bar whose controls cannot be clicked still passes every test that sets
values through the DOM. Playwright's `select_option` and `fill` bypass the mouse
the same way, so the tests here click, type and read focus.

Skipped when Playwright or a browser is not installed.
"""

from __future__ import annotations

import socket
import threading
import time

import pytest

from tests.conftest import build_pdf, requires_fonts

playwright_api = pytest.importorskip("playwright.sync_api", reason="playwright no instalado")


def _free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


@pytest.fixture(scope="module")
def server():
    """Serve the editor on a spare port for the length of the module."""
    import uvicorn

    from app.main import app, store

    port = _free_port()
    config = uvicorn.Config(app, host="127.0.0.1", port=port, log_level="error")
    instance = uvicorn.Server(config)
    thread = threading.Thread(target=instance.run, daemon=True)
    thread.start()

    deadline = time.time() + 20
    while not instance.started and time.time() < deadline:
        time.sleep(0.05)
    if not instance.started:
        pytest.skip("el servidor de pruebas no arrancó")

    yield f"http://127.0.0.1:{port}"

    instance.should_exit = True
    thread.join(timeout=10)
    store.close_all()


def _launch(playwright):
    """Start a browser, falling back to a browser installed out of band."""
    import glob
    import os

    try:
        return playwright.chromium.launch()
    except Exception:
        root = os.environ.get("PLAYWRIGHT_BROWSERS_PATH", "/opt/pw-browsers")
        for pattern in ("chromium-*/chrome-linux/chrome", "chromium-*/chrome-mac/*/Chromium"):
            for path in sorted(glob.glob(os.path.join(root, pattern))):
                try:
                    return playwright.chromium.launch(
                        executable_path=path, args=["--no-sandbox"]
                    )
                except Exception:
                    continue
        pytest.skip("no hay navegador de Playwright instalado")


@pytest.fixture(scope="module")
def browser():
    with playwright_api.sync_playwright() as playwright:
        instance = _launch(playwright)
        yield instance
        instance.close()


@pytest.fixture
def page(browser, server, tmp_path):
    """A fresh tab with the sample document open.

    Per test rather than per module: an open native dropdown or a half-finished
    edit left behind by one test swallows the next test's clicks, and a shared
    tab makes such a failure look like a bug in the feature under test.
    """
    tab = browser.new_page(viewport={"width": 1400, "height": 900})
    errors: list[str] = []
    tab.on("pageerror", lambda exc: errors.append(str(exc)))
    tab.on("console", lambda msg: errors.append(msg.text) if msg.type == "error" else None)

    sample = tmp_path / "ejemplo.pdf"
    sample.write_bytes(build_pdf())
    tab.goto(server, wait_until="networkidle")
    tab.set_input_files("#file-input", str(sample))
    tab.wait_for_selector(".page .span", timeout=30000)

    tab.console_errors = errors  # type: ignore[attr-defined]
    yield tab
    tab.close()


def span_with(page, text: str):
    """The overlay covering a given piece of text, found by what it covers.

    Never by index: rewriting a line changes how the page splits into spans, so
    positions shift under any test that edits something. The overlay itself is
    empty, so the search goes through the text it carries as an attribute.
    """
    return page.locator(f'.page .span[data-text*="{text}"]').first


def open_editor(page, text: str = "Primera linea"):
    """Click a span the way a person would, and wait for its box."""
    target = span_with(page, text)
    target.wait_for(state="visible", timeout=10000)
    target.click()
    page.wait_for_selector(".span.is-editing .span__input", timeout=10000)


@requires_fonts
class TestClickingText:
    def test_clicking_a_span_puts_the_caret_in_it(self, page):
        """The box used to open with focus left on the body, so whatever the
        user typed next went nowhere."""
        open_editor(page)
        focused = page.evaluate(
            "() => document.activeElement?.classList?.contains('span__input') || false"
        )
        assert focused, "el foco no llegó al cuadro de edición"

    def test_typing_after_clicking_reaches_the_text(self, page):
        open_editor(page)
        before = page.eval_on_selector(".span.is-editing .span__input", "e => e.textContent")
        page.keyboard.type("ZZZ")
        after = page.eval_on_selector(".span.is-editing .span__input", "e => e.textContent")
        assert after != before and "ZZZ" in after


@requires_fonts
class TestFormatBar:
    """Its controls used to be unclickable: a preventDefault meant to keep the
    text box open also stopped the bar's own inputs from taking focus."""

    def test_the_bar_appears_when_text_is_being_edited(self, page):
        open_editor(page)
        assert page.locator("#formatbar").is_visible()

    def test_clicking_the_size_field_focuses_it(self, page):
        open_editor(page)
        page.locator("#fmt-size").click()
        assert page.evaluate("() => document.activeElement?.id") == "fmt-size"

    def test_typing_a_size_changes_the_field(self, page):
        open_editor(page)
        page.locator("#fmt-size").click()
        page.keyboard.press("Control+a")
        page.keyboard.type("21")
        assert page.locator("#fmt-size").input_value() == "21"

    def test_clicking_the_font_list_focuses_it(self, page):
        open_editor(page)
        page.locator("#fmt-family").click()
        assert page.evaluate("() => document.activeElement?.id") == "fmt-family"

    def test_the_font_list_offers_more_than_the_generic_families(self, page):
        open_editor(page)
        labels = page.eval_on_selector_all("#fmt-family optgroup", "els => els.map(e => e.label)")
        options = page.eval_on_selector_all("#fmt-family option", "els => els.length")
        assert any("Estándar" in label for label in labels)
        assert any("Instaladas" in label for label in labels)
        assert options > 6, "el desplegable debería ofrecer bastante más que las genéricas"

    def test_the_size_field_is_not_rewritten_while_being_typed_in(self, page):
        """Re-rendering the bar on every state change used to clobber input."""
        open_editor(page)
        page.locator("#fmt-size").click()
        page.keyboard.press("Control+a")
        page.keyboard.type("3")
        page.keyboard.type("3")
        assert page.locator("#fmt-size").input_value() == "33"


@requires_fonts
class TestApplyingChanges:
    def test_a_size_change_reaches_the_document(self, page):
        # A line of its own, so the change takes the single-line path and the
        # text stays findable; re-wrapping is covered on its own below.
        open_editor(page, "Titulo en negrita")
        original = float(page.locator("#fmt-size").input_value())
        assert original != 26

        page.locator("#fmt-size").click()
        page.keyboard.press("Control+a")
        page.keyboard.type("26")
        page.keyboard.press("Enter")  # Enter inside the bar applies
        page.wait_for_function("() => !document.querySelector('.span.is-editing')", timeout=15000)
        page.wait_for_selector(".page .span", timeout=15000)

        # Read it back from the document, not from the bar's leftover state.
        open_editor(page, "Titulo en negrita")
        assert float(page.locator("#fmt-size").input_value()) == pytest.approx(26, abs=0.6)

    def test_a_font_change_reaches_the_document(self, page):
        open_editor(page, "Titulo en negrita")
        page.locator("#fmt-family").click()
        page.keyboard.press("Escape")  # close the native list, keep the box open
        page.select_option("#fmt-family", "Liberation Mono")
        page.click("#fmt-apply")
        page.wait_for_function("() => !document.querySelector('.span.is-editing')", timeout=15000)
        page.wait_for_selector(".page .span", timeout=15000)

        open_editor(page, "Titulo en negrita")
        assert "Mono" in page.locator("#fmt-family").input_value()

    def test_no_console_errors_were_raised_along_the_way(self, page):
        assert page.console_errors == []


@requires_fonts
class TestReflow:
    """Text that runs across several lines re-wraps; a line on its own does not."""

    def test_the_reflow_button_appears_only_for_multi_line_text(self, page):
        open_editor(page, "Primera linea")
        assert page.locator("#fmt-reflow").is_visible()

        page.keyboard.press("Escape")
        open_editor(page, "Una linea en sans")
        assert page.locator("#fmt-reflow").is_hidden()

    def test_re_wrapping_keeps_every_word(self, page):
        open_editor(page, "Primera linea")
        page.click("#fmt-reflow")
        page.wait_for_function("() => !document.querySelector('.span.is-editing')", timeout=15000)
        page.wait_for_selector(".page .span", timeout=15000)
        text = " ".join(
            page.eval_on_selector_all(".page .span", "els => els.map(e => e.dataset.text)")
        )
        for word in ("Primera", "documento", "Segunda", "debajo"):
            assert word in text, f"se perdió «{word}» al reajustar"


@requires_fonts
class TestUnsavedWork:
    """The document lives in the server's memory and nowhere else, so leaving
    the page throws the edits away. It used to do that silently."""

    def _edit_something(self, page):
        open_editor(page)
        page.keyboard.type("XX")
        page.keyboard.press("Enter")
        page.wait_for_function("() => !document.querySelector('.span.is-editing')", timeout=15000)
        page.wait_for_selector(".page .span", timeout=15000)

    def test_a_freshly_opened_document_has_nothing_to_lose(self, page):
        assert page.evaluate("() => window.__editor?.hasUnsavedWork") in (False, None)

    def test_leaving_after_an_edit_is_challenged(self, page):
        self._edit_something(page)
        # Playwright answers the browser's own leave prompt; seeing it asked at
        # all is the point.
        asked = []
        page.on("dialog", lambda dialog: (asked.append(dialog.message), dialog.accept()))
        page.evaluate("""() => {
            const event = new Event('beforeunload', { cancelable: true });
            window.dispatchEvent(event);
            window.__leaveWasBlocked = event.defaultPrevented;
        }""")
        assert page.evaluate("() => window.__leaveWasBlocked") is True

    def test_leaving_without_edits_is_not_challenged(self, page):
        page.evaluate("""() => {
            const event = new Event('beforeunload', { cancelable: true });
            window.dispatchEvent(event);
            window.__leaveWasBlocked = event.defaultPrevented;
        }""")
        assert page.evaluate("() => window.__leaveWasBlocked") is False

    def test_opening_another_pdf_over_unsaved_edits_asks_first(self, page, tmp_path):
        self._edit_something(page)
        asked = []
        page.on("dialog", lambda dialog: (asked.append(dialog.message), dialog.dismiss()))
        other = tmp_path / "otro.pdf"
        other.write_bytes(build_pdf())
        page.set_input_files("#file-input", str(other))
        page.wait_for_timeout(600)
        assert asked, "no preguntó nada antes de descartar el trabajo"
        assert "sin guardar" in asked[0]

    def test_the_editing_box_has_spell_checking_on(self, page):
        open_editor(page)
        assert page.eval_on_selector(".span.is-editing .span__input", "e => e.spellcheck") is True


@requires_fonts
class TestFindBar:
    def _search(self, page, query):
        page.keyboard.press("Control+f")
        page.wait_for_selector("#findbar:not([hidden])", timeout=5000)
        page.fill("#find-query", query)
        page.wait_for_timeout(700)

    def test_ctrl_f_opens_it(self, page):
        assert page.locator("#findbar").is_hidden()
        page.keyboard.press("Control+f")
        page.wait_for_selector("#findbar:not([hidden])", timeout=5000)
        assert page.evaluate("() => document.activeElement?.id") == "find-query"

    def test_escape_closes_it(self, page):
        self._search(page, "linea")
        page.keyboard.press("Escape")
        page.wait_for_timeout(300)
        assert page.locator("#findbar").is_hidden()

    def test_results_are_counted_and_highlighted(self, page):
        self._search(page, "linea")
        assert page.locator("#find-count").text_content().startswith("1 de ")
        assert page.locator(".hit").count() >= 2
        assert page.locator(".hit--current").count() == 1

    def test_stepping_moves_the_current_result(self, page):
        self._search(page, "linea")
        first = page.locator("#find-count").text_content()
        page.click("#find-next")
        page.wait_for_timeout(300)
        assert page.locator("#find-count").text_content() != first
        assert page.locator(".hit--current").count() == 1

    def test_a_query_with_no_results_says_so(self, page):
        self._search(page, "zzzznoexiste")
        assert "sin resultados" in page.locator("#find-count").text_content()
        assert page.locator(".hit").count() == 0

    def test_replacing_changes_the_document(self, page):
        page.on("dialog", lambda dialog: dialog.accept())
        self._search(page, "Primera")
        page.fill("#find-replacement", "REEMPLAZADA")
        page.click("#find-replace-all")
        page.wait_for_timeout(3500)
        texts = " ".join(
            page.eval_on_selector_all(".page .span", "els => els.map(e => e.dataset.text)")
        )
        assert "REEMPLAZADA" in texts
        assert "Primera" not in texts

    def test_no_console_errors_while_searching(self, page):
        self._search(page, "linea")
        page.click("#find-next")
        page.wait_for_timeout(400)
        assert page.console_errors == []


@requires_fonts
class TestMoveTool:
    def test_dragging_a_block_moves_it(self, page):
        page.click('[data-tool="move"]')
        target = span_with(page, "Primera linea")
        before = target.bounding_box()

        page.mouse.move(before["x"] + before["width"] / 2, before["y"] + before["height"] / 2)
        page.mouse.down()
        page.mouse.move(
            before["x"] + before["width"] / 2 + 120,
            before["y"] + before["height"] / 2 + 140,
            steps=10,
        )
        assert page.locator(".ghost").count() >= 1, "no se ve dónde caería"
        page.mouse.up()
        page.wait_for_timeout(3500)

        after = span_with(page, "Primera linea").bounding_box()
        assert after["x"] > before["x"] + 60
        assert after["y"] > before["y"] + 80

    def test_the_ghost_disappears_after_dropping(self, page):
        page.click('[data-tool="move"]')
        target = span_with(page, "Primera linea")
        box = target.bounding_box()
        page.mouse.move(box["x"] + 20, box["y"] + box["height"] / 2)
        page.mouse.down()
        page.mouse.move(box["x"] + 90, box["y"] + 100, steps=6)
        page.mouse.up()
        page.wait_for_timeout(3000)
        assert page.locator(".ghost").count() == 0

    def test_moving_does_not_open_the_editor(self, page):
        page.click('[data-tool="move"]')
        target = span_with(page, "Titulo en negrita")
        box = target.bounding_box()
        page.mouse.move(box["x"] + 10, box["y"] + box["height"] / 2)
        page.mouse.down()
        page.mouse.move(box["x"] + 60, box["y"] + 60, steps=5)
        page.mouse.up()
        page.wait_for_timeout(2500)
        assert page.locator(".span.is-editing").count() == 0
