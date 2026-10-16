import 'dart:async';
import 'dart:isolate';
import 'dart:typed_data';

import 'document.dart';
import 'models.dart';

/// A PDF open in the engine, which runs in an isolate of its own.
///
/// Reading a big page or saving a long document takes long enough to drop
/// frames if done on the UI isolate. Here every call is a message and returns a
/// [Future], so the screen stays smooth. Calls are answered in the order made.
///
/// Same rules as [PdfDocument]: run ids come from the last [page] reading and
/// any edit, [undo] or [redo] invalidates them.
class PdfSession {
  PdfSession._(this._isolate, this._commands, this._replies) {
    _subscription = _replies.listen(_onReply);
  }

  final Isolate _isolate;
  final SendPort _commands;
  final ReceivePort _replies;
  late final StreamSubscription<dynamic> _subscription;
  final _pending = <int, Completer<Object?>>{};
  var _next = 0;
  var _closed = false;

  static Future<PdfSession> open(Uint8List bytes, {String? password, String? libraryPath}) async {
    final inbox = ReceivePort();
    final isolate = await Isolate.spawn(
      _engineMain,
      _Start(inbox.sendPort, TransferableTypedData.fromList([bytes]), libraryPath, password),
      errorsAreFatal: false,
    );
    final first = await inbox.first;
    inbox.close();
    if (first is SendPort) {
      // `first` consumed the port's stream; a new port carries the replies.
      final replies = ReceivePort();
      first.send(replies.sendPort);
      return PdfSession._(isolate, first, replies);
    }
    isolate.kill(priority: Isolate.immediate);
    throw _exception(first as Map<Object?, Object?>);
  }

  void _onReply(dynamic message) {
    final reply = message as List<Object?>;
    final completer = _pending.remove(reply[0] as int);
    if (completer == null) return;
    if (reply[1] as bool) {
      completer.complete(reply[2]);
    } else {
      completer.completeError(_exception(reply[2] as Map<Object?, Object?>));
    }
  }

  Future<T> _call<T>(String op, [List<Object?> args = const []]) {
    if (_closed) return Future.error(StateError('La sesión ya se cerró'));
    final id = _next++;
    final completer = Completer<Object?>();
    _pending[id] = completer;
    _commands.send([id, op, args]);
    return completer.future.then((v) => v as T);
  }

  Future<int> pageCount() => _call<int>('pageCount');
  Future<PageData> page(int index) => _call<PageData>('page', [index]);
  Future<List<FormField>> fields() => _call<List<FormField>>('fields');

  /// Fails with a `not_editable` [PdfException] (see [TextRun.editable]) for text that cannot be reproduced.
  Future<EditResult> replaceText(PageData page, TextRun run, String text,
          {bool allowFallback = false, Reflow reflow = Reflow.following}) =>
      _call<EditResult>('replaceText', [page.index, run.id, text, allowFallback, reflow]);

  Future<List<PdfLine>> lines(int page) => _call<List<PdfLine>>('lines', [page]);

  Future<List<SearchMatch>> find(String query, {SearchOptions options = const SearchOptions()}) =>
      _call<List<SearchMatch>>('find', [query, options]);

  Future<ReplaceAllResult> replaceAll(String find, String replacement,
          {SearchOptions options = const SearchOptions(), bool allowFallback = false, Reflow reflow = Reflow.following}) =>
      _call<ReplaceAllResult>('replaceAll', [find, replacement, options, allowFallback, reflow]);

  Future<List<PdfItem>> items(int page) => _call<List<PdfItem>>('items', [page]);
  Future<void> moveItems(int page, List<ItemMove> moves) => _call<void>('moveItems', [page, moves]);
  Future<void> arrangeItems(int page, List<PdfItem> items, Arrange how) => _call<void>('arrangeItems', [page, items, how]);
  Future<void> spaceLines(int page, List<PdfItem> items, {double? leading}) =>
      _call<void>('spaceLines', [page, items, leading]);
  Future<void> alignParagraph(PageData page, TextRun run, TextAlign align) =>
      _call<void>('alignParagraph', [page, run, align]);
  Future<void> setFieldsAlignment(List<String> names, TextAlign align) => _call<void>('setFieldsAlignment', [names, align]);

  Future<List<double>> moveField(String name, List<double> box, {int widget = 0}) =>
      _call<List<double>>('moveField', [name, box, widget]);

  Future<void> arrangeFields(List<FieldRefs> fields, Arrange how) => _call<void>('arrangeFields', [fields, how]);

  Future<List<RequiredFont>> requiredFonts() => _call<List<RequiredFont>>('requiredFonts', []);

