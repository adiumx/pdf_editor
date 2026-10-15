/// Measuring the engine on a document, on the device it will run on.
///
/// Call [benchmarkDocument] from the app (a debug button, a test on the phone) with the bytes of a
/// real PDF and send or show the result: the times are for THAT device, which nothing in this
/// repository can measure. The same function runs from a terminal with `dart run tool/benchmark.dart`.
library;

import 'dart:io';
import 'dart:typed_data';

import 'src/document.dart';

/// Milliseconds of each step, and memory in MB, for the document [pdf]. Steps that do not apply
/// (no text, no fields) are left out. Nothing is written anywhere; the document is closed.
Map<String, Object> benchmarkDocument(Uint8List pdf, {String? libraryPath, int repeat = 3}) {
  final watch = Stopwatch();
  final steps = <String, double>{};
  double time(String name, void Function() body) {
    watch
      ..reset()
      ..start();
    body();
    watch.stop();
    final ms = watch.elapsedMicroseconds / 1000.0;
    // Keep the best of the repeats: the first run also pays for loading and cold caches.
    steps[name] = steps.containsKey(name) && steps[name]! < ms ? steps[name]! : ms;
    return ms;
  }

  final rssBefore = ProcessInfo.currentRss;
  PdfDocument? doc;
  final first = <String, double>{};
  for (var i = 0; i < repeat; i++) {
    doc?.close();
    first['open'] = time('open', () => doc = PdfDocument.open(pdf, libraryPath: libraryPath));
    final d = doc!;
    time('page', () => d.page(0));
    time('blocks', () => d.blocks(0));
    time('items', () => d.items(0));
    time('lines', () => d.lines(0));
    time('ocr_report', d.ocrReport);
    time('required_fonts', d.requiredFonts);
    final page = d.page(0);
    final runs = page.runs.where((r) => r.editable && r.text.trim().isNotEmpty).toList();
    if (runs.isNotEmpty) {
      final run = runs.first;
      time('replace_text', () => d.replaceText(page, run, '${run.text} '));
      time('undo', d.undo);
    }
    final blocks = d.blocks(0).where((b) => b.editable).toList();
    if (blocks.isNotEmpty) {
      final block = blocks.first;
      time('replace_block', () => d.replaceBlock(0, block, '${block.text} añadido'));
      time('undo_block', d.undo);
    }
    time('save', d.save);
    time('save_incremental', d.saveIncremental);
  }
  final result = <String, Object>{
    'bytes': pdf.length,
    'pages': doc!.pageCount,
    'repeat': repeat,
    'first_open_ms': first['open']!,
    'best_ms': steps,
    'rss_mb_before': rssBefore / (1024 * 1024),
    'rss_mb_after': ProcessInfo.currentRss / (1024 * 1024),
    'rss_mb_peak': ProcessInfo.maxRss / (1024 * 1024),
  };
  doc!.close();
  return result;
}
