/// Plain data, no Flutter types, so the package works anywhere Dart does.
/// A box is `[x0, y0, x1, y1]` in points on the page *as shown* (origin at the
/// top-left, y down); multiply by your zoom to get screen pixels.
library;

class PdfException implements Exception {
  PdfException(this.kind, this.message, {this.chars = '', this.reason});

  /// `pdf`, `no_such_page`, `missing_glyphs`, `no_such_field`, `rejected`,
  /// `unsupported`, `not_editable`, `password_required`, `wrong_password`,
  /// `stale_run`, `stale_item`, `stale_block`, `bad_argument` or `panic`.
  final String kind;
  final String message;

  /// For `missing_glyphs`: the characters no available font can draw.
  final String chars;

  /// For `not_editable` (JSON `reason`): why the text cannot be edited, one of
  /// `type3`, `vertical_writing`, `symbolic_font` or `rtl_text`. The text can
  /// still be read, moved and aligned.
  final String? reason;
  bool get isNotEditable => kind == 'not_editable';

  /// The run ids of the last reading are no longer valid: read the page again.
  bool get isStaleRun => kind == 'stale_run';
  /// The block ids of the last `blocks` reading are no longer valid.
  bool get isStaleBlock => kind == 'stale_block';
  bool get isMissingGlyphs => kind == 'missing_glyphs';

  factory PdfException.fromJson(Map<String, dynamic> json) => PdfException(
        json['kind'] as String? ?? 'unknown',
        json['error'] as String? ?? 'error desconocido',
        chars: json['chars'] as String? ?? '',
        reason: json['reason'] as String?,
      );

  @override
  String toString() => 'PdfException($kind): $message';
}

List<double> _box(Object? raw) =>
    (raw as List).map((v) => (v as num).toDouble()).toList(growable: false);

class TextRun {
  TextRun({
    required this.id,
    required this.text,
    required this.font,
    required this.baseFont,
    required this.size,
    required this.display,
    required this.origin,
    required this.invisible,
    this.editable = true,
    this.notEditable,
    this.fontEmbedded = false,
    this.fontKind = 'not_embedded',
    this.substituteFont = '',
    this.substituteSimilarity = 100,
  });

  /// Names this run in the *last reading of its page*; any edit, undo or redo
  /// invalidates it.
  final int id;
  final String text;
  final String font;
  final String baseFont;
  final double size;
  final List<double> display;
  final List<double> origin;

  /// Invisible text: the hidden layer of a scanned page.
  final bool invisible;

  /// JSON `editable` / `not_editable`: false, with the reason (`type3`,
  /// `vertical_writing`, `symbolic_font` or `rtl_text`), when the text can be read
  /// but `replaceText` would refuse it with a `not_editable` error.
  final bool editable;
  final String? notEditable;

  /// JSON `font_embedded`: the font program is in the file (subset or whole).
  final bool fontEmbedded;

  /// JSON `font_kind`: `embedded_full`, `embedded_subset`, `standard`,
  /// `not_embedded` or `type3`. Use it to warn that a font cannot be reproduced
  /// exactly (`not_embedded`, `embedded_subset`).
  final String fontKind;

  /// JSON `substitute_font` / `substitute_similarity`: the font that would draw letters this run's
  /// font lacks (a registered or bundled one) and how close it is, 0-100. Under 90 it is a
  /// different face: say so before the person types a letter the font does not have.
  final String substituteFont;
  final int substituteSimilarity;

  factory TextRun.fromJson(Map<String, dynamic> j) => TextRun(
        id: j['id'] as int,
        text: j['text'] as String,
        font: j['font'] as String,
        baseFont: j['base_font'] as String,
        size: (j['size'] as num).toDouble(),
        display: _box(j['display']),
        origin: _box(j['origin']),
        invisible: j['invisible'] as bool,
        editable: j['editable'] as bool? ?? true,
        notEditable: j['not_editable'] as String?,
        fontEmbedded: j['font_embedded'] as bool? ?? false,
        fontKind: j['font_kind'] as String? ?? 'not_embedded',
        substituteFont: j['substitute_font'] as String? ?? '',
        substituteSimilarity: (j['substitute_similarity'] as num?)?.toInt() ?? 100,
      );
}