  Future<RegisteredFont> registerFont(Uint8List font) => _call<RegisteredFont>('registerFont', [font]);

  Future<PdfHit?> hitTest(int page, double x, double y) => _call<PdfHit?>('hitTest', [page, x, y]);

  Future<void> deleteItems(int page, List<PdfItem> items) => _call<void>('deleteItems', [page, items]);

  Future<void> resizeItem(int page, PdfItem item, List<double> box) => _call<void>('resizeItem', [page, item, box]);

  Future<RefreshResult> refreshAppearances({bool onlyMissing = true}) =>
      _call<RefreshResult>('refreshAppearances', [onlyMissing]);

  Future<List<Map<String, dynamic>>> batch(List<BatchOp> ops) => _call<List<Map<String, dynamic>>>('batch', [ops]);

  Future<List<PdfAnnotation>> annotations(int page) => _call<List<PdfAnnotation>>('annotations', [page]);

  Future<int> addAnnotation(int page, NewAnnotation annotation) => _call<int>('addAnnotation', [page, annotation]);

  Future<void> deleteAnnotation(int page, PdfAnnotation annotation) => _call<void>('deleteAnnotation', [page, annotation]);

  Future<void> setAnnotationContents(int page, PdfAnnotation annotation, String contents) =>
      _call<void>('setAnnotationContents', [page, annotation, contents]);

  Future<void> moveAnnotation(int page, PdfAnnotation annotation, double dx, double dy) =>
      _call<void>('moveAnnotation', [page, annotation, dx, dy]);

  Future<FlattenResult> flatten({List<int>? pages, bool annotations = true, bool forms = true}) =>
      _call<FlattenResult>('flatten', [pages, annotations, forms]);

  Future<RedactResult> redact(int page, List<List<double>> boxes, {List<double> fill = const [0, 0, 0], bool removeImages = false}) =>
      _call<RedactResult>('redact', [page, boxes, fill, removeImages]);

  Future<List<PageReport>> ocrReport() => _call<List<PageReport>>('ocrReport', []);

  Future<Uint8List> saveIncremental() => _call<Uint8List>('saveIncremental', []);

  Future<List<PdfGraphic>> graphics(int page) => _call<List<PdfGraphic>>('graphics', [page]);

  Future<void> setSubstitution(Substitution policy) => _call<void>('setSubstitution', [policy]);

  Future<List<PdfBlock>> blocks(int page) => _call<List<PdfBlock>>('blocks', [page]);

  Future<BlockResult> replaceBlock(int page, PdfBlock block, String text,
          {bool allowFallback = false, bool? justify, bool pushDown = false}) =>
      _call<BlockResult>('replaceBlock', [page, block.id, text, allowFallback, justify, pushDown]);

  Future<BlockResult> replaceRange(int page, PdfBlock block, int start, int end, String text,
          {bool allowFallback = false, bool? justify, bool pushDown = false}) =>
      _call<BlockResult>('replaceRange', [page, block, start, end, text, allowFallback, justify, pushDown]);

  Future<BlockResult> setBlockStyle(int page, PdfBlock block, int start, int end,
          {bool? bold, bool? italic, double? size, List<double>? color, bool allowFallback = false, bool? justify, bool pushDown = false}) =>
      _call<BlockResult>('setBlockStyle', [page, block, start, end, bold, italic, size, color, allowFallback, justify, pushDown]);

  Future<List<List<double>>> blockChars(int page, PdfBlock block) => _call<List<List<double>>>('blockChars', [page, block]);

  Future<PdfParagraph?> paragraphOf(PageData page, TextRun run) => _call<PdfParagraph?>('paragraph', [page.index, run.id]);

  Future<ParagraphResult> replaceParagraph(PageData page, TextRun run, String text, {bool allowFallback = false}) =>
      _call<ParagraphResult>('replaceParagraph', [page.index, run.id, text, allowFallback]);

  Future<AddedImage> addImage(int page, Uint8List picture,
          {required double x, required double y, required double width, double? height}) =>
      _call<AddedImage>('addImage', [page, TransferableTypedData.fromList([picture]), x, y, width, height]);

  Future<void> rotatePage(int page, int degrees) => _call<void>('rotatePage', [page, degrees]);
  Future<void> deletePage(int page) => _call<void>('deletePage', [page]);
  Future<void> movePage(int from, int to) => _call<void>('movePage', [from, to]);

  Future<int> insertPages(Uint8List source, {int? at, List<int>? pages, String? password}) =>
      _call<int>('insertPages', [TransferableTypedData.fromList([source]), at, pages, password]);

  Future<Uint8List> extractPages(List<int> pages) async {
    final data = await _call<TransferableTypedData>('extractPages', [pages]);
    return data.materialize().asUint8List();
  }

