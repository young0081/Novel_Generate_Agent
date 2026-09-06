// ai_client.dart — 直连 OpenAI 兼容 / Anthropic API，支持流式输出
// 不依赖任何后端服务，所有 AI 调用从移动端直接发出。

import 'dart:async';
import 'dart:convert';
import 'package:http/http.dart' as http;

// ── 模型供应商配置 ─────────────────────────────────────────────────
enum AiProtocol { openAi, anthropic }

class AiProvider {
  final String name;
  final AiProtocol protocol;
  final String baseUrl;
  final String apiKey;
  final String model;

  const AiProvider({
    required this.name,
    required this.protocol,
    required this.baseUrl,
    required this.apiKey,
    required this.model,
  });

  bool get isConfigured => apiKey.isNotEmpty && model.isNotEmpty;

  Map<String, dynamic> toJson() => {
    'name': name,
    'protocol': protocol.name,
    'baseUrl': baseUrl,
    'apiKey': apiKey,
    'model': model,
  };

  factory AiProvider.fromJson(Map<String, dynamic> j) => AiProvider(
    name: j['name'] as String? ?? '',
    protocol: j['protocol'] == 'anthropic'
        ? AiProtocol.anthropic
        : AiProtocol.openAi,
    baseUrl: j['baseUrl'] as String? ?? '',
    apiKey: j['apiKey'] as String? ?? '',
    model: j['model'] as String? ?? '',
  );
}

// ── 内置预设 ────────────────────────────────────────────────────────
const List<AiProvider> kProviderPresets = [
  AiProvider(
    name: 'DeepSeek',
    protocol: AiProtocol.openAi,
    baseUrl: 'https://api.deepseek.com/v1',
    apiKey: '',
    model: 'deepseek-chat',
  ),
  AiProvider(
    name: 'OpenAI',
    protocol: AiProtocol.openAi,
    baseUrl: 'https://api.openai.com/v1',
    apiKey: '',
    model: 'gpt-4o-mini',
  ),
  AiProvider(
    name: 'Kimi',
    protocol: AiProtocol.openAi,
    baseUrl: 'https://api.moonshot.cn/v1',
    apiKey: '',
    model: 'moonshot-v1-8k',
  ),
  AiProvider(
    name: '智谱 GLM',
    protocol: AiProtocol.openAi,
    baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    apiKey: '',
    model: 'glm-4-flash',
  ),
  AiProvider(
    name: 'Anthropic',
    protocol: AiProtocol.anthropic,
    baseUrl: 'https://api.anthropic.com',
    apiKey: '',
    model: 'claude-3-5-haiku-latest',
  ),
  AiProvider(
    name: 'Ollama（本地）',
    protocol: AiProtocol.openAi,
    baseUrl: 'http://localhost:11434/v1',
    apiKey: 'ollama',
    model: 'llama3.1',
  ),
];

// ── AI 消息 ─────────────────────────────────────────────────────────
class AiMessage {
  final String role; // 'system' | 'user' | 'assistant'
  final String content;
  const AiMessage({required this.role, required this.content});
  Map<String, dynamic> toJson() => {'role': role, 'content': content};
}

class AiStreamDelta {
  final String content;
  final String reasoning;

  const AiStreamDelta({this.content = '', this.reasoning = ''});
}

class AiRequestCancelledException implements Exception {
  const AiRequestCancelledException();

  @override
  String toString() => '请求已取消';
}

String _apiErrorMessage(Object? error) {
  if (error is Map) {
    final message = error['message'];
    if (message is String && message.isNotEmpty) return message;
  }
  return error?.toString() ?? '未知 API 错误';
}

/// Decode complete SSE `data` fields while preserving lines split across
/// arbitrary HTTP chunks (including chunks split inside a UTF-8 character).
Stream<String> decodeSseData(Stream<List<int>> byteStream) async* {
  final dataLines = <String>[];

  await for (final line
      in byteStream.transform(utf8.decoder).transform(const LineSplitter())) {
    if (line.isEmpty) {
      if (dataLines.isNotEmpty) {
        yield dataLines.join('\n');
        dataLines.clear();
      }
      continue;
    }
    if (line.startsWith(':')) continue;

    final colon = line.indexOf(':');
    final field = colon < 0 ? line : line.substring(0, colon);
    if (field != 'data') continue;

    var value = colon < 0 ? '' : line.substring(colon + 1);
    if (value.startsWith(' ')) value = value.substring(1);
    dataLines.add(value);
  }

  if (dataLines.isNotEmpty) yield dataLines.join('\n');
}

