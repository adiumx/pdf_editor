import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'models.dart';

/// Opens the native library.
///
/// Without a [path]: Android and Linux look for `libpdfcore_ffi.so` where the
/// app's libraries are; iOS and macOS find the symbols in the app itself (the
/// static library is linked into it); Windows loads `pdfcore_ffi.dll`.
DynamicLibrary loadPdfCoreLibrary({String? path}) {
  if (path != null) return DynamicLibrary.open(path);
  if (Platform.isAndroid || Platform.isLinux) return DynamicLibrary.open('libpdfcore_ffi.so');
  if (Platform.isIOS || Platform.isMacOS) return DynamicLibrary.process();
  if (Platform.isWindows) return DynamicLibrary.open('pdfcore_ffi.dll');
  throw UnsupportedError('Plataforma no soportada: ${Platform.operatingSystem}');
}

const _reflowCodes = {Reflow.none: 0, Reflow.following: 1, Reflow.centered: 2, Reflow.rightAligned: 3};

List<double> _box(Object? raw) => (raw as List).map((v) => (v as num).toDouble()).toList();

typedef _Str = Pointer<Utf8>;
typedef _Handle = Pointer<Void>;

class _Native {
  _Native(DynamicLibrary lib)
      : open = lib.lookupFunction<_Handle Function(Pointer<Uint8>, Size), _Handle Function(Pointer<Uint8>, int)>(
            'pdfcore_open'),
        openWithPassword = lib.lookupFunction<_Handle Function(Pointer<Uint8>, Size, _Str),
            _Handle Function(Pointer<Uint8>, int, _Str)>('pdfcore_open_with_password'),
        lastError = lib.lookupFunction<_Str Function(), _Str Function()>('pdfcore_last_error'),
        lastErrorKind = lib.lookupFunction<_Str Function(), _Str Function()>('pdfcore_last_error_kind'),
        close = lib.lookupFunction<Void Function(_Handle), void Function(_Handle)>('pdfcore_close'),
        closeAddress = lib.lookup<NativeFunction<Void Function(_Handle)>>('pdfcore_close'),
        freeString = lib.lookupFunction<Void Function(_Str), void Function(_Str)>('pdfcore_free_string'),
        freeBytes = lib.lookupFunction<Void Function(Pointer<Uint8>, Size), void Function(Pointer<Uint8>, int)>(
            'pdfcore_free_bytes'),
        pageCount = lib.lookupFunction<Int Function(_Handle), int Function(_Handle)>('pdfcore_page_count'),
        page = lib.lookupFunction<_Str Function(_Handle, Int), _Str Function(_Handle, int)>('pdfcore_page'),
        fields = lib.lookupFunction<_Str Function(_Handle), _Str Function(_Handle)>('pdfcore_fields'),
        replaceText = lib.lookupFunction<_Str Function(_Handle, Int, Int, _Str, Int),
            _Str Function(_Handle, int, int, _Str, int)>('pdfcore_replace_text'),
        replaceTextEx = lib.lookupFunction<_Str Function(_Handle, Int, Int, _Str, Int, Int),
            _Str Function(_Handle, int, int, _Str, int, int)>('pdfcore_replace_text_ex'),
        lines = lib.lookupFunction<_Str Function(_Handle, Int), _Str Function(_Handle, int)>('pdfcore_lines'),
        find = lib.lookupFunction<_Str Function(_Handle, _Str, Int), _Str Function(_Handle, _Str, int)>('pdfcore_find'),
        replaceAll = lib.lookupFunction<_Str Function(_Handle, _Str, _Str, Int, Int, Int),
            _Str Function(_Handle, _Str, _Str, int, int, int)>('pdfcore_replace_all'),
        moveField = lib.lookupFunction<_Str Function(_Handle, _Str, Int, Float, Float, Float, Float),
            _Str Function(_Handle, _Str, int, double, double, double, double)>('pdfcore_move_field'),
        arrangeFields = lib.lookupFunction<_Str Function(_Handle, _Str), _Str Function(_Handle, _Str)>('pdfcore_arrange_fields'),
        items = lib.lookupFunction<_Str Function(_Handle, Int), _Str Function(_Handle, int)>('pdfcore_items'),
        moveItems = lib.lookupFunction<_Str Function(_Handle, Int, _Str), _Str Function(_Handle, int, _Str)>('pdfcore_move_items'),
        arrangeItems =
            lib.lookupFunction<_Str Function(_Handle, Int, _Str), _Str Function(_Handle, int, _Str)>('pdfcore_arrange_items'),
        spaceLines = lib.lookupFunction<_Str Function(_Handle, Int, _Str), _Str Function(_Handle, int, _Str)>('pdfcore_space_lines'),
        alignParagraph = lib.lookupFunction<_Str Function(_Handle, Int, Int, Int), _Str Function(_Handle, int, int, int)>(
            'pdfcore_align_paragraph'),
        setFieldsAlignment =
            lib.lookupFunction<_Str Function(_Handle, _Str), _Str Function(_Handle, _Str)>('pdfcore_set_fields_alignment'),
        requiredFonts = lib.lookupFunction<_Str Function(_Handle), _Str Function(_Handle)>('pdfcore_required_fonts'),
        registerFont = lib.lookupFunction<_Str Function(_Handle, Pointer<Uint8>, Size),
            _Str Function(_Handle, Pointer<Uint8>, int)>('pdfcore_register_font'),
        replaceBlockEx = lib.lookupFunction<_Str Function(_Handle, Int, Int, _Str, _Str),
            _Str Function(_Handle, int, int, _Str, _Str)>('pdfcore_replace_block_ex'),
        replaceRange = lib.lookupFunction<_Str Function(_Handle, Int, Int, Int, Int, _Str, _Str),
            _Str Function(_Handle, int, int, int, int, _Str, _Str)>('pdfcore_replace_range'),
        setBlockStyle = lib.lookupFunction<_Str Function(_Handle, Int, Int, Int, Int, _Str),
            _Str Function(_Handle, int, int, int, int, _Str)>('pdfcore_set_block_style'),
        blockChars = lib.lookupFunction<_Str Function(_Handle, Int, Int), _Str Function(_Handle, int, int)>('pdfcore_block_chars'),
        hitTest = lib.lookupFunction<_Str Function(_Handle, Int, Float, Float), _Str Function(_Handle, int, double, double)>('pdfcore_hit_test'),
        deleteItems = lib.lookupFunction<_Str Function(_Handle, Int, _Str), _Str Function(_Handle, int, _Str)>('pdfcore_delete_items'),
        resizeItem = lib.lookupFunction<_Str Function(_Handle, Int, Int, Float, Float, Float, Float),
            _Str Function(_Handle, int, int, double, double, double, double)>('pdfcore_resize_item'),
        refreshAppearances =
            lib.lookupFunction<_Str Function(_Handle, Int), _Str Function(_Handle, int)>('pdfcore_refresh_appearances'),
        batch = lib.lookupFunction<_Str Function(_Handle, _Str), _Str Function(_Handle, _Str)>('pdfcore_batch'),
        setSubstitution = lib.lookupFunction<_Str Function(_Handle, Int), _Str Function(_Handle, int)>('pdfcore_set_substitution'),
        blocks = lib.lookupFunction<_Str Function(_Handle, Int), _Str Function(_Handle, int)>('pdfcore_blocks'),
        replaceBlock = lib.lookupFunction<_Str Function(_Handle, Int, Int, _Str, Int),
            _Str Function(_Handle, int, int, _Str, int)>('pdfcore_replace_block'),
        paragraph = lib.lookupFunction<_Str Function(_Handle, Int, Int), _Str Function(_Handle, int, int)>('pdfcore_paragraph'),
        replaceParagraph = lib.lookupFunction<_Str Function(_Handle, Int, Int, _Str, Int),
            _Str Function(_Handle, int, int, _Str, int)>('pdfcore_replace_paragraph'),
        addImage = lib.lookupFunction<_Str Function(_Handle, Int, Pointer<Uint8>, Size, _Str),
            _Str Function(_Handle, int, Pointer<Uint8>, int, _Str)>('pdfcore_add_image'),
        rotatePage = lib.lookupFunction<_Str Function(_Handle, Int, Int), _Str Function(_Handle, int, int)>('pdfcore_rotate_page'),
        deletePage = lib.lookupFunction<_Str Function(_Handle, Int), _Str Function(_Handle, int)>('pdfcore_delete_page'),
        movePage = lib.lookupFunction<_Str Function(_Handle, Int, Int), _Str Function(_Handle, int, int)>('pdfcore_move_page'),
        insertPages = lib.lookupFunction<_Str Function(_Handle, Pointer<Uint8>, Size, _Str, Pointer<Int>, Size, Int),
            _Str Function(_Handle, Pointer<Uint8>, int, _Str, Pointer<Int>, int, int)>('pdfcore_insert_pages'),
        extractPages = lib.lookupFunction<Pointer<Uint8> Function(_Handle, Pointer<Int>, Size, Pointer<Size>),
            Pointer<Uint8> Function(_Handle, Pointer<Int>, int, Pointer<Size>)>('pdfcore_extract_pages'),
        signatures = lib.lookupFunction<_Str Function(_Handle), _Str Function(_Handle)>('pdfcore_signatures'),
        addText = lib.lookupFunction<_Str Function(_Handle, Int, _Str), _Str Function(_Handle, int, _Str)>(
            'pdfcore_add_text'),
        setField = lib.lookupFunction<_Str Function(_Handle, _Str, _Str, Int), _Str Function(_Handle, _Str, _Str, int)>(
            'pdfcore_set_field'),
        undo = lib.lookupFunction<_Str Function(_Handle), _Str Function(_Handle)>('pdfcore_undo'),
        redo = lib.lookupFunction<_Str Function(_Handle), _Str Function(_Handle)>('pdfcore_redo'),
        history = lib.lookupFunction<_Str Function(_Handle), _Str Function(_Handle)>('pdfcore_history'),
        save = lib.lookupFunction<Pointer<Uint8> Function(_Handle, Pointer<Size>),
            Pointer<Uint8> Function(_Handle, Pointer<Size>)>('pdfcore_save');