  Future<List<PdfSignature>> signatures() => _call<List<PdfSignature>>('signatures');

  Future<AddedText> addText(int page, NewText text) => _call<AddedText>('addText', [page, text]);

  Future<void> setField(String name, String value, {bool ignoreReadOnly = false}) =>
      _call<void>('setField', [name, value, ignoreReadOnly]);

  Future<bool> undo() => _call<bool>('undo');
  Future<bool> redo() => _call<bool>('redo');
  Future<History> history() => _call<History>('history');

  Future<Uint8List> save() async {
    final data = await _call<TransferableTypedData>('save');
    return data.materialize().asUint8List();
  }

  /// Release the document and stop the isolate. The session is unusable after.
  Future<void> close() async {
    if (_closed) return;
    await _call<void>('close');
    _closed = true;
    await _subscription.cancel();
    _replies.close();
    _isolate.kill(priority: Isolate.beforeNextEvent);
  }
}

PdfException _exception(Map<Object?, Object?> m) =>
    PdfException(m['kind'] as String, m['error'] as String, chars: (m['chars'] as String?) ?? '', reason: m['reason'] as String?);

Map<String, Object?> _describe(PdfException e) => {'kind': e.kind, 'error': e.message, 'chars': e.chars, 'reason': e.reason};

class _Start {
  _Start(this.reply, this.bytes, this.libraryPath, this.password);
  final SendPort reply;
  final TransferableTypedData bytes;
  final String? libraryPath;
  final String? password;
}

/// Entry point of the engine's isolate.
void _engineMain(_Start start) {
  final PdfDocument doc;
  try {
    doc = PdfDocument.open(start.bytes.materialize().asUint8List(),
        password: start.password, libraryPath: start.libraryPath);
  } on PdfException catch (e) {
    start.reply.send(_describe(e));
    return;
  } catch (e) {
    start.reply.send({'kind': 'panic', 'error': '$e'});
    return;
  }

  final commands = ReceivePort();
  late final SendPort replies;
  var wired = false;
  start.reply.send(commands.sendPort);

  commands.listen((message) {
    if (!wired) {
      replies = message as SendPort; // the first message is where to answer
      wired = true;
      return;
    }
    final [id as int, op as String, args as List<Object?>] = message as List<Object?>;
    try {
      final result = _run(doc, op, args);
      replies.send([id, true, result]);
    } on PdfException catch (e) {
      replies.send([id, false, _describe(e)]);
    } catch (e) {
      replies.send([id, false, {'kind': 'panic', 'error': '$e'}]);
    }
    if (op == 'close') commands.close();
  });
}

