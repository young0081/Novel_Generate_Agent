// studio_screen.dart — AI 创作主屏（独立版）
// 直连 AI API 流式输出；自动注入本地记忆库上下文（复刻桌面端 Story State 注入思路）。

import 'package:flutter/material.dart';

import '../main.dart' show InkPalette;
import '../motion.dart';
import '../ai_client.dart';
import '../storage.dart';

enum _Role { user, assistant }

class _Message {
  final _Role role;
  final String text;
  final bool streaming;
  const _Message({required this.role, required this.text, this.streaming = false});
  _Message copyWith({String? text, bool? streaming}) =>
      _Message(role: role, text: text ?? this.text, streaming: streaming ?? this.streaming);
}

const _quickPrompts = [
  '续写这一段，保持人物性格一致',
  '把这段改写得更有张力',
  '给主角加一段内心独白',
  '为这一章写一个转折点',
  '检查前后设定是否矛盾',
];

class StudioScreen extends StatefulWidget {
  /// 供应商变化时外部会重建本屏（key 变化），无需内部监听。
  final AiProvider? provider;
  final VoidCallback onGoSettings;
  const StudioScreen({super.key, required this.provider, required this.onGoSettings});

  @override
  State<StudioScreen> createState() => _StudioScreenState();
}

class _StudioScreenState extends State<StudioScreen> {
  final List<_Message> _messages = [];
  final TextEditingController _input = TextEditingController();
  final ScrollController _scroll = ScrollController();
  bool _sending = false;

