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
    protocol: j['protocol'] == 'anthropic' ? AiProtocol.anthropic : AiProtocol.openAi,
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
    apiKey: '', model: 'deepseek-chat',
  ),
  AiProvider(
    name: 'OpenAI',
    protocol: AiProtocol.openAi,
    baseUrl: 'https://api.openai.com/v1',
    apiKey: '', model: 'gpt-4o-mini',
  ),
  AiProvider(
    name: 'Kimi',
    protocol: AiProtocol.openAi,
    baseUrl: 'https://api.moonshot.cn/v1',
    apiKey: '', model: 'moonshot-v1-8k',
  ),
  AiProvider(
    name: '智谱 GLM',
    protocol: AiProtocol.openAi,
    baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    apiKey: '', model: 'glm-4-flash',
  ),
  AiProvider(
    name: 'Anthropic',
    protocol: AiProtocol.anthropic,
    baseUrl: 'https://api.anthropic.com',
    apiKey: '', model: 'claude-3-5-haiku-latest',
  ),
  AiProvider(
    name: 'Ollama（本地）',
    protocol: AiProtocol.openAi,
    baseUrl: 'http://localhost:11434/v1',
    apiKey: 'ollama', model: 'llama3.1',
  ),
];

// ── AI 消息 ─────────────────────────────────────────────────────────
class AiMessage {
  final String role; // 'system' | 'user' | 'assistant'
  final String content;
  const AiMessage({required this.role, required this.content});
  Map<String, dynamic> toJson() => {'role': role, 'content': content};
}

// ── AI 客户端 ────────────────────────────────────────────────────────
class AiClient {
  final AiProvider provider;

  AiClient(this.provider);

  // 流式聊天：每收到一个 token 调用 onToken，完成后 Future 结束
  Future<String> chatStream(
    List<AiMessage> messages, {
    required void Function(String token) onToken,
    int maxTokens = 2048,
  }) async {
    return switch (provider.protocol) {
      AiProtocol.openAi => _openAiStream(messages, onToken, maxTokens),
      AiProtocol.anthropic => _anthropicStream(messages, onToken, maxTokens),
    };
  }

  // 非流式聊天（快速调用，用于意图检测等场景）
  Future<String> chat(List<AiMessage> messages, {int maxTokens = 512}) async {
    return switch (provider.protocol) {
      AiProtocol.openAi => _openAiChat(messages, maxTokens),
      AiProtocol.anthropic => _anthropicChat(messages, maxTokens),
    };
  }

  // ── OpenAI 兼容流式 ────────────────────────────────────────────────
  Future<String> _openAiStream(
    List<AiMessage> messages,
    void Function(String) onToken,
    int maxTokens,
  ) async {
    final uri = Uri.parse('${provider.baseUrl}/chat/completions');
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

    final response = await req.send();
    if (response.statusCode != 200) {
      final body = await response.stream.bytesToString();
      throw Exception('API 错误 ${response.statusCode}: $body');
    }

    final buf = StringBuffer();
    await for (final chunk in response.stream.transform(utf8.decoder)) {
      for (final line in chunk.split('\n')) {
        final trimmed = line.trim();
        if (!trimmed.startsWith('data: ')) continue;
        final data = trimmed.substring(6);
        if (data == '[DONE]') break;
        try {
          final json = jsonDecode(data) as Map<String, dynamic>;
          final delta = (json['choices'] as List?)
              ?.firstOrNull?['delta']?['content'] as String?;
          if (delta != null && delta.isNotEmpty) {
            buf.write(delta);
            onToken(delta);
          }
        } catch (_) {}
      }
    }
    return buf.toString();
  }

  // ── OpenAI 兼容非流式 ──────────────────────────────────────────────
  Future<String> _openAiChat(List<AiMessage> messages, int maxTokens) async {
    final res = await http.post(
      Uri.parse('${provider.baseUrl}/chat/completions'),
      headers: {
        'Content-Type': 'application/json',
        'Authorization': 'Bearer ${provider.apiKey}',
      },
      body: jsonEncode({
        'model': provider.model,
        'messages': messages.map((m) => m.toJson()).toList(),
        'max_tokens': maxTokens,
      }),
    );
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
    final other  = messages.where((m) => m.role != 'system').toList();
    final body = <String, dynamic>{
      'model': provider.model,
      'max_tokens': maxTokens,
      'messages': other.map((m) => m.toJson()).toList(),
      'stream': true,
    };
    if (system.isNotEmpty) body['system'] = system.first.content;

    final req = http.Request('POST', Uri.parse('${provider.baseUrl}/v1/messages'))
      ..headers.addAll({
        'Content-Type': 'application/json',
        'x-api-key': provider.apiKey,
        'anthropic-version': '2023-06-01',
      })
      ..body = jsonEncode(body);

    final response = await req.send();
    if (response.statusCode != 200) {
      final b = await response.stream.bytesToString();
      throw Exception('Anthropic 错误 ${response.statusCode}: $b');
    }

    final buf = StringBuffer();
    await for (final chunk in response.stream.transform(utf8.decoder)) {
      for (final line in chunk.split('\n')) {
        final trimmed = line.trim();
        if (!trimmed.startsWith('data: ')) continue;
        try {
          final json = jsonDecode(trimmed.substring(6)) as Map<String, dynamic>;
          if (json['type'] == 'content_block_delta') {
            final delta = json['delta']?['text'] as String?;
            if (delta != null && delta.isNotEmpty) {
              buf.write(delta);
              onToken(delta);
            }
          }
        } catch (_) {}
      }
    }
    return buf.toString();
  }

  // ── Anthropic 非流式 ───────────────────────────────────────────────
  Future<String> _anthropicChat(List<AiMessage> messages, int maxTokens) async {
    final system = messages.where((m) => m.role == 'system').toList();
    final other  = messages.where((m) => m.role != 'system').toList();
    final body = <String, dynamic>{
      'model': provider.model,
      'max_tokens': maxTokens,
      'messages': other.map((m) => m.toJson()).toList(),
    };
    if (system.isNotEmpty) body['system'] = system.first.content;

    final res = await http.post(
      Uri.parse('${provider.baseUrl}/v1/messages'),
      headers: {
        'Content-Type': 'application/json',
        'x-api-key': provider.apiKey,
        'anthropic-version': '2023-06-01',
      },
      body: jsonEncode(body),
    );
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