class PageData {
  PageData(this.index, this.width, this.height, this.rotation, this.runs);
  final int index;
  final double width;
  final double height;
  final int rotation;
  final List<TextRun> runs;

  factory PageData.fromJson(int index, Map<String, dynamic> j) => PageData(
        index,
        (j['width'] as num).toDouble(),
        (j['height'] as num).toDouble(),
        j['rotation'] as int,
        (j['runs'] as List).map((r) => TextRun.fromJson(r as Map<String, dynamic>)).toList(),
      );
}

/// What happens to the rest of the line when a replacement changes its length.
enum Reflow {
  /// Nothing moves: a longer text may overlap what follows.
  none,

  /// What follows on the line moves by the difference. For left-aligned text.
  following,

  /// The line keeps its centre.
  centered,

  /// The line keeps its right edge.
  rightAligned,
}

/// Something about an edit's result that the person may want to know.
///
/// `kind` is one of `exceeds_page` and `exceeds_margin` (`by`: points past the
/// edge), `follower_not_moved` (`text`: a run inside a form XObject, which can
/// be read but not edited, stayed where it was), `line_may_be_aligned`
/// (`alignment`: `centered` or `right`; consider [Reflow.centered] or
/// [Reflow.rightAligned]), `reflow_failed`, and, for a re-laid paragraph,
/// `overlaps_text` (`text`: what the new lines now cover) and `below_page`
/// (`by`: points past the page's bottom edge).
class PdfWarning {
  PdfWarning(this.kind, {this.by, this.text, this.alignment});
  final String kind;
  final double? by;
  final String? text;
  final String? alignment;

  factory PdfWarning.fromJson(Map<String, dynamic> j) => PdfWarning(
        j['kind'] as String,
        by: (j['by'] as num?)?.toDouble(),
        text: j['text'] as String?,
        alignment: j['alignment'] as String?,
      );

  @override
  String toString() => 'PdfWarning($kind${by != null ? ', by: $by' : ''}${text != null ? ', text: $text' : ''})';
}

class EditResult {
  EditResult(this.oldWidth, this.newWidth, this.fallbackChars, this.reread,
      {this.moved = 0, this.warnings = const [], this.substitutes = const []});
  final double oldWidth;
  final double newWidth;

  /// How many other runs of the line were moved to make room (or close the gap).
  final int moved;

  /// Things worth telling the person: the line now runs past the margin, a run
  /// that could not be moved, a line that looks centred...
  final List<PdfWarning> warnings;

  /// Characters drawn with the bundled font because the document's own could
  /// not: worth telling the person.
  final String fallbackChars;

  /// The page's operations were renumbered: read it again.
  final bool reread;

  /// Every letter the original font lacked and how it was drawn (JSON `substitutes`).
  final List<FontSubstitute> substitutes;

  factory EditResult.fromJson(Map<String, dynamic> j) => EditResult(
        (j['old_width'] as num).toDouble(),
        (j['new_width'] as num).toDouble(),
        j['fallback_chars'] as String,
        j['reread'] as bool,
        substitutes: FontSubstitute.list(j['substitutes']),
        moved: (j['moved'] as int?) ?? 0,
        warnings: ((j['warnings'] as List?) ?? const [])
            .map((w) => PdfWarning.fromJson(w as Map<String, dynamic>))
            .toList(),
      );
}

enum Anchor { topLeft, baseline }

enum TextAlign { left, center, right }

class NewText {
  NewText(
    this.text,
    this.x,
    this.y, {
    this.anchor = Anchor.topLeft,
    this.size = 12,
    this.color = const [0, 0, 0],
    this.serif = false,
    this.bold = false,
    this.italic = false,
    this.align = TextAlign.left,
    this.maxWidth,
  });

  final String text;
  final double x;
  final double y;
  final Anchor anchor;
  final double size;

  /// Red, green, blue, each 0..1.
  final List<double> color;
  final bool serif;
  final bool bold;
  final bool italic;
  final TextAlign align;
  final double? maxWidth;