  final _Handle Function(Pointer<Uint8>, int) open;
  final _Handle Function(Pointer<Uint8>, int, _Str) openWithPassword;
  final _Str Function() lastError;
  final _Str Function() lastErrorKind;
  final void Function(_Handle) close;
  final Pointer<NativeFunction<Void Function(_Handle)>> closeAddress;
  final void Function(_Str) freeString;
  final void Function(Pointer<Uint8>, int) freeBytes;
  final int Function(_Handle) pageCount;
  final _Str Function(_Handle, int) page;
  final _Str Function(_Handle) fields;
  final _Str Function(_Handle, int, int, _Str, int) replaceText;
  final _Str Function(_Handle, int, int, _Str, int, int) replaceTextEx;
  final _Str Function(_Handle, int) lines;
  final _Str Function(_Handle, _Str, int) find;
  final _Str Function(_Handle, _Str, _Str, int, int, int) replaceAll;
  final _Str Function(_Handle, _Str, int, double, double, double, double) moveField;
  final _Str Function(_Handle, _Str) arrangeFields;
  final _Str Function(_Handle, int) items;
  final _Str Function(_Handle, int, _Str) moveItems;
  final _Str Function(_Handle, int, _Str) arrangeItems;
  final _Str Function(_Handle, int, _Str) spaceLines;
  final _Str Function(_Handle, int, int, int) alignParagraph;
  final _Str Function(_Handle, _Str) setFieldsAlignment;
  final _Str Function(_Handle) requiredFonts;
  final _Str Function(_Handle, Pointer<Uint8>, int) registerFont;
  final _Str Function(_Handle, int, int, _Str, _Str) replaceBlockEx;
  final _Str Function(_Handle, int, int, int, int, _Str, _Str) replaceRange;
  final _Str Function(_Handle, int, int, int, int, _Str) setBlockStyle;
  final _Str Function(_Handle, int, int) blockChars;
  final _Str Function(_Handle, int, double, double) hitTest;
  final _Str Function(_Handle, int, _Str) deleteItems;
  final _Str Function(_Handle, int, int, double, double, double, double) resizeItem;
  final _Str Function(_Handle, int) refreshAppearances;
  final _Str Function(_Handle, _Str) batch;
  final _Str Function(_Handle, int) setSubstitution;
  final _Str Function(_Handle, int) blocks;
  final _Str Function(_Handle, int, int, _Str, int) replaceBlock;
  final _Str Function(_Handle, int, int) paragraph;
  final _Str Function(_Handle, int, int, _Str, int) replaceParagraph;
  final _Str Function(_Handle, int, Pointer<Uint8>, int, _Str) addImage;
  final _Str Function(_Handle, int, int) rotatePage;
  final _Str Function(_Handle, int) deletePage;
  final _Str Function(_Handle, int, int) movePage;
  final _Str Function(_Handle, Pointer<Uint8>, int, _Str, Pointer<Int>, int, int) insertPages;
  final Pointer<Uint8> Function(_Handle, Pointer<Int>, int, Pointer<Size>) extractPages;
  final _Str Function(_Handle) signatures;
  final _Str Function(_Handle, int, _Str) addText;
  final _Str Function(_Handle, _Str, _Str, int) setField;
  final _Str Function(_Handle) undo;
  final _Str Function(_Handle) redo;
  final _Str Function(_Handle) history;
  final Pointer<Uint8> Function(_Handle, Pointer<Size>) save;
}

