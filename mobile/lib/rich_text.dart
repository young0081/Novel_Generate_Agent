// rich_text.dart — 轻量富文本渲染器（水墨风格）
// 解析常见的 markdown 语法并渲染为原生 Flutter widgets，无第三方依赖。
//
// 支持的块级语法：
//   # / ## / ### 标题        → 分级墨字标题（带朱砂装饰）
//   > 引用                   → 左侧朱砂竖线 + 浅底
//   - / * / 1. 列表          → 墨点 / 数字条目
//   --- 分隔线               → 水墨渐隐线
//   ``` 代码块               → 等宽字体深底块
//
// 支持的行内语法：
//   **加粗**  *斜体*  `行内代码`  ~~删除线~~
//
// 设计原则：解析失败时永远退回纯文本，绝不吞内容。

import 'package:flutter/material.dart';
import 'main.dart' show InkPalette;

/// 把含 markdown 的文本渲染为富文本 widget 列表。
/// [baseStyle] 是正文样式；[selectable] 为 true 时正文可选中复制。
class InkRichText extends StatelessWidget {
  final String text;
  final TextStyle? baseStyle;
  final bool selectable;

  const InkRichText({
    super.key,
    required this.text,
    this.baseStyle,
    this.selectable = true,
  });

  @override
  Widget build(BuildContext context) {
    final style = baseStyle ??
        const TextStyle(fontSize: 13.5, color: InkPalette.ink, height: 1.6);
    final blocks = _parseBlocks(text);
    if (blocks.isEmpty) return const SizedBox.shrink();

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        for (var i = 0; i < blocks.length; i++) ...[
          if (i > 0) SizedBox(height: blocks[i].topGap),
          blocks[i].build(context, style, selectable),
        ],
      ],
    );
  }
}

// ── 块级解析 ─────────────────────────────────────────────────────

abstract class _Block {
  double get topGap => 8;
  Widget build(BuildContext context, TextStyle base, bool selectable);
}

List<_Block> _parseBlocks(String text) {
  final blocks = <_Block>[];
  final lines = text.split('\n');
  var i = 0;

  final para = <String>[];
  void flushPara() {
    if (para.isEmpty) return;
    blocks.add(_Paragraph(para.join('\n')));
    para.clear();
  }

  while (i < lines.length) {
    final line = lines[i];
    final trimmed = line.trimLeft();

    // 代码块 ```
    if (trimmed.startsWith('```')) {
      flushPara();
      final buf = <String>[];
      i++;
      while (i < lines.length && !lines[i].trimLeft().startsWith('```')) {
        buf.add(lines[i]);
        i++;
      }
      i++; // 跳过结尾 ```
      blocks.add(_CodeBlock(buf.join('\n')));
      continue;
    }

    // 标题 # ~ ###
    final h = RegExp(r'^(#{1,3})\s+(.+)$').firstMatch(trimmed);
    if (h != null) {
      flushPara();
      blocks.add(_Heading(h.group(2)!.trim(), h.group(1)!.length));
      i++;
      continue;
    }

    // 分隔线 --- / ***
    if (RegExp(r'^(-{3,}|\*{3,})\s*$').hasMatch(trimmed)) {
      flushPara();
      blocks.add(_Rule());
      i++;
      continue;
    }

    // 引用 >
    if (trimmed.startsWith('> ') || trimmed == '>') {
      flushPara();
      final buf = <String>[];
      while (i < lines.length) {
        final t = lines[i].trimLeft();
        if (t.startsWith('> ')) {
          buf.add(t.substring(2));
        } else if (t == '>') {
          buf.add('');
        } else {
          break;
        }
        i++;
      }
      blocks.add(_Quote(buf.join('\n')));
      continue;
    }

    // 无序列表 - / *
    if (RegExp(r'^[-*]\s+').hasMatch(trimmed)) {
      flushPara();
      final items = <String>[];
      while (i < lines.length &&
          RegExp(r'^[-*]\s+').hasMatch(lines[i].trimLeft())) {
        items.add(lines[i].trimLeft().replaceFirst(RegExp(r'^[-*]\s+'), ''));
        i++;
      }
      blocks.add(_BulletList(items));
      continue;
    }

    // 有序列表 1.
    if (RegExp(r'^\d+[.、]\s+').hasMatch(trimmed)) {
      flushPara();
      final items = <String>[];
      while (i < lines.length &&
          RegExp(r'^\d+[.、]\s+').hasMatch(lines[i].trimLeft())) {
        items.add(
            lines[i].trimLeft().replaceFirst(RegExp(r'^\d+[.、]\s+'), ''));
        i++;
      }
      blocks.add(_NumberList(items));
      continue;
    }

    // 空行 → 段落切分
    if (trimmed.isEmpty) {
      flushPara();
      i++;
      continue;
    }

    para.add(line);
    i++;
  }
  flushPara();
  return blocks;
}