  @override
  void dispose() {
    _input.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _scrollToBottom() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_scroll.hasClients) {
        _scroll.animateTo(
          _scroll.position.maxScrollExtent,
          duration: Motion.normal,
          curve: Motion.standard,
        );
      }
    });
  }

  /// 构建系统提示：注入本地记忆库里的设定（复刻桌面端 Story State 注入）
  Future<String> _buildSystemPrompt() async {
    final memories = await LocalStorage.instance.listMemories();
    final buf = StringBuffer(
      '你是一位专业的小说创作助手，文笔精炼而富有文学性。用简体中文回复。');
    if (memories.isNotEmpty) {
      buf.writeln('\n\n# 当前作品设定（必须严格遵守）');
      // 只注入前 12 条，控制 token
      for (final m in memories.take(12)) {
        buf.writeln('- 【${_kindLabel(m.kind)}】${m.title}: '
            '${m.content.length > 120 ? m.content.substring(0, 120) : m.content}');
      }
      buf.writeln('\n续写或创作时必须与以上设定保持一致，不可矛盾。');
    }
    return buf.toString();
  }

  static String _kindLabel(String kind) => switch (kind) {
    'character' => '人物',
    'worldbuilding' => '世界观',
    'plot' => '情节',
    'foreshadow' => '伏笔',
    'lore' => '设定',
    _ => '其他',
  };

  Future<void> _send([String? override]) async {
    final text = (override ?? _input.text).trim();
    if (text.isEmpty || _sending) return;

    final provider = widget.provider;
    if (provider == null || !provider.isConfigured) {
      _showConfigPrompt();
      return;
    }

    _input.clear();
    setState(() {
      _messages.add(_Message(role: _Role.user, text: text));
      _messages.add(const _Message(role: _Role.assistant, text: '', streaming: true));
      _sending = true;
    });
    _scrollToBottom();

    try {
      final systemPrompt = await _buildSystemPrompt();
      final history = _messages
          .where((m) => !m.streaming && m.text.isNotEmpty)
          .toList()
          .reversed
          .take(8)
          .toList()
          .reversed
          .map((m) => AiMessage(
                role: m.role == _Role.user ? 'user' : 'assistant',
                content: m.text,
              ))
          .toList();

      final client = AiClient(provider);
      var streamed = '';
      await client.chatStream(
        [
          AiMessage(role: 'system', content: systemPrompt),
          ...history,
        ],
        onToken: (token) {
          if (!mounted) return;
          streamed += token;
          setState(() {
            final idx = _messages.lastIndexWhere((m) => m.streaming);
            if (idx != -1) {
              _messages[idx] = _messages[idx].copyWith(text: streamed);
            }
          });
          _scrollToBottom();
        },
      );

      if (!mounted) return;
      setState(() {
        final idx = _messages.lastIndexWhere((m) => m.streaming);
        if (idx != -1) {
          _messages[idx] = _messages[idx].copyWith(streaming: false);
        }
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        final idx = _messages.lastIndexWhere((m) => m.streaming);
        if (idx != -1) {
          _messages[idx] = _messages[idx].copyWith(
            text: '调用失败：$e\n\n请检查「设置」中的 API Key 与网络。',
            streaming: false,
          );
        }
      });
    } finally {
      if (mounted) {
        setState(() => _sending = false);
        _scrollToBottom();
      }
    }
  }

  void _showConfigPrompt() {
    showDialog<void>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: InkPalette.paperHi,
        title: const Text('尚未配置 AI 模型',
          style: TextStyle(fontSize: 16, color: InkPalette.ink)),
        content: const Text(
          '前往「设置」填入你的 API Key（支持 DeepSeek / OpenAI / Kimi / 智谱 / Claude 等），即可开始创作。',
          style: TextStyle(fontSize: 13.5, color: InkPalette.ink3, height: 1.6)),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(),
            child: const Text('稍后'),
          ),
          FilledButton(
            onPressed: () {
              Navigator.of(ctx).pop();
              widget.onGoSettings();
            },
            child: const Text('去配置'),
          ),
        ],
      ),
    );
  }

  /// 把最后一条 AI 回复存为章节
  Future<void> _saveAsChapter() async {
    final lastAi = _messages.lastWhere(
      (m) => m.role == _Role.assistant && !m.streaming && m.text.isNotEmpty,
      orElse: () => const _Message(role: _Role.assistant, text: ''),
    );
    if (lastAi.text.isEmpty) return;

    final controller = TextEditingController(text: '新章节');
    final title = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: InkPalette.paperHi,
        title: const Text('存为章节', style: TextStyle(fontSize: 16)),
        content: TextField(
          controller: controller,
          autofocus: true,
          decoration: const InputDecoration(labelText: '章节标题'),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(),
            child: const Text('取消')),
          FilledButton(
            onPressed: () => Navigator.of(ctx).pop(controller.text.trim()),
            child: const Text('保存')),
        ],
      ),
    );
    if (title == null || title.isEmpty) return;

    final ch = Chapter.create(title)..content = lastAi.text;
    await LocalStorage.instance.saveChapter(ch);
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text('已保存章节「$title」')),
    );
  }

  bool get _hasSaveableReply => _messages.any(
    (m) => m.role == _Role.assistant && !m.streaming && m.text.isNotEmpty);

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        _HeaderBanner(
          provider: widget.provider,
          canSave: _hasSaveableReply,
          onSave: _saveAsChapter,
        ),
        Expanded(
          child: _messages.isEmpty
              ? _EmptyPromptArea(onChip: _send)
              : _MessageList(messages: _messages, controller: _scroll),
        ),
        _InputBar(controller: _input, sending: _sending, onSend: _send),
      ],
    );
  }
}

// ── 顶部横幅 ─────────────────────────────────────────────────────
class _HeaderBanner extends StatelessWidget {
  final AiProvider? provider;
  final bool canSave;
  final VoidCallback onSave;
  const _HeaderBanner({
    required this.provider, required this.canSave, required this.onSave});