  Map<String, Object?> toJson() => {
        'text': text,
        'x': x,
        'y': y,
        'anchor': anchor == Anchor.baseline ? 'baseline' : 'top_left',
        'size': size,
        'color': color,
        'serif': serif,
        'bold': bold,
        'italic': italic,
        'align': align.name,
        if (maxWidth != null) 'max_width': maxWidth,
      };
}

class AddedText {
  AddedText(this.display, this.lines);
  final List<double> display;
  final int lines;
}

enum FieldKind { text, checkbox, radio, pushbutton, combo, list, signature }

class FieldOption {
  FieldOption(this.export, this.label);
  final String export;
  final String label;
}

class FieldWidget {
  FieldWidget(this.page, this.display, this.onState, {this.hasAppearance = true});
  final int? page;
  final List<double> display;
  final String? onState;

  /// It carries an appearance stream, which is what a reader draws.
  final bool hasAppearance;
}

/// How `arrangeFields` lines fields up.
enum Arrange {
  alignLeft('align_left'),
  alignRight('align_right'),
  alignTop('align_top'),
  alignBottom('align_bottom'),

  /// Centres share a vertical line.
  centerHorizontally('center_horizontally'),

  /// Centres share a horizontal line.
  centerVertically('center_vertically'),

  /// Equal gaps from left to right (three fields or more).
  distributeHorizontally('distribute_horizontally'),

  /// Equal gaps from top to bottom (three fields or more).
  distributeVertically('distribute_vertically'),
  sameWidth('same_width'),
  sameHeight('same_height');

  const Arrange(this.code);
  final String code;
}

/// One widget of one field: a field that appears in several places has several.
class FieldRefs {
  const FieldRefs(this.name, [this.widget = 0]);
  final String name;
  final int widget;
}

enum PdfItemKind { text, image, field }

/// Something a person moves around as a unit, like in a design tool: a block of
/// text, a picture or a form field. Read again after any edit.
class PdfItem {
  PdfItem(this.id, this.kind, this.display, this.text, {this.lines = 0, this.fieldName, this.widget = 0});
  final int id;
  final PdfItemKind kind;

  /// `[x0, y0, x1, y1]` as a reader shows the page.
  final List<double> display;

  /// A text's words, or a field's value.
  final String text;

  /// For text: how many lines it has (more than one for a paragraph).
  final int lines;
  final String? fieldName;
  final int widget;
}

/// How far to move an item, in points as the page is shown.
class ItemMove {
  const ItemMove(this.item, this.dx, this.dy);
  final PdfItem item;
  final double dx;
  final double dy;
}

/// Several lines that read as one block of text. [text] is all of it: show it
/// in one editing box and hand the result to `replaceParagraph`.
class PdfParagraph {
  PdfParagraph(this.text, this.runIds, this.display);
  final String text;

  /// The [TextRun.id] of each line, top to bottom.
  final List<int> runIds;
  final List<double> display;
}

class ParagraphResult {
  ParagraphResult(this.lines, this.added, this.removed, this.fallbackChars, this.warnings);

  /// Lines the paragraph has now.
  final int lines;

  /// Lines added below the old last one, and lines at the end emptied.
  final int added;
  final int removed;
  final String fallbackChars;
  final List<PdfWarning> warnings;
}

/// Where a picture ended up.
class AddedImage {
  AddedImage(this.display, this.pixelWidth, this.pixelHeight);

  /// The box it occupies, `[x0, y0, x1, y1]` as a reader shows the page.
  final List<double> display;

  /// Size in pixels as the picture is meant to be seen (a photo's EXIF turn applied).
  final int pixelWidth;
  final int pixelHeight;
}

/// One word of a [PdfLine], with the box to highlight it.
class PdfWord {
  PdfWord(this.text, this.display);
  final String text;

  /// `[x0, y0, x1, y1]` as a reader shows it (top-left origin, y down).
  final List<double> display;
}