/// A PDF open in the engine. Call [close] when done.
///
/// Synchronous and not thread-safe: use it from a single isolate. For a UI,
/// prefer `PdfSession`.
class PdfDocument implements Finalizable {
  PdfDocument._(this._n, this._handle) {
    _finalizer(_n).attach(this, _handle.cast(), detach: this);
  }

  final _Native _n;
  _Handle _handle;

  static final _natives = Expando<_Native>();
  static final _finalizers = <_Native, NativeFinalizer>{};

  static NativeFinalizer _finalizer(_Native n) =>
      _finalizers.putIfAbsent(n, () => NativeFinalizer(n.closeAddress.cast()));

  /// Parse [bytes] as a PDF. Throws [PdfException] if they are not one.
  ///
  /// A document protected only against editing or printing opens as any other.
  /// One that needs a password throws a [PdfException] of kind
  /// `password_required`; call again with [password], and a wrong one throws
  /// `wrong_password`. Saving writes the document without protection.
  static PdfDocument open(Uint8List bytes, {String? password, DynamicLibrary? library, String? libraryPath}) {
    final lib = library ?? loadPdfCoreLibrary(path: libraryPath);
    final n = _natives[lib] ??= _Native(lib);
    final buffer = malloc<Uint8>(bytes.isEmpty ? 1 : bytes.length);
    try {
      buffer.asTypedList(bytes.length).setAll(0, bytes);
      final secret = (password ?? '').toNativeUtf8();
      final Pointer<Void> handle;
      try {
        handle = n.openWithPassword(buffer, bytes.length, secret);
      } finally {
        malloc.free(secret);
      }
      if (handle == nullptr) {
        throw PdfException(n.lastErrorKind().toDartString(), n.lastError().toDartString());
      }
      return PdfDocument._(n, handle);
    } finally {
      malloc.free(buffer);
    }
  }

