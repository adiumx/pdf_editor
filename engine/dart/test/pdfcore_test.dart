// Run with the library built first:
//   cargo build -p pdfcore-ffi && cd dart && dart test
import 'dart:io';
import 'dart:typed_data';

import 'package:pdfcore/benchmark.dart';
import 'package:pdfcore/pdfcore.dart';
import 'package:test/test.dart';

final engine = Directory.current.parent;
final libraryPath = Platform.environment['PDFCORE_LIB'] ?? '${engine.path}/target/debug/libpdfcore_ffi.so';

Uint8List fixture(String name) => File('${engine.path}/pdfcore/tests/fixtures/$name.pdf').readAsBytesSync();

PdfDocument open(String name) => PdfDocument.open(fixture(name), libraryPath: libraryPath);

void main() {
  group('PdfDocument', () {
    late PdfDocument doc;
    setUp(() => doc = open('standard'));
    tearDown(() => doc.close());

    test('reads pages and runs', () {
      expect(doc.pageCount, 1);
      final page = doc.page(0);
      expect([page.width, page.height, page.rotation], [595.0, 842.0, 0]);
      expect(page.runs.first.text, 'Hola mundo');
      expect(page.runs.first.baseFont, 'Helvetica');
      expect(page.runs.first.display, hasLength(4));
      expect(page.runs.first.invisible, isFalse);
    });

    test('a file that is not a PDF throws with a reason', () {
      expect(
        () => PdfDocument.open(Uint8List.fromList('esto no es un pdf'.codeUnits), libraryPath: libraryPath),
        throwsA(isA<PdfException>().having((e) => e.kind, 'kind', 'pdf')),
      );
    });

    test('replaces text and the old run ids stop meaning anything', () {
      final page = doc.page(0);
      final done = doc.replaceText(page, page.runs[0], 'Adiós');
      expect(done.newWidth, lessThan(done.oldWidth));
      expect(
        () => doc.replaceText(page, page.runs[0], 'otra vez'),
        throwsA(isA<PdfException>().having((e) => e.isStaleRun, 'stale', isTrue)),
      );
      final again = doc.page(0);
      expect(again.runs[0].text, 'Adiós');
      doc.replaceText(again, again.runs[0], 'otra vez');
    });

    test('adds text with options and refuses what no font can draw', () {
      final added = doc.addText(0, NewText('Nombre completo', 72, 300, size: 14, bold: true, color: [0, 0, 1]));
      expect(added.lines, 1);
      final runs = doc.page(0).runs;
      expect(runs.last.text, 'Nombre completo');
      expect(runs.last.baseFont, 'LiberationSans-Bold');
      expect(
        () => doc.addText(0, NewText('世界', 1, 1)),
        throwsA(isA<PdfException>()
            .having((e) => e.isMissingGlyphs, 'missing', isTrue)
            .having((e) => e.chars, 'chars', '世界')),
      );
    });

    test('undo and redo walk the history', () {
      expect(doc.history.canUndo, isFalse);
      final page = doc.page(0);
      doc.replaceText(page, page.runs[0], 'Cambiado');
      expect(doc.history.canUndo, isTrue);
      expect(doc.undo(), isTrue);
      expect(doc.page(0).runs[0].text, 'Hola mundo');
      expect(doc.undo(), isFalse);
      expect(doc.redo(), isTrue);
      expect(doc.page(0).runs[0].text, 'Cambiado');
    });

    test('saves bytes that open again with the edits', () {
      final page = doc.page(0);
      doc.replaceText(page, page.runs[0], 'Guardado');
      final bytes = doc.save();
      expect(String.fromCharCodes(bytes.sublist(0, 5)), '%PDF-');
      final again = PdfDocument.open(bytes, libraryPath: libraryPath);
      addTearDown(again.close);
      expect(again.page(0).runs[0].text, 'Guardado');
    });

    test('a closed document says so instead of crashing', () {
      final other = open('standard')..close();
      expect(() => other.page(0), throwsStateError);
      other.close(); // closing twice is harmless
    });
  });

  test('a subset font lacking a letter: refused, then drawn with the fallback', () {
    final doc = open('embedded');
    addTearDown(doc.close);
    var page = doc.page(0);
    expect(
      () => doc.replaceText(page, page.runs[0], 'Póliza XQ'),
      throwsA(isA<PdfException>().having((e) => e.chars, 'chars', 'XQ')),
    );
    final done = doc.replaceText(page, page.runs[0], 'Póliza XQ', allowFallback: true);
    expect(done.fallbackChars, 'XQ');
    expect(done.reread, isTrue);
    page = doc.page(0);
    expect(page.runs.map((r) => r.text).join(), 'Póliza XQ');
  });

  test('form fields: read, fill, refuse, force', () {
    final doc = open('form');
    addTearDown(doc.close);
    final fields = {for (final f in doc.fields()) f.name: f};
    expect(fields['nombre']!.value, 'Ana');
    expect(fields['fijo']!.readOnly, isTrue);
    expect(fields['pais']!.kind, FieldKind.combo);
    expect(fields['pais']!.options.map((o) => o.export), ['España', 'Francia', 'Portugal']);
    expect(fields['respuesta']!.widgets.map((w) => w.onState), ['Si', 'No', 'NS']);

    doc.setField('nombre', 'María José');
    doc.setField('respuesta', 'No');
    doc.setField('acepto', 'true');
    expect(() => doc.setField('corto', 'demasiado largo'), throwsA(isA<PdfException>()));
    expect(() => doc.setField('fijo', 'x'), throwsA(isA<PdfException>().having((e) => e.kind, 'kind', 'rejected')));
    doc.setField('fijo', 'x', ignoreReadOnly: true);
    expect(() => doc.setField('nada', 'x'), throwsA(isA<PdfException>().having((e) => e.kind, 'kind', 'no_such_field')));

    final after = {for (final f in doc.fields()) f.name: f};
    expect(after['nombre']!.value, 'María José');
    expect(after['respuesta']!.value, 'No');
    expect(after['acepto']!.checked, isTrue);
    expect(after['fijo']!.value, 'x');
  });

  test('opening and closing many times does not fall over', () {
    for (var i = 0; i < 100; i++) {
      final d = open('standard');
      d.page(0);
      d.close();
    }
  });

  group('PdfSession (own isolate)', () {
    test('every call is a Future and results come back intact', () async {
      final session = await PdfSession.open(fixture('standard'), libraryPath: libraryPath);
      try {
        expect(await session.pageCount(), 1);
        final page = await session.page(0);
        expect(page.runs.first.text, 'Hola mundo');
        final done = await session.replaceText(page, page.runs[0], 'Desde otro isolate');
        expect(done.fallbackChars, '');
        expect((await session.page(0)).runs[0].text, 'Desde otro isolate');

        await session.addText(0, NewText('añadido', 72, 300));
        expect((await session.history()).canUndo, isTrue);
        expect(await session.undo(), isTrue);

        final bytes = await session.save();
        expect(String.fromCharCodes(bytes.sublist(0, 5)), '%PDF-');
      } finally {
        await session.close();
      }
    });

    test('errors cross the isolate boundary with their kind and characters', () async {
      final session = await PdfSession.open(fixture('embedded'), libraryPath: libraryPath);
      try {
        final page = await session.page(0);
        await expectLater(
          session.replaceText(page, page.runs[0], 'Póliza XQ'),
          throwsA(isA<PdfException>().having((e) => e.chars, 'chars', 'XQ')),
        );
        await expectLater(session.setField('nada', 'x'), throwsA(isA<PdfException>()));
        // The session survives an error.
        expect(await session.pageCount(), 1);
      } finally {
        await session.close();
      }
    });

    test('a bad file fails to open', () async {
      await expectLater(
        PdfSession.open(Uint8List.fromList([1, 2, 3]), libraryPath: libraryPath),
        throwsA(isA<PdfException>()),
      );
    });

    test('calls made together are answered in order', () async {
      final session = await PdfSession.open(fixture('form'), libraryPath: libraryPath);
      try {
        final results = await Future.wait([
          session.pageCount(),
          session.fields(),
          session.page(0),
        ]);
        expect(results[0], 1);
        expect((results[1] as List<FormField>).length, 8);
        expect((results[2] as PageData).runs, isNotEmpty);
      } finally {
        await session.close();
      }
    });

    test('a closed session refuses further calls', () async {
      final session = await PdfSession.open(fixture('standard'), libraryPath: libraryPath);
      await session.close();
      await expectLater(session.pageCount(), throwsStateError);
    });
  });

  group('reflow, warnings and signatures', () {
    TextRun named(PageData page, String text) => page.runs.firstWhere((r) => r.text.trim() == text);

    test('a longer run moves the rest of its line and says how many', () {
      final doc = open('line_absolute');
      addTearDown(doc.close);
      var page = doc.page(0);
      final after = named(page, 'de 9 a 14 horas').origin[0];
      final done = doc.replaceText(page, named(page, 'lunes a viernes'), 'de lunes a viernes inclusive');
      expect(done.moved, 1);
      expect(done.warnings, isEmpty);
      expect(done.reread, isTrue);
      page = doc.page(0);
      expect(named(page, 'de 9 a 14 horas').origin[0] - after, closeTo(done.newWidth - done.oldWidth, 0.05));
      expect(named(page, 'Segunda linea del documento').origin, [72.0, 680.0]);
    });

    test('Reflow.none leaves everything where it was', () {
      final doc = open('line_absolute');
      addTearDown(doc.close);
      var page = doc.page(0);
      final before = named(page, 'de 9 a 14 horas').origin[0];
      final done = doc.replaceText(page, named(page, 'lunes a viernes'), 'de lunes a viernes inclusive',
          reflow: Reflow.none);
      expect(done.moved, 0);
      expect(named(doc.page(0), 'de 9 a 14 horas').origin[0], before);
    });

    test('warnings cross the boundary with their details', () {
      var doc = open('line_absolute');
      addTearDown(doc.close);
      var page = doc.page(0);
      var done = doc.replaceText(page, named(page, 'lunes a viernes'), 'lunes a viernes y tambien ' * 6);
      expect(done.warnings.map((w) => w.kind), contains('exceeds_page'));
      expect(done.warnings.firstWhere((w) => w.kind == 'exceeds_page').by, greaterThan(0));

      doc = open('line_xobject');
      addTearDown(doc.close);
      page = doc.page(0);
      done = doc.replaceText(page, named(page, 'El resto de las oficinas de'), 'El resto de todas las oficinas de pasaportes de');
      expect(done.warnings.single.kind, 'follower_not_moved');
      expect(done.warnings.single.text, 'lunes a viernes');
    });

    test('signatures, and which widgets carry an appearance', () {
      final signed = open('signed');
      addTearDown(signed.close);
      expect(signed.signatures().single.field, 'firma');
      expect(signed.signatures().single.signer, 'Cesar Ruiz');
      expect(signed.fields().firstWhere((f) => f.name == 'firma').signed, isTrue);

      final bare = open('form_no_appearance');
      addTearDown(bare.close);
      expect(bare.fields().firstWhere((f) => f.name == 'nombre').widgets.first.hasAppearance, isFalse);
      bare.setField('nombre', 'Con apariencia');
      expect(bare.fields().firstWhere((f) => f.name == 'nombre').widgets.first.hasAppearance, isTrue);
      expect(open('form')..close(), isNotNull);
    });

    test('the session passes the reflow mode and returns signatures', () async {
      final session = await PdfSession.open(fixture('signed'), libraryPath: libraryPath);
      try {
        expect((await session.signatures()).single.signer, 'Cesar Ruiz');
      } finally {
        await session.close();
      }
      final lines = await PdfSession.open(fixture('line_absolute'), libraryPath: libraryPath);
      try {
        final page = await lines.page(0);
        final done = await lines.replaceText(page, named(page, 'lunes a viernes'), 'de lunes a viernes inclusive',
            reflow: Reflow.centered);
        expect(done.moved, greaterThan(0));
      } finally {
        await lines.close();
      }
    });
  });

  group('protected documents', () {
    test('ask for a password only when one is needed', () async {
      final free = open('encrypted_owner');
      addTearDown(free.close);
      expect(free.page(0).runs.first.text, 'Hola mundo');

      expect(() => open('encrypted_user'), throwsA(isA<PdfException>().having((e) => e.kind, 'kind', 'password_required')));
      expect(() => PdfDocument.open(fixture('encrypted_user'), password: 'mal', libraryPath: libraryPath),
          throwsA(isA<PdfException>().having((e) => e.kind, 'kind', 'wrong_password')));
      final doc = PdfDocument.open(fixture('encrypted_user'), password: 'secreto', libraryPath: libraryPath);
      addTearDown(doc.close);
      expect(doc.pageCount, 1);

      await expectLater(PdfSession.open(fixture('encrypted_user'), libraryPath: libraryPath),
          throwsA(isA<PdfException>().having((e) => e.kind, 'kind', 'password_required')));
      final session = await PdfSession.open(fixture('encrypted_user'), password: 'secreto', libraryPath: libraryPath);
      await session.close();
    });
  });

  group('harder forms', () {
    test('list boxes take several rows, and fields carry their filling order', () {
      final doc = PdfDocument.open(fixture('form_hard'), libraryPath: libraryPath);
      addTearDown(doc.close);
      final fields = doc.fields();
      final ramos = fields.firstWhere((f) => f.name == 'ramos');
      expect(ramos.multiSelect, isTrue);
      doc.setField('ramos', 'Vida\nAuto');
      expect(doc.fields().firstWhere((f) => f.name == 'ramos').value, 'Vida\nAuto');
      final ordered = [...fields]..sort((a, b) => a.order.compareTo(b.order));
      expect(ordered.take(3).map((f) => f.name), ['rfc', 'notas', 'importe']);
    });
  });

  group('lines, search and replace-all', () {
    test('lines hold the words of a line, search finds them, replace-all is one undo', () async {
      final doc = PdfDocument.open(fixture('multi'), libraryPath: libraryPath);
      addTearDown(doc.close);
      final lines = doc.lines(0);
      expect(lines.first.text, 'Contrato 1 de la aseguradora Prudential');
      expect(lines.first.words.map((w) => w.text), ['Contrato', '1', 'de', 'la', 'aseguradora', 'Prudential']);
      final cliente = lines.firstWhere((l) => l.text.startsWith('Cliente'));
      expect(cliente.runIds, hasLength(2));

      expect(doc.find('perez nunez'), hasLength(3));
      final hit = doc.find('nunez').first;
      expect(hit.withinRun, isNotNull);
      expect(doc.find('prudential', options: const SearchOptions(caseSensitive: true)), hasLength(3));

      final done = doc.replaceAll('Prudential', 'Metlife', options: const SearchOptions(caseSensitive: true));
      expect([done.replaced, done.skipped], [9, 0]);
      expect(doc.lines(2).first.text, 'Contrato 3 de la aseguradora Metlife');
      expect(doc.undo(), isTrue);
      expect(doc.lines(2).first.text, 'Contrato 3 de la aseguradora Prudential');
      expect(doc.history.canUndo, isFalse);

      final session = await PdfSession.open(fixture('multi'), libraryPath: libraryPath);
      try {
        expect((await session.find('poliza', options: const SearchOptions(wholeWord: true))), hasLength(3));
        expect((await session.replaceAll('Zurich', 'x')).replaced, 0);
        expect((await session.lines(0)).first.words, isNotEmpty);
      } finally {
        await session.close();
      }
    });
  });

  group('pages', () {
    test('rotate, move, delete, bring in and extract', () async {
      final doc = PdfDocument.open(fixture('multi'), libraryPath: libraryPath);
      addTearDown(doc.close);
      doc.rotatePage(0, 90);
      expect(doc.page(0).rotation, 90);
      doc.movePage(0, 2);
      expect(doc.lines(2).any((l) => l.text.startsWith('Contrato 1')), isTrue);
      doc.deletePage(0);
      expect(doc.pageCount, 2);
      expect(doc.insertPages(fixture('form'), at: 0), 1);
      expect(doc.pageCount, 3);
      expect(doc.fields().any((f) => f.name == 'nombre'), isTrue);
      final part = PdfDocument.open(doc.extractPages([2, 1]), libraryPath: libraryPath);
      addTearDown(part.close);
      expect(part.pageCount, 2);
      expect(() => doc.extractPages([99]), throwsA(isA<PdfException>().having((e) => e.kind, 'kind', 'no_such_page')));
      expect(() => doc.deletePage(99), throwsA(isA<PdfException>()));

      final session = await PdfSession.open(fixture('multi'), libraryPath: libraryPath);
      try {
        await session.deletePage(1);
        expect(await session.pageCount(), 2);
        expect(await session.insertPages(fixture('standard'), pages: [0]), 1);
        expect(await session.pageCount(), 3);
        final out = await session.extractPages([0]);
        expect(out, isNotEmpty);
      } finally {
        await session.close();
      }
    });
  });

  group('pictures', () {
    test('a JPEG and a PNG are placed, and bad data is refused', () async {
      final doc = open('standard');
      addTearDown(doc.close);
      final jpeg = File('${engine.path}/pdfcore/tests/fixtures/img_rgb.jpg').readAsBytesSync();
      final added = doc.addImage(0, jpeg, x: 100, y: 200, width: 160);
      expect(added.display, [100, 200, 260, 280]);
      expect([added.pixelWidth, added.pixelHeight], [80, 40]);
      final png = File('${engine.path}/pdfcore/tests/fixtures/img_alpha.png').readAsBytesSync();
      expect(doc.addImage(0, png, x: 10, y: 10, width: 30, height: 30).display, [10, 10, 40, 40]);
      expect(() => doc.addImage(0, Uint8List.fromList([1, 2, 3]), x: 0, y: 0, width: 10),
          throwsA(isA<PdfException>().having((e) => e.kind, 'kind', 'unsupported')));
      expect(doc.history.canUndo, isTrue);
      expect(doc.undo() && doc.undo() && !doc.history.canUndo, isTrue);

      final session = await PdfSession.open(fixture('standard'), libraryPath: libraryPath);
      try {
        expect((await session.addImage(0, jpeg, x: 0, y: 0, width: 80)).pixelHeight, 40);
      } finally {
        await session.close();
      }
    });
  });

  group('font book', () {
    test('required fonts, a registered font and the letters it draws', () {
      final doc = open('font_subset_as_verdana');
      addTearDown(doc.close);
      final fonts = doc.requiredFonts();
      expect(fonts.single.needed, isTrue);
      expect(fonts.single.source!.name, 'Open Sans (bundled)');
      expect(doc.page(0).runs.first.substituteSimilarity, lessThan(90));
      final font = File('${engine.path}/pdfcore/tests/fixtures/registered_verdana.ttf').readAsBytesSync();
      expect(doc.registerFont(font).family, 'Verdana');
      expect(doc.requiredFonts().single.registered, isTrue);
      doc.setSubstitution(Substitution.wholeRun);
      final page = doc.page(0);
      final done = doc.replaceText(page, page.runs.first, 'Hola Ñandú', allowFallback: true);
      expect(done.substitutes.every((s) => s.similarity == 100 && s.isBundledFont), isTrue);
    });
  });

  group('graphics', () {
    test('rectangles, lines and paths come back as the page is shown, also from the isolate session', () async {
      final doc = open('graphics');
      addTearDown(doc.close);
      final g = doc.graphics(0);
      expect(g, hasLength(7));
      expect(g[0].isRect && g[0].filled && g[0].stroked, isTrue);
      expect(g[0].display, [100, 112, 150, 142]);
      expect(g[0].fill, [1, 0, 0]);
      expect(g[1].isLine, isTrue);
      expect(g[1].ends, [[200, 142], [300, 92]]);
      expect(g[5].xobject, 'Fm1');
      final turned = open('graphics_rotated');
      addTearDown(turned.close);
      expect(turned.graphics(0)[0].display, [700, 100, 730, 150]);
      final session = await PdfSession.open(fixture('graphics'), libraryPath: libraryPath);
      try {
        expect((await session.graphics(0)).map((x) => x.kind).take(3), ['rect', 'line', 'rect']);
      } finally {
        await session.close();
      }
    });
    test('the three check boxes of the real form', () {
      final path = Platform.environment['CAMBIO_PDF'];
      if (path == null || !File(path).existsSync()) return;
      final doc = PdfDocument.open(File(path).readAsBytesSync(), libraryPath: libraryPath);
      addTearDown(doc.close);
      final boxes = doc
          .graphics(0)
          .where((g) => g.isRect && (g.width - 9.4).abs() < 0.3 && (g.height - 9.4).abs() < 0.3 && (g.display[1] - 266.2).abs() < 0.5)
          .toList();
      expect(boxes, hasLength(3));
      for (final (i, x) in [29.6, 87.9, 174.5].indexed) {
        expect(boxes[i].display[0], closeTo(x, 0.2));
      }
    });
  });

  group('benchmark', () {
    test('measures every step on a real document', () {
      final result = benchmarkDocument(fixture('producer_reportlab_ttf'), libraryPath: libraryPath, repeat: 2);
      final steps = result['best_ms'] as Map<String, double>;
      expect(steps.keys, containsAll(['open', 'page', 'blocks', 'replace_block', 'save', 'save_incremental']));
      expect(steps.values.every((ms) => ms >= 0), isTrue);
      expect(result['pages'], 1);
    });
  });

  group('annotations, flatten, redact, OCR report, incremental save', () {
    test('they cross the boundary', () {
      final doc = open('standard');
      addTearDown(doc.close);
      final box = doc.page(0).runs.first.display;
      doc.addAnnotation(0, NewAnnotation.highlight([box]));
      doc.addAnnotation(0, NewAnnotation.note(300, 100, 'Revisar'));
      final listed = doc.annotations(0);
      expect(listed.map((a) => a.kind), ['highlight', 'note']);
      doc.setAnnotationContents(0, listed[1], 'Otra nota');
      doc.deleteAnnotation(0, listed[0]);
      expect(doc.annotations(0).single.contents, 'Otra nota');
      expect(doc.flatten(annotations: true, forms: false).drawn, 1);
      expect(doc.annotations(0), isEmpty);
      final done = doc.redact(0, [box]);
      expect(done.chars, greaterThan(0));
      expect(doc.ocrReport().single.kind, 'native');
      expect(doc.saveIncremental().length, greaterThan(fixture('standard').length));
    });
  });

  group('managing items', () {
    test('hit test, resize, delete and a batch that is one undo step', () {
      final doc = open('canvas');
      addTearDown(doc.close);
      final items = doc.items(0);
      final text = items.firstWhere((i) => i.kind == PdfItemKind.text && i.text.startsWith('Primera'));
      final hit = doc.hitTest(0, (text.display[0] + text.display[2]) / 2, (text.display[1] + text.display[3]) / 2)!;
      expect(hit.kind, PdfItemKind.text);
      expect(hit.caret, isNotNull);
      expect(doc.hitTest(0, 590, 835), isNull);
      final picture = items.firstWhere((i) => i.kind == PdfItemKind.image);
      final field = items.firstWhere((i) => i.kind == PdfItemKind.field);
      final results = doc.batch([
        BatchOp.resizeItem(0, picture, [picture.display[0], picture.display[1], picture.display[0] + 30, picture.display[1] + 30]),
        BatchOp.setField(field.fieldName!, 'Lote'),
      ]);
      expect(results, hasLength(2));
      doc.undo();
      expect(doc.fields().firstWhere((f) => f.name == field.fieldName).value, isNot('Lote'));
      final fresh = doc.items(0);
      doc.deleteItems(0, fresh.where((i) => i.kind == PdfItemKind.image).toList());
      expect(doc.items(0).where((i) => i.kind == PdfItemKind.image), isEmpty);
    });
  });

  group('block styles', () {
    test('a range is made bold and red, replaced, and what is below is pushed down', () {
      final doc = open('producer_reportlab_ttf');
      addTearDown(doc.close);
      var block = doc.blocks(0).firstWhere((b) => b.text.startsWith('Para consultar'));
      doc.setBlockStyle(0, block, 0, 4, bold: true, color: [1, 0, 0]);
      block = doc.blocks(0).firstWhere((b) => b.text.startsWith('Para consultar'));
      expect(block.spans.first.bold, isTrue);
      expect(block.spans.first.color, [1, 0, 0]);
      expect(() => doc.setBlockStyle(0, block, 0, 4, italic: true),
          throwsA(isA<PdfException>().having((e) => e.kind, 'kind', 'rejected')));
      expect(doc.blockChars(0, block), hasLength(block.text.length));
      doc.replaceRange(0, block, 0, 4, 'Para');
      block = doc.blocks(0).firstWhere((b) => b.text.startsWith('Para consultar'));
      final other = doc.blocks(0).firstWhere((b) => b.text.startsWith('Otro'));
      doc.replaceBlock(0, block, '${block.text} ${'Texto agregado para ocupar mas lineas. ' * 8}', pushDown: true);
      final moved = doc.blocks(0).firstWhere((b) => b.text.startsWith('Otro'));
      expect(moved.display[1], greaterThan(other.display[1] + 10));
      doc.replaceBlock(0, doc.blocks(0).first, doc.blocks(0).first.text, justify: false);
    });
  });

  group('blocks', () {
    test('a paragraph with bold words is one block and keeps its styles when edited', () async {
      final doc = open('producer_reportlab_ttf');
      addTearDown(doc.close);
      final block = doc.blocks(0).firstWhere((b) => b.text.startsWith('Para consultar'));
      expect(block.lines, hasLength(4));
      expect(block.justified, isTrue);
      final bold = [for (final s in block.spans.where((s) => s.bold)) block.text.substring(s.start, s.end)];
      expect(bold.last, '55 893 24827');
      final done = doc.replaceBlock(0, block, block.text.replaceFirst('24827', '24828'));
      expect(done.rewritten, greaterThanOrEqualTo(1));
      expect(() => doc.replaceBlock(0, block, 'x'), throwsA(isA<PdfException>().having((e) => e.isStaleBlock, 'stale', true)));
      final after = doc.blocks(0).firstWhere((b) => b.text.startsWith('Para consultar'));
      expect(after.spans.where((s) => s.bold).map((s) => after.text.substring(s.start, s.end)).last, '55 893 24828');
    });
  });

  group('paragraphs', () {
    test('a paragraph is found, re-laid out longer and shorter, and undone', () async {
      final doc = open('paragraph');
      addTearDown(doc.close);
      var page = doc.page(0);
      final line = page.runs.firstWhere((r) => r.text.startsWith('El asegurado'));
      final paragraph = doc.paragraphOf(page, line)!;
      expect(paragraph.runIds, hasLength(5));
      expect(paragraph.text, startsWith('El asegurado debe notificar'));
      expect(doc.paragraphOf(page, page.runs.firstWhere((r) => r.text.startsWith('Condiciones'))), isNull);

      final longer = doc.replaceParagraph(page, line, '${paragraph.text} Ademas, debera entregar copia de cualquier documento que respalde el cambio.');
      expect(longer.added, greaterThan(0));
      expect(longer.warnings, isEmpty);
      page = doc.page(0);
      final again = page.runs.firstWhere((r) => r.text.startsWith('El asegurado'));
      expect(doc.paragraphOf(page, again)!.runIds, hasLength(5 + longer.added));
      expect(doc.undo(), isTrue);

      page = doc.page(0);
      final shorter = doc.replaceParagraph(page, page.runs.firstWhere((r) => r.text.startsWith('El asegurado')), 'El asegurado avisa.');
      expect([shorter.lines, shorter.removed], [1, 4]);
      expect(() => doc.replaceParagraph(doc.page(0), doc.page(0).runs.firstWhere((r) => r.text.startsWith('Condiciones')), 'x'),
          throwsA(isA<PdfException>().having((e) => e.kind, 'kind', 'unsupported')));

      final session = await PdfSession.open(fixture('paragraph'), libraryPath: libraryPath);
      try {
        final p = await session.page(0);
        final r = p.runs.firstWhere((r) => r.text.startsWith('El asegurado'));
        expect((await session.paragraphOf(p, r))!.runIds, hasLength(5));
        expect((await session.replaceParagraph(p, r, 'Corto.')).removed, 4);
      } finally {
        await session.close();
      }
    });
  });

  group('field layout', () {
    test('a field moves, several line up, and one undo takes each back', () async {
      final doc = PdfDocument.open(fixture('form_hard'), libraryPath: libraryPath);
      addTearDown(doc.close);
      final box = doc.moveField('rfc', [300, 100, 450, 124]);
      expect(box, [300, 100, 450, 124]);
      expect(doc.fields().firstWhere((f) => f.name == 'rfc').widgets.first.display, [300, 100, 450, 124]);
      doc.arrangeFields([const FieldRefs('rfc'), const FieldRefs('importe'), const FieldRefs('estado')], Arrange.alignLeft);
      final lefts = doc.fields().where((f) => ['rfc', 'importe', 'estado'].contains(f.name)).map((f) => f.widgets.first.display[0]).toSet();
      expect(lefts, hasLength(1));
      expect(doc.undo(), isTrue);
      expect(() => doc.arrangeFields([const FieldRefs('rfc')], Arrange.alignLeft), throwsA(isA<PdfException>()));
      expect(() => doc.moveField('nada', [0, 0, 10, 10]), throwsA(isA<PdfException>().having((e) => e.kind, 'kind', 'no_such_field')));

      final session = await PdfSession.open(fixture('form_hard'), libraryPath: libraryPath);
      try {
        expect((await session.moveField('rfc', [10, 10, 100, 30])).first, 10);
        await session.arrangeFields([const FieldRefs('rfc'), const FieldRefs('importe')], Arrange.sameWidth);
      } finally {
        await session.close();
      }
    });
  });

  group('items, like a design tool', () {
    test('texts, pictures and fields line up and space evenly; paragraphs and fields align their text', () async {
      final doc = PdfDocument.open(fixture('canvas'), libraryPath: libraryPath);
      addTearDown(doc.close);
      var items = doc.items(0);
      expect(items.where((i) => i.kind == PdfItemKind.text), hasLength(4));
      expect(items.where((i) => i.kind == PdfItemKind.image), hasLength(2));
      expect(items.where((i) => i.kind == PdfItemKind.field).single.fieldName, 'dato');
      final block = items.firstWhere((i) => i.text.startsWith('Primera'));
      expect(block.lines, 3);

      PdfItem named(String t) => doc.items(0).firstWhere((i) => i.text.startsWith(t));
      doc.arrangeItems(0, [named('Uno'), named('Dos mas'), named('Tres')], Arrange.alignLeft);
      final left = doc.items(0).where((i) => ['Uno', 'Dos mas', 'Tres'].any(i.text.startsWith)).map((i) => i.display[0]);
      expect(left.map((v) => v.round()).toSet(), hasLength(1));
      expect(doc.undo(), isTrue);

      doc.moveItems(0, [ItemMove(named('Dos mas'), 20, 5)]);
      expect(doc.items(0).firstWhere((i) => i.text.startsWith('Dos mas')).display[0], closeTo(130 + 20, 1));
      expect(doc.undo(), isTrue);

      doc.spaceLines(0, [named('Uno'), named('Dos mas'), named('Tres')], leading: 24);
      expect(() => doc.spaceLines(0, [named('Uno')]), throwsA(isA<PdfException>()));

      var page = doc.page(0);
      doc.alignParagraph(page, page.runs.firstWhere((r) => r.text.startsWith('Primera')), TextAlign.center);
      doc.setFieldsAlignment(['dato'], TextAlign.right);
      expect(doc.fields().firstWhere((f) => f.name == 'dato'), isNotNull);

      final session = await PdfSession.open(fixture('canvas'), libraryPath: libraryPath);
      try {
        final list = await session.items(0);
        await session.arrangeItems(0, list.where((i) => i.kind == PdfItemKind.text).take(3).toList(), Arrange.alignRight);
        expect((await session.items(0)).isNotEmpty, isTrue);
      } finally {
        await session.close();
      }
    });
  });

  group('font fidelity', () {
    test('runs say how reproducible their font is, and what cannot be edited says why', () async {
      final macroman = open('font_macroman');
      addTearDown(macroman.close);
      expect(macroman.page(0).runs.first.text, 'café ñandú');
      expect(macroman.page(0).runs.first.fontKind, 'standard');
      expect(macroman.page(0).runs.first.editable, isTrue);

      final subset = open('embedded');
      addTearDown(subset.close);
      expect([subset.page(0).runs.first.fontKind, subset.page(0).runs.first.fontEmbedded], ['embedded_subset', true]);

      for (final entry in {'font_type3': 'type3', 'font_type0_identity_v': 'vertical_writing', 'font_zapf': 'symbolic_font', 'font_rtl_arabic': 'rtl_text'}.entries) {
        final doc = open(entry.key);
        addTearDown(doc.close);
        final page = doc.page(0);
        expect(page.runs.first.editable, isFalse, reason: entry.key);
        expect(page.runs.first.notEditable, entry.value, reason: entry.key);
        expect(
            () => doc.replaceText(page, page.runs.first, 'Hola'),
            throwsA(isA<PdfException>()
                .having((e) => e.kind, 'kind', 'not_editable')
                .having((e) => e.reason, 'reason', entry.value)
                .having((e) => e.isNotEditable, 'isNotEditable', isTrue)),
            reason: entry.key);
        expect(doc.history.canUndo, isFalse);
      }

      final session = await PdfSession.open(fixture('font_type3'), libraryPath: libraryPath);
      try {
        final page = await session.page(0);
        await expectLater(session.replaceText(page, page.runs.first, 'Hola'),
            throwsA(isA<PdfException>().having((e) => e.reason, 'reason across the isolate', 'type3')));
      } finally {
        await session.close();
      }
    });
  });
}