/// Text as a person reads it: the strings on one baseline joined into a line of
/// words (a bold label and the value after it are one line).
class PdfLine {
  PdfLine(this.text, this.display, this.runIds, this.words, this.invisible);
  final String text;
  final List<double> display;

  /// The [TextRun.id]s that make up the line, valid like the page reading they come from.
  final List<int> runIds;
  final List<PdfWord> words;

  /// The text layer of a scanned page.
  final bool invisible;
}

/// How [PdfDocument.find] compares text. By default case and accents are ignored.
class SearchOptions {
  const SearchOptions({this.caseSensitive = false, this.wholeWord = false, this.accentSensitive = false});
  final bool caseSensitive;
  final bool wholeWord;
  final bool accentSensitive;

  int get flags => (caseSensitive ? 1 : 0) | (wholeWord ? 2 : 0) | (accentSensitive ? 4 : 0);
}

class SearchMatch {
  SearchMatch(this.page, this.text, this.line, this.display, this.runId, this.withinRun);
  final int page;

  /// The text that matched, as the page has it.
  final String text;

  /// The line it was found in.
  final String line;

  /// Box to highlight, `[x0, y0, x1, y1]` as a reader shows it.
  final List<double> display;

  /// Id of the run it starts in (valid after reading [page]) and the character
  /// range of that run it covers, or null if it runs on into the next string.
  final int runId;
  final (int, int)? withinRun;
}

class ReplaceAllResult {
  ReplaceAllResult(this.replaced, this.skipped);

  /// Occurrences changed.
  final int replaced;

  /// Occurrences left alone: the text crosses two strings or the font lacks a letter.
  final int skipped;
}

/// A completed digital signature. Any edit invalidates it.
class PdfSignature {
  PdfSignature(this.field, this.signer);
  final String field;
  final String? signer;
}

class FormField {
  FormField({
    required this.name,
    required this.kind,
    required this.value,
    required this.checked,
    required this.readOnly,
    required this.required,
    required this.multiline,
    required this.password,
    required this.comb,
    required this.editable,
    required this.multiSelect,
    required this.order,
    required this.maxLen,
    required this.options,
    required this.widgets,
    this.signed = false,
    this.signer,
  });

  final String name;
  final FieldKind kind;
  final String value;
  final bool checked;

  /// The form marks it read-only. Some forms do so by mistake; filling it needs
  /// `ignoreReadOnly: true`, which is for the person to decide.
  final bool readOnly;
  final bool required;
  final bool multiline;
  final bool password;
  final bool comb;
  final bool editable;

  /// A list box that allows several rows; set its value with one option per line.
  final bool multiSelect;

  /// Position in the order a person fills the form (page, then top to bottom,
  /// then left to right): sort by it for a "next field" button.
  final int order;
  final int? maxLen;
  final List<FieldOption> options;
  final List<FieldWidget> widgets;

  /// A signature field holding a completed signature.
  final bool signed;
  final String? signer;

  factory FormField.fromJson(Map<String, dynamic> j) => FormField(
        name: j['name'] as String,
        kind: FieldKind.values.firstWhere((k) => k.name == j['kind'], orElse: () => FieldKind.text),
        value: j['value'] as String,
        checked: j['checked'] as bool,
        readOnly: j['read_only'] as bool,
        required: j['required'] as bool,
        multiline: j['multiline'] as bool,
        password: j['password'] as bool,
        comb: j['comb'] as bool,
        editable: j['editable'] as bool,
        multiSelect: j['multi_select'] as bool,
        order: (j['order'] as num).toInt(),
        maxLen: j['max_len'] as int?,
        options: (j['options'] as List)
            .map((o) => FieldOption((o as Map)['export'] as String, o['label'] as String))
            .toList(),
        widgets: (j['widgets'] as List)
            .map((w) => FieldWidget((w as Map)['page'] as int?, _box(w['display']), w['on_state'] as String?,
                hasAppearance: (w['has_appearance'] as bool?) ?? true))
            .toList(),
        signed: (j['signed'] as bool?) ?? false,
        signer: j['signer'] as String?,
      );
}

class History {
  History(this.canUndo, this.canRedo);
  final bool canUndo;
  final bool canRedo;
}