  @override
  Widget build(BuildContext context) {
    final configured = provider?.isConfigured ?? false;
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.fromLTRB(16, 10, 12, 10),
      decoration: const BoxDecoration(
        color: InkPalette.paperHi,
        border: Border(bottom: BorderSide(color: InkPalette.line, width: 0.8)),
      ),
      child: Row(
        children: [
          Container(
            width: 32, height: 32,
            decoration: BoxDecoration(
              color: InkPalette.cinnabar,
              borderRadius: BorderRadius.circular(8),
            ),
            alignment: Alignment.center,
            child: const Text('創',
              style: TextStyle(fontSize: 15, fontWeight: FontWeight.w700,
                color: InkPalette.paperHi)),
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Text('AI 创作助手',
                  style: TextStyle(fontSize: 14, fontWeight: FontWeight.w700,
                    color: InkPalette.ink)),
                Text(
                  configured
                      ? '${provider!.name} · ${provider!.model}'
                      : '未配置模型 — 前往设置',
                  style: TextStyle(
                    fontSize: 11,
                    color: configured ? InkPalette.ink4 : InkPalette.cinnabar,
                  ),
                  overflow: TextOverflow.ellipsis,
                ),
              ],
            ),
          ),
          if (canSave)
            IconButton(
              onPressed: onSave,
              tooltip: '把 AI 回复存为章节',
              icon: const Icon(Icons.bookmark_add_outlined,
                size: 21, color: InkPalette.cinnabar),
            ),
        ],
      ),
    );
  }
}

// ── 空状态 ────────────────────────────────────────────────────────
class _EmptyPromptArea extends StatelessWidget {
  final void Function(String) onChip;
  const _EmptyPromptArea({required this.onChip});

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      padding: const EdgeInsets.fromLTRB(16, 24, 16, 16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const _InkTitle('常用指令'),
          const SizedBox(height: 12),
          Wrap(
            spacing: 8, runSpacing: 8,
            children: _quickPrompts
                .map((p) => _PromptChip(label: p, onTap: () => onChip(p)))
                .toList(),
          ),
          const SizedBox(height: 28),
          const _InkTitle('创作提示'),
          const SizedBox(height: 12),
          const _TipCard(
            icon: Icons.auto_stories_rounded,
            title: '接续上文',
            body: '把已写好的段落粘贴进来，AI 在风格和情节上无缝续写。',
          ),
          const SizedBox(height: 10),
          const _TipCard(
            icon: Icons.psychology_rounded,
            title: '设定即上下文',
            body: '在「记忆」页录入人物与世界观，创作时 AI 自动带着这些设定写作，前后不矛盾。',
          ),
          const SizedBox(height: 10),
          const _TipCard(
            icon: Icons.bookmark_add_rounded,
            title: '一键存章',
            body: '满意的 AI 回复可直接存为章节，在「文件」页继续编辑，在「快照」页做版本管理。',
          ),
        ],
      ),
    );
  }
}

class _InkTitle extends StatelessWidget {
  final String text;
  const _InkTitle(this.text);

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        Container(
          width: 3, height: 16,
          decoration: BoxDecoration(
            color: InkPalette.cinnabar,
            borderRadius: BorderRadius.circular(2)),
        ),
        const SizedBox(width: 8),
        Text(text,
          style: const TextStyle(fontSize: 13, fontWeight: FontWeight.w700,
            color: InkPalette.ink2, letterSpacing: 0.5)),
      ],
    );
  }
}

class _TipCard extends StatelessWidget {
  final IconData icon;
  final String title;
  final String body;
  const _TipCard({required this.icon, required this.title, required this.body});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: InkPalette.paperHi,
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: InkPalette.line, width: 0.8),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            padding: const EdgeInsets.all(6),
            decoration: BoxDecoration(
              color: InkPalette.cinnabarWash,
              borderRadius: BorderRadius.circular(7)),
            child: Icon(icon, size: 18, color: InkPalette.cinnabar),
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(title,
                  style: const TextStyle(fontSize: 13,
                    fontWeight: FontWeight.w600, color: InkPalette.ink)),
                const SizedBox(height: 3),
                Text(body,
                  style: const TextStyle(fontSize: 12,
                    color: InkPalette.ink3, height: 1.45)),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _PromptChip extends StatelessWidget {
  final String label;
  final VoidCallback onTap;
  const _PromptChip({required this.label, required this.onTap});

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onTap,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 7),
        decoration: BoxDecoration(
          color: InkPalette.paperHi,
          borderRadius: BorderRadius.circular(20),
          border: Border.all(color: InkPalette.line, width: 0.8),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            const Icon(Icons.bolt_rounded, size: 14, color: InkPalette.cinnabar),
            const SizedBox(width: 4),
            Text(label,
              style: const TextStyle(fontSize: 12.5, color: InkPalette.ink2)),
          ],
        ),
      ),
    );
  }
}