  _Handle get _live {
    if (_handle == nullptr) throw StateError('El documento ya se cerró');
    return _handle;
  }

  /// Every call returns JSON; an `error` key means it failed.
  Map<String, dynamic> _json(_Str raw) {
    final text = raw.toDartString();
    _n.freeString(raw);
    final decoded = jsonDecode(text) as Map<String, dynamic>;
    if (decoded.containsKey('error')) throw PdfException.fromJson(decoded);
    return decoded;
  }

  T _withString<T>(String value, T Function(_Str) use) {
    final native = value.toNativeUtf8();
    try {
      return use(native);
    } finally {
      malloc.free(native);
    }
  }

  int get pageCount => _n.pageCount(_live);

  /// Size and text runs of a page. Renews every run id.
  PageData page(int index) => PageData.fromJson(index, _json(_n.page(_live, index)));

  List<FormField> fields() =>
      (_json(_n.fields(_live))['fields'] as List).map((f) => FormField.fromJson(f as Map<String, dynamic>)).toList();

  /// Replace a run's text with the font it already uses.
  ///
  /// With [allowFallback], characters that font lacks are drawn with a bundled
  /// one; otherwise they throw a `missing_glyphs` [PdfException].
  ///
  /// A different length would leave a gap or an overlap on the line, so by
  /// default what follows on the line moves with it ([Reflow.following]); the
  /// result's `warnings` say if that pushed the line past the margin. Invalidates
  /// every run id: read the page again.
  ///
  /// Throws a `not_editable` [PdfException] (`reason`: `type3`, `vertical_writing`, `symbolic_font`,
  /// `rtl_text`) for text that cannot be reproduced; check [TextRun.editable] first.
  EditResult replaceText(PageData page, TextRun run, String text,
          {bool allowFallback = false, Reflow reflow = Reflow.following}) =>
      replaceTextById(page.index, run.id, text, allowFallback: allowFallback, reflow: reflow);

  /// As [replaceText], naming the run by its page and id.
  EditResult replaceTextById(int page, int runId, String text,
          {bool allowFallback = false, Reflow reflow = Reflow.following}) =>
      EditResult.fromJson(_withString(
          text,
          (t) => _json(_n.replaceTextEx(
              _live, page, runId, t, allowFallback ? 1 : 0, _reflowCodes[reflow]!))));

  /// Put a field's widget in the box `[x0, y0, x1, y1]` (as a reader shows the
  /// page) and redraw it at that size. The box is kept on the page and at least
  /// a few points each way; returns where it ended up, so a grid can show it.
  /// A signed signature cannot be moved.
  List<double> moveField(String name, List<double> box, {int widget = 0}) {
    final j = _withString(name, (n) => _json(_n.moveField(_live, n, widget, box[0], box[1], box[2], box[3])));
    return _box(j['display']);
  }

  /// Line fields up or space them evenly, as one step for [undo].
  void arrangeFields(List<FieldRefs> fields, Arrange how) {
    final params = jsonEncode({
      'how': how.code,
      'fields': [for (final f in fields) {'name': f.name, 'widget': f.widget}],
    });
    _withString(params, (p) => _json(_n.arrangeFields(_live, p)));
  }