/// A stretch of a [PdfBlock]'s text in one style (JSON `spans`). `start`/`end`
/// count characters of [PdfBlock.text], `[start, end)`.
class StyleSpan {
  StyleSpan(this.start, this.end, this.font, this.baseFont, this.size, this.color, this.bold, this.italic);
  final int start;
  final int end;
  final String font;
  final String baseFont;
  final double size;

  /// Red, green, blue in 0..1 when the fill is a plain colour.
  final List<double>? color;

  /// From the font's name; show them in your editor as B / I.
  final bool bold;
  final bool italic;

  factory StyleSpan.fromJson(Map<String, dynamic> j) => StyleSpan(
        (j['start'] as num).toInt(),
        (j['end'] as num).toInt(),
        j['font'] as String,
        j['base_font'] as String,
        (j['size'] as num).toDouble(),
        j['color'] == null ? null : _box(j['color']),
        j['bold'] as bool,
        j['italic'] as bool,
      );
}

class BlockLine {
  BlockLine(this.text, this.display, this.baseline);
  final String text;
  final List<double> display;
  final double baseline;
}

/// A paragraph (or a single line) as ONE unit, even when it mixes fonts, sizes
/// or colours: edit it with [PdfDocument.replaceBlock]; every letter that stays
/// keeps its style, typed letters take the style of the letter before them.
class PdfBlock {
  PdfBlock(this.id, this.text, this.display, this.lines, this.spans, this.justified, this.firstLineIndent,
      this.editable, this.reason);

  /// Good until the next edit (then `stale_block`).
  final int id;

  /// The lines joined by single spaces.
  final String text;
  final List<double> display;
  final List<BlockLine> lines;
  final List<StyleSpan> spans;

  /// Re-laid text is justified too.
  final bool justified;

  /// Negative: hanging.
  final double firstLineIndent;
  final bool editable;

  /// When not [editable]: `type3`, `vertical_writing`, `symbolic_font` or `rtl_text`.
  final String? reason;

  factory PdfBlock.fromJson(Map<String, dynamic> j) => PdfBlock(
        (j['id'] as num).toInt(),
        j['text'] as String,
        _box(j['display']),
        (j['lines'] as List)
            .map((l) => BlockLine(l['text'] as String, _box(l['display']), (l['baseline'] as num).toDouble()))
            .toList(),
        (j['spans'] as List).map((s) => StyleSpan.fromJson(s as Map<String, dynamic>)).toList(),
        j['justified'] as bool,
        (j['first_line_indent'] as num).toDouble(),
        j['editable'] as bool,
        j['reason'] as String?,
      );
}

/// What `replaceBlock` did (JSON `lines`, `kept`, `rewritten`, `added`, `removed`,
/// `fallback_chars`, `warnings`). Lines [kept] are not touched at all.
class BlockResult {
  BlockResult(this.lines, this.kept, this.rewritten, this.added, this.removed, this.fallbackChars, this.warnings,
      {this.substitutes = const []});
  final List<FontSubstitute> substitutes;
  final int lines;
  final int kept;
  final int rewritten;
  final int added;
  final int removed;
  final String fallbackChars;
  final List<PdfWarning> warnings;
}

/// A letter the document's font could not draw, and what drew it instead (JSON
/// `{"char","via"}`). `via` is `encoding` (added to the font's own encoding: the
/// reader draws it in the same face), `font_program` (the embedded font has the glyph, the
/// document just did not list it: exactly the same face) or `bundled_font` (a Liberation font of the
/// closest style). Tell the person about every one.
class FontSubstitute {
  FontSubstitute(this.char, this.via, {this.font = '', this.similarity = 100});
  final String char;
  final String via;

  /// The font that drew it (the original's name for `encoding`, e.g. `Carlito-Regular` otherwise).
  final String font;

  /// 0-100: how close that face is to the original. 100 is a metric clone; under 90 tell the
  /// person it is a different face.
  final int similarity;
  bool get isDifferentFace => similarity < 90;
  bool get isBundledFont => via == 'bundled_font';