// ── 行内解析（**bold** *italic* `code` ~~strike~~）───────────────

List<InlineSpan> parseInline(String text, TextStyle base) {
  final spans = <InlineSpan>[];
  // 统一 token 正则：粗体 | 斜体 | 行内代码 | 删除线
  final pattern = RegExp(
    r'\*\*([^*]+)\*\*'      // **bold**
    r'|\*([^*]+)\*'          // *italic*
    r'|`([^`]+)`'            // `code`
    r'|~~([^~]+)~~',         // ~~strike~~
  );

  var last = 0;
  for (final m in pattern.allMatches(text)) {
    if (m.start > last) {
      spans.add(TextSpan(text: text.substring(last, m.start), style: base));
    }
    if (m.group(1) != null) {
      // bold — 墨色加深加粗
      spans.add(TextSpan(
        text: m.group(1),
        style: base.copyWith(
          fontWeight: FontWeight.w700, color: InkPalette.ink),
      ));
    } else if (m.group(2) != null) {
      // italic
      spans.add(TextSpan(
        text: m.group(2),
        style: base.copyWith(fontStyle: FontStyle.italic),
      ));
    } else if (m.group(3) != null) {
      // inline code — 等宽 + 浅底
      spans.add(WidgetSpan(
        alignment: PlaceholderAlignment.middle,
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
          decoration: BoxDecoration(
            color: InkPalette.paperLo,
            borderRadius: BorderRadius.circular(4),
            border: Border.all(color: InkPalette.line, width: 0.6),
          ),
          child: Text(
            m.group(3)!,
            style: base.copyWith(
              fontFamily: 'monospace',
              fontSize: (base.fontSize ?? 13.5) - 1,
              color: InkPalette.cinnabar,
            ),
          ),
        ),
      ));
    } else if (m.group(4) != null) {
      // strike
      spans.add(TextSpan(
        text: m.group(4),
        style: base.copyWith(
          decoration: TextDecoration.lineThrough,
          color: InkPalette.ink4),
      ));
    }
    last = m.end;
  }
  if (last < text.length) {
    spans.add(TextSpan(text: text.substring(last), style: base));
  }
  return spans;
}

// ── 块级 widgets ─────────────────────────────────────────────────

class _Paragraph extends _Block {
  final String text;
  _Paragraph(this.text);

  @override
  Widget build(BuildContext context, TextStyle base, bool selectable) {
    final spans = parseInline(text, base);
    return selectable
        ? SelectableText.rich(TextSpan(children: spans))
        : Text.rich(TextSpan(children: spans));
  }
}

class _Heading extends _Block {
  final String text;
  final int level;
  _Heading(this.text, this.level);

  @override
  double get topGap => level == 1 ? 14 : 12;