  /// What can be moved on [page]: text (a paragraph is one item), pictures and
  /// form fields, top to bottom. Read it again after any edit.
  List<PdfItem> items(int page) => (_json(_n.items(_live, page))['items'] as List).map((i) {
        final m = i as Map<String, dynamic>;
        return PdfItem(
          (m['id'] as num).toInt(),
          const {'text': PdfItemKind.text, 'image': PdfItemKind.image, 'field': PdfItemKind.field}[m['kind']]!,
          _box(m['display']),
          m['text'] as String,
          lines: (m['lines'] as num?)?.toInt() ?? 0,
          fieldName: m['name'] as String?,
          widget: (m['widget'] as num?)?.toInt() ?? 0,
        );
      }).toList();

  /// Move items by the distances given, as one step for [undo]. The text near
  /// a moved text stays where it was.
  void moveItems(int page, List<ItemMove> moves) {
    final params = jsonEncode({
      'moves': [for (final m in moves) {'id': m.item.id, 'dx': m.dx, 'dy': m.dy}],
    });
    _withString(params, (p) => _json(_n.moveItems(_live, page, p)));
  }

  /// What is under the point ([x], [y] as the page is shown): a field before text before a picture.
  /// For text it says which block, which letter and where the caret goes. Reads the page's items
  /// and blocks again, so the ids in the answer are the current ones.
  PdfHit? hitTest(int page, double x, double y) {
    final h = _json(_n.hitTest(_live, page, x, y))['hit'] as Map<String, dynamic>?;
    if (h == null) return null;
    return PdfHit(
      const {'text': PdfItemKind.text, 'image': PdfItemKind.image, 'field': PdfItemKind.field}[h['kind']]!,
      (h['id'] as num).toInt(),
      _box(h['display']),
      fieldName: h['name'] as String?,
      widget: (h['widget'] as num?)?.toInt() ?? 0,
      block: (h['block'] as num?)?.toInt(),
      char: (h['char'] as num?)?.toInt(),
      caret: (h['caret'] as num?)?.toInt(),
    );
  }

  /// Take items off the page (text stops drawing, a picture is cancelled, a field leaves the page
  /// and the form), as one step for [undo].
  void deleteItems(int page, List<PdfItem> items) {
    final params = jsonEncode({'ids': [for (final i in items) i.id]});
    _withString(params, (p) => _json(_n.deleteItems(_live, page, p)));
  }

  /// Resize [item] to [box] (`[x0, y0, x1, y1]` as the page is shown): a picture is scaled, a field
  /// gets the box, text is laid out again within the width (its height follows from the text).
  void resizeItem(int page, PdfItem item, List<double> box) =>
      _json(_n.resizeItem(_live, page, item.id, box[0], box[1], box[2], box[3]));

  /// Build the look of the form's fields again from their values; with [onlyMissing] just the ones
  /// with a widget that has none (a reader that builds none of its own shows empty boxes).
  RefreshResult refreshAppearances({bool onlyMissing = true}) {
    final j = _json(_n.refreshAppearances(_live, onlyMissing ? 1 : 0));
    return RefreshResult((j['rebuilt'] as num).toInt(), (j['skipped'] as num).toInt());
  }

  /// Several edits as ONE undo step: if any fails none is applied (the error says which kind). Each
  /// [BatchOp] points at ids of the last readings; they must point at different things.
  /// Returns the JSON result of each op (`substitutes`, `warnings`...), in order.
  List<Map<String, dynamic>> batch(List<BatchOp> ops) {
    final params = jsonEncode({'ops': [for (final o in ops) o.json]});
    final j = _withString(params, (p) => _json(_n.batch(_live, p)));
    return [for (final r in j['results'] as List) r as Map<String, dynamic>];
  }

  /// Line items up (or space them evenly) like a design tool does, as one step.
  /// Texts, pictures and fields can be mixed; making sizes equal is for fields only.
  void arrangeItems(int page, List<PdfItem> items, Arrange how) {
    final params = jsonEncode({'ids': [for (final i in items) i.id], 'how': how.code});
    _withString(params, (p) => _json(_n.arrangeItems(_live, page, p)));
  }

  /// An even leading for text items: the first stays and each next baseline is
  /// [leading] points below the one before (the average of the gaps there are now when null).
  void spaceLines(int page, List<PdfItem> items, {double? leading}) {
    final params = jsonEncode({'ids': [for (final i in items) i.id], 'leading': leading});
    _withString(params, (p) => _json(_n.spaceLines(_live, page, p)));
  }

  /// Align the lines of the paragraph [run] belongs to against its left edge,
  /// centre or right edge (like a word processor's alignment buttons).
  void alignParagraph(PageData page, TextRun run, TextAlign align) =>
      _json(_n.alignParagraph(_live, page.index, run.id, align.index));