  static List<FontSubstitute> list(Object? raw) => ((raw as List?) ?? const [])
      .map((s) => FontSubstitute((s as Map)['char'] as String, s['via'] as String,
          font: (s['font'] as String?) ?? '', similarity: (s['similarity'] as num?)?.toInt() ?? 100))
      .toList();
}

/// What to do with a run that has letters its font lacks, when the bundled font is allowed.
enum Substitution {
  /// Only those letters (default).
  perChar,

  /// The whole run, so no word mixes two faces.
  wholeRun,

  /// [wholeRun] unless the bundled face is a metric clone of the original.
  auto,
}

/// A free font that looks like one the document uses (JSON `source`).
class FontSource {
  FontSource(this.name, this.license, this.url);
  final String name;
  final String license;
  final String url;
}

/// A font the document uses and what editing it exactly needs (`pdfcore_required_fonts`).
class RequiredFont {
  RequiredFont(this.baseFont, this.kind, this.pages, this.runs, this.needed, this.registered, this.bundled,
      this.similarity, this.source);
  final String baseFont;

  /// `embedded_full`, `embedded_subset`, `standard`, `not_embedded` or `type3`.
  final String kind;
  final List<int> pages;
  final int runs;

  /// The real font is wanted to edit this exactly (not embedded, or only in part).
  final bool needed;

  /// A font of this family was handed over with `registerFont`.
  final bool registered;

  /// The bundled font that stands in for it, and how close it is (0-100).
  final String bundled;
  final int similarity;
  final FontSource? source;

  factory RequiredFont.fromJson(Map<String, dynamic> j) => RequiredFont(
        j['base_font'] as String,
        j['kind'] as String,
        (j['pages'] as List).map((p) => (p as num).toInt()).toList(),
        (j['runs'] as num).toInt(),
        j['needed'] as bool,
        j['registered'] as bool,
        j['bundled'] as String,
        (j['similarity'] as num).toInt(),
        j['source'] == null
            ? null
            : FontSource(j['source']['name'] as String, j['source']['license'] as String, j['source']['url'] as String),
      );
}

/// What `registerFont` understood about the font (JSON `family`, `bold`, `italic`).
class RegisteredFont {
  RegisteredFont(this.family, this.bold, this.italic);
  final String family;
  final bool bold;
  final bool italic;
}

/// What is under a point of a page (`hitTest`). Ids are those of the current readings.
class PdfHit {
  PdfHit(this.kind, this.item, this.display, {this.fieldName, this.widget = 0, this.block, this.char, this.caret});
  final PdfItemKind kind;

  /// Id in the page's `items()`.
  final int item;
  final List<double> display;
  final String? fieldName;
  final int widget;

  /// For text: id in the page's `blocks()`, the letter under the point, and the insertion point
  /// (0..length, between letters) nearest to it.
  final int? block;
  final int? char;
  final int? caret;
}

/// What `refreshAppearances` did.
class RefreshResult {
  RefreshResult(this.rebuilt, this.skipped);
  final int rebuilt;
  final int skipped;
}

/// One edit of a `batch`: they all happen as ONE undo step, or none does. Each points at ids of the
/// last readings as they were when the batch began, and the ops of a batch must point at different
/// things.
class BatchOp {
  const BatchOp._(this.json);
  final Map<String, dynamic> json;

  factory BatchOp.replaceText(int page, TextRun run, String text, {bool allowFallback = false}) =>
      BatchOp._({'op': 'replace_text', 'page': page, 'run': run.id, 'text': text, 'allow_fallback': allowFallback});

  factory BatchOp.replaceBlock(int page, PdfBlock block, String text,
          {bool allowFallback = false, bool? justify, bool pushDown = false, double? width}) =>
      BatchOp._({
        'op': 'replace_block', 'page': page, 'block': block.id, 'text': text,
        'allow_fallback': allowFallback, 'justify': justify, 'push_down': pushDown, 'width': width,
      });