  @override
  Widget build(BuildContext context, TextStyle base, bool selectable) {
    final size = switch (level) { 1 => 17.0, 2 => 15.5, _ => 14.0 };
    return Row(
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        // 朱砂题头装饰
        Container(
          width: 3,
          height: size + 2,
          margin: const EdgeInsets.only(right: 7),
          decoration: BoxDecoration(
            color: InkPalette.cinnabar,
            borderRadius: BorderRadius.circular(2)),
        ),
        Expanded(
          child: Text.rich(
            TextSpan(children: parseInline(
              text,
              base.copyWith(
                fontSize: size,
                fontWeight: FontWeight.w700,
                color: InkPalette.ink,
                height: 1.4,
              ),
            )),
          ),
        ),
      ],
    );
  }
}

class _Quote extends _Block {
  final String text;
  _Quote(this.text);

  @override
  Widget build(BuildContext context, TextStyle base, bool selectable) {
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.fromLTRB(10, 8, 10, 8),
      decoration: const BoxDecoration(
        color: Color(0xFFF0EAE0),
        border: Border(
          left: BorderSide(color: InkPalette.cinnabar, width: 2.5)),
      ),
      child: Text.rich(
        TextSpan(children: parseInline(
          text,
          base.copyWith(
            color: InkPalette.ink3,
            fontStyle: FontStyle.italic,
            fontSize: (base.fontSize ?? 13.5) - 0.5,
          ),
        )),
      ),
    );
  }
}

class _BulletList extends _Block {
  final List<String> items;
  _BulletList(this.items);

  @override
  Widget build(BuildContext context, TextStyle base, bool selectable) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final item in items)
          Padding(
            padding: const EdgeInsets.only(bottom: 4),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                // 墨点
                Container(
                  width: 5,
                  height: 5,
                  margin: EdgeInsets.only(
                    top: ((base.fontSize ?? 13.5) * (base.height ?? 1.6) - 5) / 2,
                    right: 8, left: 2),
                  decoration: const BoxDecoration(
                    color: InkPalette.cinnabar, shape: BoxShape.circle),
                ),
                Expanded(
                  child: Text.rich(
                    TextSpan(children: parseInline(item, base))),
                ),
              ],
            ),
          ),
      ],
    );
  }
}

class _NumberList extends _Block {
  final List<String> items;
  _NumberList(this.items);

  @override
  Widget build(BuildContext context, TextStyle base, bool selectable) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        for (var i = 0; i < items.length; i++)
          Padding(
            padding: const EdgeInsets.only(bottom: 4),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Container(
                  margin: const EdgeInsets.only(right: 6),
                  child: Text('${i + 1}.',
                    style: base.copyWith(
                      color: InkPalette.cinnabar,
                      fontWeight: FontWeight.w600)),
                ),
                Expanded(
                  child: Text.rich(
                    TextSpan(children: parseInline(items[i], base))),
                ),
              ],
            ),
          ),
      ],
    );
  }
}

class _Rule extends _Block {
  @override
  double get topGap => 12;

  @override
  Widget build(BuildContext context, TextStyle base, bool selectable) {
    // 水墨渐隐分隔线
    return Container(
      height: 1,
      margin: const EdgeInsets.symmetric(vertical: 2),
      decoration: const BoxDecoration(
        gradient: LinearGradient(
          colors: [
            Colors.transparent,
            InkPalette.line,
            InkPalette.paperEdge,
            InkPalette.line,
            Colors.transparent,
          ],
        ),
      ),
    );
  }
}

class _CodeBlock extends _Block {
  final String code;
  _CodeBlock(this.code);

  @override
  double get topGap => 10;

  @override
  Widget build(BuildContext context, TextStyle base, bool selectable) {
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: const Color(0xFF2B2A26), // 墨黑底
        borderRadius: BorderRadius.circular(8),
      ),
      child: SelectableText(
        code,
        style: TextStyle(
          fontFamily: 'monospace',
          fontSize: (base.fontSize ?? 13.5) - 1,
          color: const Color(0xFFE8E2D5), // 宣纸白字
          height: 1.5,
        ),
      ),
    );
  }
}