  /// Align the text inside form fields (text, choice and list fields), as one step.
  void setFieldsAlignment(List<String> names, TextAlign align) {
    final params = jsonEncode({'names': names, 'align': align.name});
    _withString(params, (p) => _json(_n.setFieldsAlignment(_live, p)));
  }

  /// The paragraph [run] is a line of (two lines or more of one font, evenly
  /// spaced and left-aligned), or null. Read the page first.
  PdfParagraph? paragraphOf(PageData page, TextRun run) => paragraphById(page.index, run.id);

  PdfParagraph? paragraphById(int page, int runId) {
    final p = _json(_n.paragraph(_live, page, runId))['paragraph'] as Map<String, dynamic>?;
    if (p == null) return null;
    return PdfParagraph(p['text'] as String, (p['runs'] as List).map((r) => (r as num).toInt()).toList(), _box(p['display']));
  }

  /// Lay the paragraph [run] belongs to out again with [text] as all of its text,
  /// within the width it has: lines are added below or emptied at the end as
  /// needed, in the paragraph's own font and colour. Invalidates every run id.
  ParagraphResult replaceParagraph(PageData page, TextRun run, String text, {bool allowFallback = false}) =>
      replaceParagraphById(page.index, run.id, text, allowFallback: allowFallback);

  ParagraphResult replaceParagraphById(int page, int runId, String text, {bool allowFallback = false}) {
    final j = _withString(text, (t) => _json(_n.replaceParagraph(_live, page, runId, t, allowFallback ? 1 : 0)));
    return ParagraphResult(
      (j['lines'] as num).toInt(),
      (j['added'] as num).toInt(),
      (j['removed'] as num).toInt(),
      j['fallback_chars'] as String,
      (j['warnings'] as List).map((w) => PdfWarning.fromJson(w as Map<String, dynamic>)).toList(),
    );
  }

  /// The fonts the document uses and which ones editing exactly would need (not embedded, or only
  /// the used letters are). Offer the person [FontSource.url] or let them pick a font from the
  /// device, then pass its bytes to [registerFont].
  List<RequiredFont> requiredFonts() =>
      (_json(_n.requiredFonts(_live))['fonts'] as List).map((f) => RequiredFont.fromJson(f as Map<String, dynamic>)).toList();

  /// Hand over a TrueType font the person may use. It is used, at similarity 100, for the letters
  /// other fonts of its family lack. Throws `rejected` when its licence forbids embedding and
  /// `unsupported` when it is not TrueType.
  RegisteredFont registerFont(Uint8List font) {
    final buffer = malloc<Uint8>(font.isEmpty ? 1 : font.length);
    try {
      buffer.asTypedList(font.length).setAll(0, font);
      final j = _json(_n.registerFont(_live, buffer, font.length));
      return RegisteredFont(j['family'] as String, j['bold'] as bool, j['italic'] as bool);
    } finally {
      malloc.free(buffer);
    }
  }

  /// Which letters use the bundled font when the original lacks some and the caller allows it
  /// (`allowFallback`): see [Substitution]. Stays until changed.
  void setSubstitution(Substitution policy) => _json(_n.setSubstitution(_live, policy.index));

  /// The paragraphs of [page], each as ONE block with the style of every stretch
  /// of its text (a paragraph with bold words is not split). Ids are good until
  /// the next edit.
  List<PdfBlock> blocks(int page) =>
      (_json(_n.blocks(_live, page))['blocks'] as List).map((b) => PdfBlock.fromJson(b as Map<String, dynamic>)).toList();

  /// Lay [block] out again with [text] as all of its text, keeping the style of
  /// each letter that stays; only the lines that change are rewritten. One undo
  /// step. Invalidates every id.
  BlockResult replaceBlock(int page, PdfBlock block, String text,
          {bool allowFallback = false, bool? justify, bool pushDown = false}) =>
      replaceBlockById(page, block.id, text, allowFallback: allowFallback, justify: justify, pushDown: pushDown);

  /// [justify] forces the lines justified (`true`) or ragged (`false`); [pushDown] moves what is
  /// below the block down by the room the added lines take (what would not fit on the page stays,
  /// with a `below_page` warning).
  BlockResult replaceBlockById(int page, int blockId, String text,
      {bool allowFallback = false, bool? justify, bool pushDown = false}) {
    final Map<String, dynamic> j;
    if (justify == null && !pushDown) {
      j = _withString(text, (t) => _json(_n.replaceBlock(_live, page, blockId, t, allowFallback ? 1 : 0)));
    } else {
      final params = jsonEncode({'allow_fallback': allowFallback, 'justify': justify, 'push_down': pushDown});
      j = _withString(text, (t) => _withString(params, (p) => _json(_n.replaceBlockEx(_live, page, blockId, t, p))));
    }
    return _blockResult(j);
  }