  factory BatchOp.replaceRange(int page, PdfBlock block, int start, int end, String text, {bool allowFallback = false}) =>
      BatchOp._({
        'op': 'replace_range', 'page': page, 'block': block.id, 'start': start, 'end': end, 'text': text,
        'allow_fallback': allowFallback,
      });

  factory BatchOp.setBlockStyle(int page, PdfBlock block, int start, int end,
          {bool? bold, bool? italic, double? size, List<double>? color, bool allowFallback = false}) =>
      BatchOp._({
        'op': 'set_block_style', 'page': page, 'block': block.id, 'start': start, 'end': end,
        'bold': bold, 'italic': italic, 'size': size, 'color': color, 'allow_fallback': allowFallback,
      });

  factory BatchOp.moveItems(int page, List<ItemMove> moves) => BatchOp._({
        'op': 'move_items', 'page': page,
        'moves': [for (final m in moves) {'id': m.item.id, 'dx': m.dx, 'dy': m.dy}],
      });

  factory BatchOp.deleteItems(int page, List<PdfItem> items) =>
      BatchOp._({'op': 'delete_items', 'page': page, 'ids': [for (final i in items) i.id]});

  factory BatchOp.resizeItem(int page, PdfItem item, List<double> box) =>
      BatchOp._({'op': 'resize_item', 'page': page, 'id': item.id, 'box': box});

  factory BatchOp.setField(String name, String value) => BatchOp._({'op': 'set_field', 'name': name, 'value': value});
}

/// An annotation of a page (`annotations`). Indexes are good until the next edit.
class PdfAnnotation {
  PdfAnnotation(this.index, this.kind, this.subtype, this.display, this.contents, this.author, this.color, this.quads,
      this.hasAppearance, this.hidden);

  /// Position in the page's list: pass it to `deleteAnnotation`, `moveAnnotation`...
  final int index;

  /// `highlight`, `underline`, `strike_out`, `squiggly`, `note`, `free_text`, `ink`, `square`,
  /// `circle`, `line`, `link`, `stamp`, `popup`, `widget` or `other`.
  final String kind;
  final String subtype;
  final List<double> display;
  final String contents;
  final String author;
  final List<double>? color;

  /// For text markup: the boxes it covers.
  final List<List<double>> quads;
  final bool hasAppearance;
  final bool hidden;

  factory PdfAnnotation.fromJson(Map<String, dynamic> j) => PdfAnnotation(
        (j['index'] as num).toInt(),
        j['kind'] as String,
        j['subtype'] as String,
        _box(j['display']),
        j['contents'] as String,
        j['author'] as String,
        j['color'] == null ? null : _box(j['color']),
        [for (final q in j['quads'] as List) _box(q)],
        j['has_appearance'] as bool,
        j['hidden'] as bool,
      );
}

/// What to add with `addAnnotation`; boxes and points are as the page is shown.
class NewAnnotation {
  const NewAnnotation._(this.json);
  final Map<String, dynamic> json;

  static Map<String, dynamic> _common(List<double> color, String contents, String author, double opacity) =>
      {'color': color, 'contents': contents, 'author': author, 'opacity': opacity};

  factory NewAnnotation.markup(String shape, List<List<double>> boxes,
          {List<double> color = const [1, 0.9, 0.2], String contents = '', String author = '', double opacity = 1}) =>
      NewAnnotation._({'shape': shape, 'boxes': boxes, ..._common(color, contents, author, opacity)});

  /// `shape` is `highlight`, `underline`, `strike_out` or `squiggly`.
  factory NewAnnotation.highlight(List<List<double>> boxes, {List<double> color = const [1, 0.9, 0.2], String contents = ''}) =>
      NewAnnotation.markup('highlight', boxes, color: color, contents: contents);

  factory NewAnnotation.note(double x, double y, String contents, {List<double> color = const [1, 0.9, 0.2], String author = ''}) =>
      NewAnnotation._({'shape': 'note', 'at': [x, y], ..._common(color, contents, author, 1)});

  factory NewAnnotation.freeText(List<double> rect, String contents,
          {double size = 12, List<double> color = const [0, 0, 0], String author = ''}) =>
      NewAnnotation._({'shape': 'free_text', 'rect': rect, 'size': size, ..._common(color, contents, author, 1)});

