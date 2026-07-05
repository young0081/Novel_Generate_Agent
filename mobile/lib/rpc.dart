import 'dart:convert';

import 'package:http/http.dart' as http;

/// A JSON-RPC error returned by the Rust core (surfaced through /api/rpc).
class RpcException implements Exception {
  final int code;
  final String message;
  RpcException(this.code, this.message);

  @override
  String toString() => '[$code] $message';
}

/// Talks to the Rust core over HTTP via the Next.js `/api/rpc` bridge.
///
/// On Android the app connects to a backend running on the team's desktop /
/// server (set the address in Settings). This reuses the exact same JSON-RPC
/// surface the web and desktop clients use.
class RpcService {
  String baseUrl;

  RpcService(this.baseUrl);

  String _endpoint() {
    var u = baseUrl.trim();
    while (u.endsWith('/')) {
      u = u.substring(0, u.length - 1);
    }
    return '$u/api/rpc';
  }

  /// Send a method + params, returning the unwrapped `result` (throws on error).
  Future<dynamic> call(String method, [Map<String, dynamic>? params]) async {
    final res = await http.post(
      Uri.parse(_endpoint()),
      headers: const {'content-type': 'application/json'},
      body: jsonEncode({
        'method': method,
        'params': params ?? <String, dynamic>{},
      }),
    );
    final body = jsonDecode(utf8.decode(res.bodyBytes)) as Map<String, dynamic>;
    final err = body['error'];
    if (err is Map) {
      throw RpcException(
        (err['code'] as num?)?.toInt() ?? -1,
        err['message']?.toString() ?? 'unknown error',
      );
    }
    return body['result'];
  }

  Future<List<dynamic>> listTools() async =>
      (await call('list_tools')) as List<dynamic>;

  Future<Map<String, dynamic>> invokeTool(
    String name,
    Map<String, dynamic> args,
  ) async => (await call(name, args)) as Map<String, dynamic>;

  Future<String> ping() async => (await call('ping')).toString();
}