  /// Replace the letters `[start, end)` of [block]'s text (characters) with [text], which takes the
  /// style of the first letter it replaces. One undo step.
  BlockResult replaceRange(int page, PdfBlock block, int start, int end, String text,
      {bool allowFallback = false, bool? justify, bool pushDown = false}) {
    final params = jsonEncode({'allow_fallback': allowFallback, 'justify': justify, 'push_down': pushDown});
    return _blockResult(_withString(
        text, (t) => _withString(params, (p) => _json(_n.replaceRange(_live, page, block.id, start, end, t, p)))));
  }

  /// Change the style of the letters `[start, end)` of [block]: bold, italic, [size] in points,
  /// [color] as `[r, g, b]` in 0..1. Bold and italic use the document's own font of that face when
  /// the page has one; otherwise a bundled one, which needs [allowFallback] (`rejected` without it).
  BlockResult setBlockStyle(int page, PdfBlock block, int start, int end,
      {bool? bold,
      bool? italic,
      double? size,
      List<double>? color,
      bool allowFallback = false,
      bool? justify,
      bool pushDown = false}) {
    final params = jsonEncode({
      'bold': bold,
      'italic': italic,
      'size': size,
      'color': color,
      'allow_fallback': allowFallback,
      'justify': justify,
      'push_down': pushDown,
    });
    return _blockResult(_withString(params, (p) => _json(_n.setBlockStyle(_live, page, block.id, start, end, p))));
  }

  /// Where every character of [block]'s text is, `[x0, y0, x1, y1]` as the page is shown: for a
  /// caret or a selection. The space that joins two lines is an empty box at the end of the first.
  List<List<double>> blockChars(int page, PdfBlock block) =>
      (_json(_n.blockChars(_live, page, block.id))['boxes'] as List).map((b) => _box(b)).toList();

  BlockResult _blockResult(Map<String, dynamic> j) => BlockResult(
        (j['lines'] as num).toInt(),
        (j['kept'] as num).toInt(),
        (j['rewritten'] as num).toInt(),
        (j['added'] as num).toInt(),
        (j['removed'] as num).toInt(),
        j['fallback_chars'] as String,
        (j['warnings'] as List).map((w) => PdfWarning.fromJson(w as Map<String, dynamic>)).toList(),
        substitutes: FontSubstitute.list(j['substitutes']),
      );

  /// The page as lines of words, top to bottom. Reads the page again, so the
  /// run ids in each line are the current ones.
  List<PdfLine> lines(int page) => (_json(_n.lines(_live, page))['lines'] as List).map((l) {
        final m = l as Map<String, dynamic>;
        return PdfLine(
          m['text'] as String,
          _box(m['display']),
          (m['runs'] as List).map((r) => (r as num).toInt()).toList(),
          (m['words'] as List).map((w) => PdfWord((w as Map)['text'] as String, _box(w['display']))).toList(),
          m['invisible'] as bool,
        );
      }).toList();

  /// Every occurrence of [query] in the document, page by page.
  List<SearchMatch> find(String query, {SearchOptions options = const SearchOptions()}) =>
      _withString(query, (q) => (_json(_n.find(_live, q, options.flags))['matches'] as List).map((m) {
            final j = m as Map<String, dynamic>;
            final range = j['within_run'] as List?;
            return SearchMatch(
              (j['page'] as num).toInt(),
              j['text'] as String,
              j['line'] as String,
              _box(j['display']),
              (j['run'] as num).toInt(),
              range == null ? null : ((range[0] as num).toInt(), (range[1] as num).toInt()),
            );
          }).toList());

  /// Replace every occurrence of [find] with [replacement] as one step that
  /// [undo] takes back whole. Invalidates every run id.
  ReplaceAllResult replaceAll(String find, String replacement,
      {SearchOptions options = const SearchOptions(), bool allowFallback = false, Reflow reflow = Reflow.following}) {
    final j = _withString(find, (f) => _withString(replacement, (r) => _json(_n.replaceAll(
        _live, f, r, options.flags, allowFallback ? 1 : 0, _reflowCodes[reflow]!))));
    return ReplaceAllResult((j['replaced'] as num).toInt(), (j['skipped'] as num).toInt());
  }

  /// The completed digital signatures. Editing invalidates them: say so before
  /// the first edit.
  List<PdfSignature> signatures() => (_json(_n.signatures(_live))['signatures'] as List)
      .map((s) => PdfSignature((s as Map)['field'] as String, s['signer'] as String?))
      .toList();