// ── 消息列表 ──────────────────────────────────────────────────────
class _MessageList extends StatelessWidget {
  final List<_Message> messages;
  final ScrollController controller;
  const _MessageList({required this.messages, required this.controller});

  @override
  Widget build(BuildContext context) {
    return ListView.builder(
      controller: controller,
      padding: const EdgeInsets.fromLTRB(12, 12, 12, 8),
      itemCount: messages.length,
      itemBuilder: (context, i) {
        final msg = messages[i];
        return msg.role == _Role.user
            ? _UserBubble(text: msg.text)
            : _AssistantBubble(text: msg.text, streaming: msg.streaming);
      },
    );
  }
}

class _UserBubble extends StatelessWidget {
  final String text;
  const _UserBubble({required this.text});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 12, left: 40),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.end,
        crossAxisAlignment: CrossAxisAlignment.end,
        children: [
          Flexible(
            child: Container(
              padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
              decoration: const BoxDecoration(
                color: InkPalette.cinnabar,
                borderRadius: BorderRadius.only(
                  topLeft: Radius.circular(14),
                  topRight: Radius.circular(14),
                  bottomLeft: Radius.circular(14),
                  bottomRight: Radius.circular(3),
                ),
              ),
              child: Text(text,
                style: const TextStyle(fontSize: 13.5,
                  color: InkPalette.paperHi, height: 1.5)),
            ),
          ),
          const SizedBox(width: 8),
          const CircleAvatar(
            radius: 14,
            backgroundColor: InkPalette.cinnabarWash,
            child: Icon(Icons.person_rounded, size: 16,
              color: InkPalette.cinnabar)),
        ],
      ),
    );
  }
}

class _AssistantBubble extends StatelessWidget {
  final String text;
  final bool streaming;
  const _AssistantBubble({required this.text, required this.streaming});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 12, right: 40),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.end,
        children: [
          Container(
            width: 28, height: 28,
            decoration: BoxDecoration(
              color: InkPalette.cinnabar,
              borderRadius: BorderRadius.circular(8)),
            alignment: Alignment.center,
            child: const Text('墨',
              style: TextStyle(fontSize: 13, fontWeight: FontWeight.w700,
                color: InkPalette.paperHi)),
          ),
          const SizedBox(width: 8),
          Flexible(
            child: Container(
              padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
              decoration: BoxDecoration(
                color: InkPalette.paperHi,
                borderRadius: const BorderRadius.only(
                  topLeft: Radius.circular(3),
                  topRight: Radius.circular(14),
                  bottomLeft: Radius.circular(14),
                  bottomRight: Radius.circular(14),
                ),
                border: Border.all(color: InkPalette.line, width: 0.8),
              ),
              child: text.isEmpty && streaming
                  ? const _ThinkingDots()
                  : Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        SelectableText(text,
                          style: const TextStyle(fontSize: 13.5,
                            color: InkPalette.ink, height: 1.55)),
                        if (streaming)
                          const Padding(
                            padding: EdgeInsets.only(top: 4),
                            child: _InkCaret(),
                          ),
                      ],
                    ),
            ),
          ),
        ],
      ),
    );
  }
}

// 流式输出中的墨点光标
class _InkCaret extends StatefulWidget {
  const _InkCaret();
  @override
  State<_InkCaret> createState() => _InkCaretState();
}