Object? _run(PdfDocument doc, String op, List<Object?> a) {
  switch (op) {
    case 'pageCount':
      return doc.pageCount;
    case 'page':
      return doc.page(a[0] as int);
    case 'fields':
      return doc.fields();
    case 'replaceText':
      return doc.replaceTextById(a[0] as int, a[1] as int, a[2] as String,
          allowFallback: a[3] as bool, reflow: a[4] as Reflow);
    case 'lines':
      return doc.lines(a[0] as int);
    case 'find':
      return doc.find(a[0] as String, options: a[1] as SearchOptions);
    case 'replaceAll':
      return doc.replaceAll(a[0] as String, a[1] as String,
          options: a[2] as SearchOptions, allowFallback: a[3] as bool, reflow: a[4] as Reflow);
    case 'items':
      return doc.items(a[0] as int);
    case 'moveItems':
      doc.moveItems(a[0] as int, (a[1] as List).cast<ItemMove>());
      return null;
    case 'arrangeItems':
      doc.arrangeItems(a[0] as int, (a[1] as List).cast<PdfItem>(), a[2] as Arrange);
      return null;
    case 'spaceLines':
      doc.spaceLines(a[0] as int, (a[1] as List).cast<PdfItem>(), leading: a[2] as double?);
      return null;
    case 'alignParagraph':
      doc.alignParagraph(a[0] as PageData, a[1] as TextRun, a[2] as TextAlign);
      return null;
    case 'setFieldsAlignment':
      doc.setFieldsAlignment((a[0] as List).cast<String>(), a[1] as TextAlign);
      return null;
    case 'moveField':
      return doc.moveField(a[0] as String, (a[1] as List).cast<double>(), widget: a[2] as int);
    case 'arrangeFields':
      doc.arrangeFields((a[0] as List).cast<FieldRefs>(), a[1] as Arrange);
      return null;
    case 'requiredFonts':
      return doc.requiredFonts();
    case 'registerFont':
      return doc.registerFont(a[0] as Uint8List);
    case 'hitTest':
      return doc.hitTest(a[0] as int, a[1] as double, a[2] as double);
    case 'deleteItems':
      doc.deleteItems(a[0] as int, (a[1] as List).cast<PdfItem>());
      return null;
    case 'resizeItem':
      doc.resizeItem(a[0] as int, a[1] as PdfItem, (a[2] as List).cast<double>());
      return null;
    case 'refreshAppearances':
      return doc.refreshAppearances(onlyMissing: a[0] as bool);
    case 'batch':
      return doc.batch((a[0] as List).cast<BatchOp>());
    case 'annotations':
      return doc.annotations(a[0] as int);
    case 'addAnnotation':
      return doc.addAnnotation(a[0] as int, a[1] as NewAnnotation);
    case 'deleteAnnotation':
      doc.deleteAnnotation(a[0] as int, a[1] as PdfAnnotation);
      return null;
    case 'setAnnotationContents':
      doc.setAnnotationContents(a[0] as int, a[1] as PdfAnnotation, a[2] as String);
      return null;
    case 'moveAnnotation':
      doc.moveAnnotation(a[0] as int, a[1] as PdfAnnotation, a[2] as double, a[3] as double);
      return null;
    case 'flatten':
      return doc.flatten(pages: (a[0] as List?)?.cast<int>(), annotations: a[1] as bool, forms: a[2] as bool);
    case 'redact':
      return doc.redact(a[0] as int, (a[1] as List).map((b) => (b as List).cast<double>()).toList(),
          fill: (a[2] as List).cast<double>(), removeImages: a[3] as bool);
    case 'ocrReport':
      return doc.ocrReport();
    case 'saveIncremental':
      return doc.saveIncremental();
    case 'graphics':
      return doc.graphics(a[0] as int);
    case 'setSubstitution':
      doc.setSubstitution(a[0] as Substitution);
      return null;
    case 'blocks':
      return doc.blocks(a[0] as int);
    case 'replaceBlock':
      return doc.replaceBlockById(a[0] as int, a[1] as int, a[2] as String,
          allowFallback: a[3] as bool, justify: a[4] as bool?, pushDown: a[5] as bool);
    case 'replaceRange':
      return doc.replaceRange(a[0] as int, a[1] as PdfBlock, a[2] as int, a[3] as int, a[4] as String,
          allowFallback: a[5] as bool, justify: a[6] as bool?, pushDown: a[7] as bool);
    case 'setBlockStyle':
      return doc.setBlockStyle(a[0] as int, a[1] as PdfBlock, a[2] as int, a[3] as int,
          bold: a[4] as bool?, italic: a[5] as bool?, size: a[6] as double?, color: (a[7] as List?)?.cast<double>(),
          allowFallback: a[8] as bool, justify: a[9] as bool?, pushDown: a[10] as bool);
    case 'blockChars':
      return doc.blockChars(a[0] as int, a[1] as PdfBlock);
    case 'paragraph':
      return doc.paragraphById(a[0] as int, a[1] as int);
    case 'replaceParagraph':
      return doc.replaceParagraphById(a[0] as int, a[1] as int, a[2] as String, allowFallback: a[3] as bool);
    case 'addImage':
      return doc.addImage(a[0] as int, (a[1] as TransferableTypedData).materialize().asUint8List(),
          x: a[2] as double, y: a[3] as double, width: a[4] as double, height: a[5] as double?);
    case 'rotatePage':
      doc.rotatePage(a[0] as int, a[1] as int);
      return null;
    case 'deletePage':
      doc.deletePage(a[0] as int);
      return null;
    case 'movePage':
      doc.movePage(a[0] as int, a[1] as int);
      return null;
    case 'insertPages':
      return doc.insertPages((a[0] as TransferableTypedData).materialize().asUint8List(),
          at: a[1] as int?, pages: (a[2] as List?)?.cast<int>(), password: a[3] as String?);
    case 'extractPages':
      return TransferableTypedData.fromList([doc.extractPages((a[0] as List).cast<int>())]);
    case 'signatures':
      return doc.signatures();
    case 'addText':
      return doc.addText(a[0] as int, a[1] as NewText);
    case 'setField':
      doc.setField(a[0] as String, a[1] as String, ignoreReadOnly: a[2] as bool);
      return null;
    case 'undo':
      return doc.undo();
    case 'redo':
      return doc.redo();
    case 'history':
      return doc.history;
    case 'save':
      return TransferableTypedData.fromList([doc.save()]);
    case 'close':
      doc.close();
      return null;
  }
  throw PdfException('bad_argument', 'operación desconocida: $op');
}
