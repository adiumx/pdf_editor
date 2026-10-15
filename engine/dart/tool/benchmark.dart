// dart run tool/benchmark.dart file.pdf [path/to/libpdfcore_ffi.so]
import 'dart:convert';
import 'dart:io';

import 'package:pdfcore/benchmark.dart';

void main(List<String> args) {
  if (args.isEmpty) {
    stderr.writeln('uso: dart run tool/benchmark.dart archivo.pdf [libpdfcore_ffi]');
    exit(64);
  }
  final result = benchmarkDocument(File(args[0]).readAsBytesSync(), libraryPath: args.length > 1 ? args[1] : null);
  stdout.writeln(const JsonEncoder.withIndent('  ').convert(result));
}
