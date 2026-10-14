/// Dart bindings for the pdfcore engine.
///
/// [PdfDocument] is the synchronous API: simplest, fine for tests and small
/// jobs. In a Flutter app use [PdfSession], which runs the engine in its own
/// isolate so editing a big document never freezes the screen.
library;

export 'src/document.dart' show PdfDocument, loadPdfCoreLibrary;
export 'src/models.dart';
export 'src/session.dart' show PdfSession;