class _InkCaretState extends State<_InkCaret>
    with SingleTickerProviderStateMixin {
  late final AnimationController _c = AnimationController(
    vsync: this, duration: const Duration(milliseconds: 800))..repeat(reverse: true);

  @override
  void dispose() { _c.dispose(); super.dispose(); }

  @override
  Widget build(BuildContext context) {
    if (Motion.reduced(context)) {
      return Container(width: 8, height: 14, color: InkPalette.cinnabar);
    }
    return FadeTransition(
      opacity: _c,
      child: Container(width: 8, height: 14, color: InkPalette.cinnabar),
    );
  }
}

class _ThinkingDots extends StatefulWidget {
  const _ThinkingDots();
  @override
  State<_ThinkingDots> createState() => _ThinkingDotsState();
}

class _ThinkingDotsState extends State<_ThinkingDots>
    with SingleTickerProviderStateMixin {
  late final AnimationController _c = AnimationController(
    vsync: this, duration: const Duration(milliseconds: 900))..repeat();

  @override
  void dispose() { _c.dispose(); super.dispose(); }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _c,
      builder: (context, _) {
        final t = _c.value;
        return Row(
          mainAxisSize: MainAxisSize.min,
          children: List.generate(3, (i) {
            final phase = (t + i / 3) % 1.0;
            final scale = 0.6 + 0.4 * (phase < 0.5 ? phase * 2 : (1 - phase) * 2);
            return Padding(
              padding: const EdgeInsets.symmetric(horizontal: 2),
              child: Transform.scale(
                scale: scale,
                child: const CircleAvatar(
                  radius: 4, backgroundColor: InkPalette.ink3)),
            );
          }),
        );
      },
    );
  }
}

// ── 输入栏 ────────────────────────────────────────────────────────
class _InputBar extends StatelessWidget {
  final TextEditingController controller;
  final bool sending;
  final void Function([String?]) onSend;
  const _InputBar({
    required this.controller, required this.sending, required this.onSend});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: EdgeInsets.only(
        left: 12, right: 12, top: 8,
        bottom: MediaQuery.of(context).padding.bottom + 8),
      decoration: const BoxDecoration(
        color: InkPalette.paperHi,
        border: Border(top: BorderSide(color: InkPalette.line, width: 0.8)),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.end,
        children: [
          Expanded(
            child: TextField(
              controller: controller,
              minLines: 1, maxLines: 5,
              textInputAction: TextInputAction.newline,
              decoration: const InputDecoration(
                hintText: '输入创作指令或粘贴文段…',
                hintStyle: TextStyle(fontSize: 13.5, color: InkPalette.inkGhost),
                contentPadding: EdgeInsets.symmetric(horizontal: 14, vertical: 10),
                border: OutlineInputBorder(
                  borderRadius: BorderRadius.all(Radius.circular(22)),
                  borderSide: BorderSide(color: InkPalette.line)),
                enabledBorder: OutlineInputBorder(
                  borderRadius: BorderRadius.all(Radius.circular(22)),
                  borderSide: BorderSide(color: InkPalette.line)),
                focusedBorder: OutlineInputBorder(
                  borderRadius: BorderRadius.all(Radius.circular(22)),
                  borderSide: BorderSide(color: InkPalette.cinnabar, width: 1.4)),
              ),
            ),
          ),
          const SizedBox(width: 8),
          AnimatedContainer(
            duration: Motion.fast,
            width: 42, height: 42,
            decoration: BoxDecoration(
              color: sending ? InkPalette.cinnabarWash : InkPalette.cinnabar,
              borderRadius: BorderRadius.circular(21)),
            child: IconButton(
              padding: EdgeInsets.zero,
              icon: sending
                  ? const SizedBox(width: 18, height: 18,
                      child: CircularProgressIndicator(strokeWidth: 2,
                        valueColor: AlwaysStoppedAnimation<Color>(InkPalette.cinnabar)))
                  : const Icon(Icons.send_rounded, size: 20,
                      color: InkPalette.paperHi),
              onPressed: sending ? null : () => onSend(),
            ),
          ),
        ],
      ),
    );
  }
}
