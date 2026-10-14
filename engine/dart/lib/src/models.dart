/// Plain data, no Flutter types, so the package works anywhere Dart does.
/// A box is `[x0, y0, x1, y1]` in points on the page *as shown* (origin at the
/// top-left, y down); multiply by your zoom to get screen pixels.
library;

class PdfException implements Exception {
  PdfException(this.kind, this.message, {this.chars = ''});

  /// `pdf`, `no_such_page`, `missing_glyphs`, `no_such_field`, `rejected`,
  /// `unsupported`, `stale_run`, `bad_argument` or `panic`.
  final String kind;
  final String message;

  /// For `missing_glyphs`: the characters no available font can draw.
  final String chars;

  /// The run ids of the last reading are no longer valid: read the page again.
  bool get isStaleRun => kind == 'stale_run';
  bool get isMissingGlyphs => kind == 'missing_glyphs';

  factory PdfException.fromJson(Map<String, dynamic> json) => PdfException(
        json['kind'] as String? ?? 'unknown',
        json['error'] as String? ?? 'error desconocido',
        chars: json['chars'] as String? ?? '',
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

  factory TextRun.fromJson(Map<String, dynamic> j) => TextRun(
        id: j['id'] as int,
        text: j['text'] as String,
        font: j['font'] as String,
        baseFont: j['base_font'] as String,
        size: (j['size'] as num).toDouble(),
        display: _box(j['display']),
        origin: _box(j['origin']),
        invisible: j['invisible'] as bool,
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
/// [Reflow.rightAligned]) and `reflow_failed`.
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
