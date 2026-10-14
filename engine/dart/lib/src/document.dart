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