Stream<AiStreamDelta> decodeOpenAiStream(Stream<List<int>> byteStream) async* {
  await for (final data in decodeSseData(byteStream)) {
    if (data.trim() == '[DONE]') return;

    Object? decoded;
    try {
      decoded = jsonDecode(data);
    } on FormatException {
      continue;
    }
    if (decoded is! Map<String, dynamic>) continue;
    if (decoded['error'] != null) {
      throw Exception('API 错误: ${_apiErrorMessage(decoded['error'])}');
    }

    final choices = decoded['choices'];
    if (choices is! List || choices.isEmpty) continue;
    final choice = choices.first;
    if (choice is! Map) continue;
    final delta = choice['delta'];
    if (delta is! Map) continue;

    final reasoning = delta['reasoning_content'];
    final content = delta['content'];
    final event = AiStreamDelta(
      reasoning: reasoning is String ? reasoning : '',
      content: content is String ? content : '',
    );
    if (event.reasoning.isNotEmpty || event.content.isNotEmpty) yield event;
  }
}

Stream<AiStreamDelta> decodeAnthropicStream(
  Stream<List<int>> byteStream,
) async* {
  await for (final data in decodeSseData(byteStream)) {
    Object? decoded;
    try {
      decoded = jsonDecode(data);
    } on FormatException {
      continue;
    }
    if (decoded is! Map<String, dynamic>) continue;
    if (decoded['type'] == 'error') {
      throw Exception('Anthropic 错误: ${_apiErrorMessage(decoded['error'])}');
    }
    if (decoded['type'] != 'content_block_delta') continue;

    final delta = decoded['delta'];
    if (delta is! Map) continue;
    final text = delta['text'];
    if (text is String && text.isNotEmpty) {
      yield AiStreamDelta(content: text);
    }
  }
}

// ── AI 客户端 ────────────────────────────────────────────────────────
class AiClient {
  final AiProvider provider;
  final Duration requestTimeout;
  final http.Client _httpClient;
  bool _cancelled = false;

  AiClient(
    this.provider, {
    http.Client? httpClient,
    this.requestTimeout = const Duration(seconds: 120),
  }) : _httpClient = httpClient ?? http.Client();

  void cancel() {
    if (_cancelled) return;
    _cancelled = true;
    _httpClient.close();
  }

  void close() => _httpClient.close();

  Uri _uri(String path) {
    final base = provider.baseUrl.replaceFirst(RegExp(r'/+$'), '');
    return Uri.parse('$base$path');
  }

  // 流式聊天：每收到一个 content token 调用 onToken；推理模型的思维链通过 onReasoning 传出
  Future<String> chatStream(
    List<AiMessage> messages, {
    required void Function(String token) onToken,
    void Function(String reasoning)? onReasoning,
    int maxTokens = 2048,
  }) async {
    try {
      return await switch (provider.protocol) {
        AiProtocol.openAi => _openAiStream(
          messages,
          onToken,
          maxTokens,
          onReasoning,
        ),
        AiProtocol.anthropic => _anthropicStream(messages, onToken, maxTokens),
      };
    } catch (_) {
      if (_cancelled) throw const AiRequestCancelledException();
      rethrow;
    }
  }

  // 非流式聊天（快速调用，用于意图检测等场景）
  Future<String> chat(List<AiMessage> messages, {int maxTokens = 512}) async {
    try {
      return await switch (provider.protocol) {
        AiProtocol.openAi => _openAiChat(messages, maxTokens),
        AiProtocol.anthropic => _anthropicChat(messages, maxTokens),
      };
    } catch (_) {
      if (_cancelled) throw const AiRequestCancelledException();
      rethrow;
    }
  }