  factory NewAnnotation.ink(List<List<List<double>>> strokes,
          {double width = 2, List<double> color = const [0, 0, 0], double opacity = 1}) =>
      NewAnnotation._({'shape': 'ink', 'strokes': strokes, 'width': width, ..._common(color, '', '', opacity)});

  factory NewAnnotation.rectangle(List<double> rect,
          {double width = 1.5, List<double> color = const [1, 0, 0], List<double>? fill}) =>
      NewAnnotation._({'shape': 'square', 'rect': rect, 'width': width, 'fill': fill, ..._common(color, '', '', 1)});

  factory NewAnnotation.ellipse(List<double> rect,
          {double width = 1.5, List<double> color = const [1, 0, 0], List<double>? fill}) =>
      NewAnnotation._({'shape': 'circle', 'rect': rect, 'width': width, 'fill': fill, ..._common(color, '', '', 1)});

  factory NewAnnotation.line(List<double> from, List<double> to, {double width = 2, List<double> color = const [1, 0, 0]}) =>
      NewAnnotation._({'shape': 'line', 'from': from, 'to': to, 'width': width, ..._common(color, '', '', 1)});
}

/// What `flatten` did.
class FlattenResult {
  FlattenResult(this.drawn, this.removed, this.skipped);
  final int drawn;
  final int removed;

  /// Left alone: signature fields.
  final int skipped;
}

/// What `redact` did. [imagesKept] are pictures that touch a box and were only covered: their
/// data is still in the file (ask for `removeImages` to take them out whole).
class RedactResult {
  RedactResult(this.chars, this.runs, this.imagesRemoved, this.imagesKept, this.annotationsRemoved);
  final int chars;
  final int runs;
  final int imagesRemoved;
  final int imagesKept;
  final int annotationsRemoved;
}

/// What a page holds (`ocrReport`).
class PageReport {
  PageReport(this.page, this.kind, this.visibleChars, this.invisibleChars, this.images, this.imageCoverage, this.needsOcr);
  final int page;

  /// `native`, `scanned_no_text`, `scanned_with_text_layer`, `image_only` or `blank`.
  final String kind;
  final int visibleChars;
  final int invisibleChars;
  final int images;
  final double imageCoverage;

  /// Offer to run OCR on this page: it is a picture with no text. The engine recognises no characters.
  final bool needsOcr;
}

/// A vector graphic a page paints (`graphics`): a rectangle (check box, table cell, frame), a line
/// or another path. Coordinates are as the page is shown (top-left origin, y down), with the
/// page's rotation and every transformation applied. Letters drawn as outlines are graphics too:
/// filter by size when only boxes matter.
class PdfGraphic {
  PdfGraphic(this.kind, this.display, this.filled, this.stroked, this.fill, this.stroke, this.lineWidth, this.xobject,
      this.ends);

  /// `rect`, `line` or `path`.
  final String kind;

  /// `[x0, y0, x1, y1]` of the path itself, without the stroke's width.
  final List<double> display;
  final bool filled;
  final bool stroked;

  /// Red, green, blue in 0..1, when the colour is a plain one.
  final List<double>? fill;
  final List<double>? stroke;
  final double lineWidth;

  /// The form XObject that paints it, if any.
  final String? xobject;

  /// For a line: `[[x0, y0], [x1, y1]]`.
  final List<List<double>>? ends;

  bool get isRect => kind == 'rect';
  bool get isLine => kind == 'line';
  double get width => display[2] - display[0];
  double get height => display[3] - display[1];

  factory PdfGraphic.fromJson(Map<String, dynamic> j) => PdfGraphic(
        j['kind'] as String,
        _box(j['display']),
        j['filled'] as bool,
        j['stroked'] as bool,
        j['fill'] == null ? null : _box(j['fill']),
        j['stroke'] == null ? null : _box(j['stroke']),
        (j['line_width'] as num).toDouble(),
        j['xobject'] as String?,
        j['ends'] == null ? null : [for (final e in j['ends'] as List) _box(e)],
      );
}