  AddedText addText(int page, NewText text) {
    final j = _withString(jsonEncode(text.toJson()), (p) => _json(_n.addText(_live, page, p)));
    return AddedText((j['display'] as List).map((v) => (v as num).toDouble()).toList(), j['lines'] as int);
  }

  /// Fill a form field. [ignoreReadOnly] fills one the form marks read-only:
  /// ask the person first.
  void setField(String name, String value, {bool ignoreReadOnly = false}) {
    _withString(name, (n) => _withString(value, (v) => _json(_n.setField(_live, n, v, ignoreReadOnly ? 1 : 0))));
  }

  /// Returns false when there was nothing to undo. Renews every run id.
  bool undo() => _json(_n.undo(_live))['ok'] as bool;
  bool redo() => _json(_n.redo(_live))['ok'] as bool;

  History get history {
    final j = _json(_n.history(_live));
    return History(j['can_undo'] as bool, j['can_redo'] as bool);
  }

  /// Draw a JPEG or PNG on [page] in the box at ([x], [y]) [width] wide, in the
  /// page as a reader shows it (origin top-left, y down). Without [height] the
  /// picture keeps its proportions. A JPEG goes in as it is; a PNG keeps its transparency.
  AddedImage addImage(int page, Uint8List picture, {required double x, required double y, required double width, double? height}) {
    final buffer = malloc<Uint8>(picture.isEmpty ? 1 : picture.length);
    try {
      buffer.asTypedList(picture.length).setAll(0, picture);
      final params = jsonEncode({'x': x, 'y': y, 'width': width, if (height != null) 'height': height});
      final j = _withString(params, (p) => _json(_n.addImage(_live, page, buffer, picture.length, p)));
      final pixels = j['pixels'] as List;
      return AddedImage(_box(j['display']), (pixels[0] as num).toInt(), (pixels[1] as num).toInt());
    } finally {
      malloc.free(buffer);
    }
  }

  /// Turn a page by [degrees] (a multiple of 90, clockwise as a reader shows it).
  void rotatePage(int page, int degrees) => _json(_n.rotatePage(_live, page, degrees));

  /// Remove a page and the form fields only it held. A document keeps one page
  /// at least. Invalidates every run id.
  void deletePage(int page) => _json(_n.deletePage(_live, page));

  /// Move the page at [from] so it ends up at index [to].
  void movePage(int from, int to) => _json(_n.movePage(_live, from, to));

  /// Bring pages of another PDF in before page [at] ([pageCount] appends), all
  /// of them or the [pages] listed. [password] is for a protected source.
  /// Returns how many came in. Field names are not made unique.
  int insertPages(Uint8List source, {int? at, List<int>? pages, String? password}) {
    final buffer = malloc<Uint8>(source.isEmpty ? 1 : source.length);
    final list = pages == null ? nullptr.cast<Int>() : malloc<Int>(pages.isEmpty ? 1 : pages.length);
    try {
      buffer.asTypedList(source.length).setAll(0, source);
      if (pages != null) {
        for (var i = 0; i < pages.length; i++) {
          list[i] = pages[i];
        }
      }
      final result = _withString(password ?? '', (secret) => _json(
          _n.insertPages(_live, buffer, source.length, secret, list, pages?.length ?? 0, at ?? pageCount)));
      return (result['added'] as num).toInt();
    } finally {
      malloc.free(buffer);
      if (pages != null) malloc.free(list);
    }
  }

  /// A new PDF with only [pages], in that order.
  Uint8List extractPages(List<int> pages) {
    final list = malloc<Int>(pages.isEmpty ? 1 : pages.length);
    final length = calloc<Size>();
    try {
      for (var i = 0; i < pages.length; i++) {
        list[i] = pages[i];
      }
      final data = _n.extractPages(_live, list, pages.length, length);
      if (data == nullptr) throw PdfException(_n.lastErrorKind().toDartString(), _n.lastError().toDartString());
      try {
        return Uint8List.fromList(data.asTypedList(length.value));
      } finally {
        _n.freeBytes(data, length.value);
      }
    } finally {
      malloc.free(list);
      calloc.free(length);
    }
  }

  /// The document as PDF bytes (a copy: safe to keep and write to a file).
  Uint8List save() {
    final length = calloc<Size>();
    try {
      final data = _n.save(_live, length);
      if (data == nullptr) throw PdfException('pdf', _n.lastError().toDartString());
      try {
        return Uint8List.fromList(data.asTypedList(length.value));
      } finally {
        _n.freeBytes(data, length.value);
      }
    } finally {
      calloc.free(length);
    }
  }

  void close() {
    if (_handle == nullptr) return;
    _finalizer(_n).detach(this);
    _n.close(_handle);
    _handle = nullptr;
  }
}
