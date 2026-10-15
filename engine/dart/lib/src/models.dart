/// Plain data, no Flutter types, so the package works anywhere Dart does.
/// A box is `[x0, y0, x1, y1]` in points on the page *as shown* (origin at the
/// top-left, y down); multiply by your zoom to get screen pixels.
library;

class PdfException implements Exception {
  PdfException(this.kind, this.message, {this.chars = '', this.reason});

  /// `pdf`, `no_such_page`, `missing_glyphs`, `no_such_field`, `rejected`,
  /// `unsupported`, `not_editable`, `password_required`, `wrong_password`,
  /// `stale_run`, `stale_item`, `bad_argument` or `panic`.
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
      {this.moved = 0, this.warnings = const []});
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

  factory EditResult.fromJson(Map<String, dynamic> j) => EditResult(
        (j['old_width'] as num).toDouble(),
        (j['new_width'] as num).toDouble(),
        j['fallback_chars'] as String,
        j['reread'] as bool,
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
