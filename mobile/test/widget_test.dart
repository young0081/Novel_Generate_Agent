import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:novel_mobile/rpc.dart';
import 'package:novel_mobile/widgets.dart';

void main() {
  test('RpcException formats as [code] message', () {
    final e = RpcException(-32601, 'method not found');
    expect(e.toString(), '[-32601] method not found');
  });

  test('RpcService stores the configured base url', () {
    final rpc = RpcService('http://192.168.1.10:3000/');
    expect(rpc.baseUrl, 'http://192.168.1.10:3000/');
  });

  testWidgets('sectionCard renders its title and children', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: ListView(
            children: [
              sectionCard('测试标题', const [Text('内部内容')]),
            ],
          ),
        ),
      ),
    );
    expect(find.text('测试标题'), findsOneWidget);
    expect(find.text('内部内容'), findsOneWidget);
  });

  testWidgets('sectionCard renders an optional subtitle', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: ListView(
            children: [
              sectionCard('标题', const [Text('内容')], subtitle: '一段说明'),
            ],
          ),
        ),
      ),
    );
    expect(find.text('一段说明'), findsOneWidget);
  });

  testWidgets('outputBox shows its text', (tester) async {
    await tester.pumpWidget(
      MaterialApp(home: Scaffold(body: outputBox('hello-output'))),
    );
    expect(find.text('hello-output'), findsOneWidget);
  });

  testWidgets('BusyLabel shows a spinner while busy and the label otherwise',
      (tester) async {
    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(body: BusyLabel(busy: true, label: '保存')),
      ),
    );
    expect(find.byType(CircularProgressIndicator), findsOneWidget);
    expect(find.text('保存'), findsNothing);

    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(body: BusyLabel(busy: false, label: '保存')),
      ),
    );
    expect(find.byType(CircularProgressIndicator), findsNothing);
    expect(find.text('保存'), findsOneWidget);
  });

  testWidgets('EmptyState shows icon, message and hint', (tester) async {
    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(
          body: EmptyState(
            icon: Icons.search_off_rounded,
            message: '空空如也',
            hint: '换个关键词',
          ),
        ),
      ),
    );
    expect(find.text('空空如也'), findsOneWidget);
    expect(find.text('换个关键词'), findsOneWidget);
    expect(find.byIcon(Icons.search_off_rounded), findsOneWidget);
  });

  testWidgets('ErrorState shows a retry button that fires its callback',
      (tester) async {
    var retried = false;
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: ErrorState(
            message: '出错了',
            onRetry: () async => retried = true,
          ),
        ),
      ),
    );
    expect(find.text('出错了'), findsOneWidget);
    await tester.tap(find.text('重试'));
    await tester.pump();
    expect(retried, isTrue);
  });

  testWidgets('showSuccessSnack displays the message', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Builder(
            builder: (context) => ElevatedButton(
              onPressed: () => showSuccessSnack(context, '已保存'),
              child: const Text('go'),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('go'));
    await tester.pump();
    expect(find.text('已保存'), findsOneWidget);
  });

  testWidgets('showErrorSnack displays the message', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Builder(
            builder: (context) => ElevatedButton(
              onPressed: () => showErrorSnack(context, '失败了'),
              child: const Text('go'),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('go'));
    await tester.pump();
    expect(find.text('失败了'), findsOneWidget);
  });
}
