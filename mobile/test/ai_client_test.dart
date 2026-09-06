import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:novel_mobile/ai_client.dart';

Stream<List<int>> fragmented(String text, List<int> sizes) async* {
  final bytes = utf8.encode(text);
  var offset = 0;
  var index = 0;
  while (offset < bytes.length) {
    final requested = sizes[index % sizes.length];
    final end = (offset + requested).clamp(0, bytes.length);
    yield bytes.sublist(offset, end);
    offset = end;
    index++;
  }
}

void main() {
  test(
    'OpenAI SSE survives JSON and UTF-8 characters split across chunks',
    () async {
      const payload =
          ': keep-alive\r\n'
          'data: {"choices":[{"delta":{"reasoning_content":"思考"}}]}\r\n\r\n'
          'data: {"choices":[{"delta":{"content":"你好"}}]}\n\n'
          'data: [DONE]\n\n';

      final events = await decodeOpenAiStream(
        fragmented(payload, [1, 2, 5, 3]),
      ).toList();

      expect(events.map((event) => event.reasoning).join(), '思考');
      expect(events.map((event) => event.content).join(), '你好');
    },
  );

  test('Anthropic SSE survives event lines split across chunks', () async {
    const payload =
        'event: content_block_delta\n'
        'data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"山"}}\n\n'
        'event: content_block_delta\n'
        'data:{"type":"content_block_delta","delta":{"type":"text_delta","text":"海"}}\n\n';

    final events = await decodeAnthropicStream(
      fragmented(payload, [4, 1, 7, 2]),
    ).toList();

    expect(events.map((event) => event.content).join(), '山海');
  });

  test(
    'SSE decoder flushes a final event without a trailing blank line',
    () async {
      final values = await decodeSseData(
        fragmented('data: final', [2, 1]),
      ).toList();

      expect(values, ['final']);
    },
  );
}
