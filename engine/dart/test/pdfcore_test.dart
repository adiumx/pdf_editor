// Run with the library built first:
//   cargo build -p pdfcore-ffi && cd dart && dart test
import 'dart:io';
import 'dart:typed_data';

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
}