  // ── OpenAI 兼容流式 ────────────────────────────────────────────────
  Future<String> _openAiStream(
    List<AiMessage> messages,
    void Function(String) onToken,
    int maxTokens,
    void Function(String)? onReasoning,
  ) async {
    final uri = _uri('/chat/completions');
    final req = http.Request('POST', uri)
      ..headers.addAll({
        'Content-Type': 'application/json',
        'Authorization': 'Bearer ${provider.apiKey}',
      })
      ..body = jsonEncode({
        'model': provider.model,
        'messages': messages.map((m) => m.toJson()).toList(),
        'stream': true,
        'max_tokens': maxTokens,
      });

    final response = await _httpClient.send(req).timeout(requestTimeout);
    if (response.statusCode != 200) {
      final body = await response.stream.bytesToString().timeout(
        requestTimeout,
      );
      throw Exception('API 错误 ${response.statusCode}: $body');
    }

    final buf = StringBuffer();
    await for (final event in decodeOpenAiStream(
      response.stream,
    ).timeout(requestTimeout)) {
      if (event.reasoning.isNotEmpty) onReasoning?.call(event.reasoning);
      if (event.content.isNotEmpty) {
        buf.write(event.content);
        onToken(event.content);
      }
    }
    return buf.toString();
  }

  // ── OpenAI 兼容非流式 ──────────────────────────────────────────────
  Future<String> _openAiChat(List<AiMessage> messages, int maxTokens) async {
    final res = await _httpClient
        .post(
          _uri('/chat/completions'),
          headers: {
            'Content-Type': 'application/json',
            'Authorization': 'Bearer ${provider.apiKey}',
          },
          body: jsonEncode({
            'model': provider.model,
            'messages': messages.map((m) => m.toJson()).toList(),
            'max_tokens': maxTokens,
          }),
        )
        .timeout(requestTimeout);
    if (res.statusCode != 200) {
      throw Exception('API 错误 ${res.statusCode}: ${res.body}');
    }
    final json = jsonDecode(utf8.decode(res.bodyBytes)) as Map<String, dynamic>;
    return json['choices'][0]['message']['content'] as String? ?? '';
  }

  // ── Anthropic 流式 ─────────────────────────────────────────────────
  Future<String> _anthropicStream(
    List<AiMessage> messages,
    void Function(String) onToken,
    int maxTokens,
  ) async {
    final system = messages.where((m) => m.role == 'system').toList();
    final other = messages.where((m) => m.role != 'system').toList();
    final body = <String, dynamic>{
      'model': provider.model,
      'max_tokens': maxTokens,
      'messages': other.map((m) => m.toJson()).toList(),
      'stream': true,
    };
    if (system.isNotEmpty) body['system'] = system.first.content;

    final req = http.Request('POST', _uri('/v1/messages'))
      ..headers.addAll({
        'Content-Type': 'application/json',
        'x-api-key': provider.apiKey,
        'anthropic-version': '2023-06-01',
      })
      ..body = jsonEncode(body);

    final response = await _httpClient.send(req).timeout(requestTimeout);
    if (response.statusCode != 200) {
      final b = await response.stream.bytesToString().timeout(requestTimeout);
      throw Exception('Anthropic 错误 ${response.statusCode}: $b');
    }

    final buf = StringBuffer();
    await for (final event in decodeAnthropicStream(
      response.stream,
    ).timeout(requestTimeout)) {
      if (event.content.isNotEmpty) {
        buf.write(event.content);
        onToken(event.content);
      }
    }
    return buf.toString();
  }

  // ── Anthropic 非流式 ───────────────────────────────────────────────
  Future<String> _anthropicChat(List<AiMessage> messages, int maxTokens) async {
    final system = messages.where((m) => m.role == 'system').toList();
    final other = messages.where((m) => m.role != 'system').toList();
    final body = <String, dynamic>{
      'model': provider.model,
      'max_tokens': maxTokens,
      'messages': other.map((m) => m.toJson()).toList(),
    };
    if (system.isNotEmpty) body['system'] = system.first.content;

    final res = await _httpClient
        .post(
          _uri('/v1/messages'),
          headers: {
            'Content-Type': 'application/json',
            'x-api-key': provider.apiKey,
            'anthropic-version': '2023-06-01',
          },
          body: jsonEncode(body),
        )
        .timeout(requestTimeout);
    if (res.statusCode != 200) {
      throw Exception('Anthropic 错误 ${res.statusCode}: ${res.body}');
    }
    final json = jsonDecode(utf8.decode(res.bodyBytes)) as Map<String, dynamic>;
    return (json['content'] as List?)?.firstOrNull?['text'] as String? ?? '';
  }

  // ── 连接测试 ────────────────────────────────────────────────────────
  Future<String> testConnection() async {
    try {
      return await chat([
        const AiMessage(role: 'user', content: '请用一个字回答：好。'),
      ], maxTokens: 8);
    } catch (e) {
      throw Exception('$e');
    }
  }
}
