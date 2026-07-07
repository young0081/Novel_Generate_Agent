import 'package:flutter/material.dart';

import '../main.dart' show InkPalette;
import '../motion.dart';
import '../rpc.dart';

// ── 消息角色 ──────────────────────────────────────────────────────
enum _Role { user, assistant }

class _Message {
  final _Role role;
  final String text;
  final bool streaming;
  const _Message({required this.role, required this.text, this.streaming = false});
  _Message copyWith({String? text, bool? streaming}) =>
      _Message(role: role, text: text ?? this.text, streaming: streaming ?? this.streaming);
}

// ── 常用创作指令芯片 ──────────────────────────────────────────────
const _quickPrompts = [
  '续写这一段，保持人物性格一致',
  '把这段改写得更有张力',
  '给主角加一段内心独白',
  '为这一章写一个转折点',
  '检查前后设定是否矛盾',
];

// ── StudioScreen —— AI 创作主屏 ──────────────────────────────────
class StudioScreen extends StatefulWidget {
  final RpcService rpc;
  const StudioScreen({super.key, required this.rpc});

  @override
  State<StudioScreen> createState() => _StudioScreenState();
}

class _StudioScreenState extends State<StudioScreen>
    with SingleTickerProviderStateMixin {
  final List<_Message> _messages = [];
  final TextEditingController _input = TextEditingController();
  final ScrollController _scroll = ScrollController();
  bool _sending = false;

  // 打字机动画控制器
  late final AnimationController _dotController = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 900),
  );

  @override
  void dispose() {
    _input.dispose();
    _scroll.dispose();
    _dotController.dispose();
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

  Future<void> _send([String? override]) async {
    final text = (override ?? _input.text).trim();
    if (text.isEmpty || _sending) return;
    _input.clear();

    setState(() {
      _messages.add(_Message(role: _Role.user, text: text));
      _messages.add(_Message(
        role: _Role.assistant, text: '', streaming: true));
      _sending = true;
    });
    _dotController.repeat();
    _scrollToBottom();

    try {
      // 构建上下文（最近 6 条消息）
      final history = _messages
          .where((m) => !m.streaming)
          .toList()
          .reversed
          .take(6)
          .toList()
          .reversed
          .map((m) => {'role': m.role == _Role.user ? 'user' : 'assistant',
                        'content': m.text})
          .toList();

      final result = await widget.rpc.call('chat', {
        'messages': [
          {
            'role': 'system',
            'content': '你是一个专业的小说创作助手，风格精炼、富有文学性。'
                '请用简洁有力的中文回复，避免废话。',
          },
          ...history,
          {'role': 'user', 'content': text},
        ],
      });

      if (!mounted) return;
      final reply = result is Map
          ? (result['text'] ?? result.toString())
          : result.toString();

      setState(() {
        final idx = _messages.lastIndexWhere((m) => m.streaming);
        if (idx != -1) {
          _messages[idx] = _messages[idx].copyWith(
            text: reply, streaming: false,
          );
        }
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        final idx = _messages.lastIndexWhere((m) => m.streaming);
        if (idx != -1) {
          _messages[idx] = _messages[idx].copyWith(
            text: '连接失败：$e\n\n请先在「设置」里配置好服务器地址。',
            streaming: false,
          );
        }
      });
    } finally {
      if (mounted) {
        setState(() => _sending = false);
        _dotController.stop();
        _dotController.reset();
        _scrollToBottom();
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        // ── 顶部装饰横幅 ──
        _HeaderBanner(),
        // ── 消息列表 ──
        Expanded(
          child: _messages.isEmpty
              ? _EmptyPromptArea(onChip: _send)
              : _MessageList(
                  messages: _messages,
                  controller: _scroll,
                  dotController: _dotController,
                ),
        ),
        // ── 输入栏 ──
        _InputBar(
          controller: _input,
          sending: _sending,
          onSend: _send,
        ),
      ],
    );
  }
}

// ── 顶部水墨装饰横幅 ──────────────────────────────────────────────
class _HeaderBanner extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 12),
      decoration: const BoxDecoration(
        color: InkPalette.paperHi,
        border: Border(bottom: BorderSide(color: InkPalette.line, width: 0.8)),
      ),
      child: Row(
        children: [
          // 朱砂印章
          Container(
            width: 34, height: 34,
            decoration: BoxDecoration(
              color: InkPalette.cinnabar,
              borderRadius: BorderRadius.circular(8),
            ),
            alignment: Alignment.center,
            child: const Text('創',
              style: TextStyle(
                fontSize: 16, fontWeight: FontWeight.w700,
                color: InkPalette.paperHi, letterSpacing: 1,
              ),
            ),
          ),
          const SizedBox(width: 10),
          const Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text('AI 创作助手',
                  style: TextStyle(
                    fontSize: 14.5, fontWeight: FontWeight.w700,
                    color: InkPalette.ink,
                  ),
                ),
                Text('与 AI 共同打磨你的故事',
                  style: TextStyle(fontSize: 11.5, color: InkPalette.ink4),
                ),
              ],
            ),
          ),
          // 状态点
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
            decoration: BoxDecoration(
              color: const Color(0xFFE8F5EE),
              borderRadius: BorderRadius.circular(20),
              border: Border.all(color: const Color(0xFF4CAF50), width: 0.6),
            ),
            child: const Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                CircleAvatar(radius: 3, backgroundColor: Color(0xFF4CAF50)),
                SizedBox(width: 4),
                Text('就绪',
                  style: TextStyle(fontSize: 11, color: Color(0xFF2E7D32)),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

// ── 空状态：快捷指令区 ────────────────────────────────────────────
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
          // 水墨装饰标题
          const _InkTitle('常用指令'),
          const SizedBox(height: 12),
          Wrap(
            spacing: 8, runSpacing: 8,
            children: _quickPrompts.map((p) => _PromptChip(
              label: p, onTap: () => onChip(p),
            )).toList(),
          ),
          const SizedBox(height: 28),
          const _InkTitle('今日创作'),
          const SizedBox(height: 12),
          _TipCard(
            icon: Icons.auto_stories_rounded,
            title: '接续上文',
            body: '把你已写好的段落发送给 AI，让它在风格和情节上无缝续写。',
          ),
          const SizedBox(height: 10),
          _TipCard(
            icon: Icons.psychology_rounded,
            title: '深化人物',
            body: '描述一个角色，让 AI 分析其性格弧线，并给出强化建议。',
          ),
          const SizedBox(height: 10),
          _TipCard(
            icon: Icons.edit_note_rounded,
            title: '打磨措辞',
            body: '粘贴一段文字，告诉 AI 你想达到的风格（如「简洁有力」「古风雅致」），AI 直接改写。',
          ),
        ],
      ),
    );
  }
}

// ── 水墨风小节标题 ────────────────────────────────────────────────
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
            borderRadius: BorderRadius.circular(2),
          ),
        ),
        const SizedBox(width: 8),
        Text(text,
          style: const TextStyle(
            fontSize: 13, fontWeight: FontWeight.w700,
            color: InkPalette.ink2, letterSpacing: 0.5,
          ),
        ),
      ],
    );
  }
}

// ── 提示卡片 ──────────────────────────────────────────────────────
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
              borderRadius: BorderRadius.circular(7),
            ),
            child: Icon(icon, size: 18, color: InkPalette.cinnabar),
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(title,
                  style: const TextStyle(
                    fontSize: 13, fontWeight: FontWeight.w600,
                    color: InkPalette.ink,
                  ),
                ),
                const SizedBox(height: 3),
                Text(body,
                  style: const TextStyle(
                    fontSize: 12, color: InkPalette.ink3, height: 1.45,
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

// ── 快捷指令芯片 ──────────────────────────────────────────────────
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
              style: const TextStyle(
                fontSize: 12.5, color: InkPalette.ink2,
              ),
            ),
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
  final AnimationController dotController;

  const _MessageList({
    required this.messages,
    required this.controller,
    required this.dotController,
  });

  @override
  Widget build(BuildContext context) {
    return ListView.builder(
      controller: controller,
      padding: const EdgeInsets.fromLTRB(12, 12, 12, 8),
      itemCount: messages.length,
      itemBuilder: (context, i) {
        final msg = messages[i];
        return StaggeredEntrance(
          index: 0,
          offsetY: 8,
          child: msg.role == _Role.user
              ? _UserBubble(text: msg.text)
              : _AssistantBubble(
                  text: msg.text,
                  streaming: msg.streaming,
                  dotController: dotController,
                ),
        );
      },
    );
  }
}

// ── 用户气泡 ──────────────────────────────────────────────────────
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
              decoration: BoxDecoration(
                color: InkPalette.cinnabar,
                borderRadius: const BorderRadius.only(
                  topLeft: Radius.circular(14),
                  topRight: Radius.circular(14),
                  bottomLeft: Radius.circular(14),
                  bottomRight: Radius.circular(3),
                ),
              ),
              child: Text(text,
                style: const TextStyle(
                  fontSize: 13.5, color: InkPalette.paperHi,
                  height: 1.5,
                ),
              ),
            ),
          ),
          const SizedBox(width: 8),
          const CircleAvatar(
            radius: 14,
            backgroundColor: InkPalette.cinnabarWash,
            child: Icon(Icons.person_rounded, size: 16, color: InkPalette.cinnabar),
          ),
        ],
      ),
    );
  }
}

// ── AI 气泡 ───────────────────────────────────────────────────────
class _AssistantBubble extends StatelessWidget {
  final String text;
  final bool streaming;
  final AnimationController dotController;

  const _AssistantBubble({
    required this.text,
    required this.streaming,
    required this.dotController,
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 12, right: 40),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.end,
        children: [
          // AI 头像
          Container(
            width: 28, height: 28,
            decoration: BoxDecoration(
              color: InkPalette.cinnabar,
              borderRadius: BorderRadius.circular(8),
            ),
            alignment: Alignment.center,
            child: const Text('墨',
              style: TextStyle(
                fontSize: 13, fontWeight: FontWeight.w700,
                color: InkPalette.paperHi,
              ),
            ),
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
              child: streaming
                  ? _ThinkingDots(controller: dotController)
                  : SelectableText(text,
                      style: const TextStyle(
                        fontSize: 13.5, color: InkPalette.ink,
                        height: 1.55,
                      ),
                    ),
            ),
          ),
        ],
      ),
    );
  }
}

// ── 思考中三点动画 ────────────────────────────────────────────────
class _ThinkingDots extends AnimatedWidget {
  const _ThinkingDots({required AnimationController controller})
      : super(listenable: controller);

  @override
  Widget build(BuildContext context) {
    final t = (listenable as AnimationController).value;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: List.generate(3, (i) {
        // 每个点延迟 0.2 的相位差
        final phase = (t + i / 3) % 1.0;
        final scale = 0.6 + 0.4 * (phase < 0.5 ? phase * 2 : (1 - phase) * 2);
        return Padding(
          padding: const EdgeInsets.symmetric(horizontal: 2),
          child: Transform.scale(
            scale: scale,
            child: const CircleAvatar(
              radius: 4,
              backgroundColor: InkPalette.ink3,
            ),
          ),
        );
      }),
    );
  }
}

// ── 输入栏 ────────────────────────────────────────────────────────
class _InputBar extends StatelessWidget {
  final TextEditingController controller;
  final bool sending;
  final void Function([String?]) onSend;

  const _InputBar({
    required this.controller,
    required this.sending,
    required this.onSend,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: EdgeInsets.only(
        left: 12, right: 12, top: 8,
        bottom: MediaQuery.of(context).padding.bottom + 8,
      ),
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
              minLines: 1,
              maxLines: 5,
              textInputAction: TextInputAction.newline,
              decoration: const InputDecoration(
                hintText: '输入创作指令或粘贴文段…',
                hintStyle: TextStyle(fontSize: 13.5, color: InkPalette.inkGhost),
                contentPadding: EdgeInsets.symmetric(horizontal: 14, vertical: 10),
                border: OutlineInputBorder(
                  borderRadius: BorderRadius.all(Radius.circular(22)),
                  borderSide: BorderSide(color: InkPalette.line),
                ),
                enabledBorder: OutlineInputBorder(
                  borderRadius: BorderRadius.all(Radius.circular(22)),
                  borderSide: BorderSide(color: InkPalette.line),
                ),
                focusedBorder: OutlineInputBorder(
                  borderRadius: BorderRadius.all(Radius.circular(22)),
                  borderSide: BorderSide(color: InkPalette.cinnabar, width: 1.4),
                ),
              ),
            ),
          ),
          const SizedBox(width: 8),
          // 发送按钮
          AnimatedContainer(
            duration: Motion.fast,
            width: 42, height: 42,
            decoration: BoxDecoration(
              color: sending ? InkPalette.cinnabarWash : InkPalette.cinnabar,
              borderRadius: BorderRadius.circular(21),
            ),
            child: IconButton(
              padding: EdgeInsets.zero,
              icon: sending
                  ? const SizedBox(
                      width: 18, height: 18,
                      child: CircularProgressIndicator(
                        strokeWidth: 2,
                        valueColor: AlwaysStoppedAnimation<Color>(InkPalette.cinnabar),
                      ),
                    )
                  : const Icon(Icons.send_rounded, size: 20, color: InkPalette.paperHi),
              onPressed: sending ? null : () => onSend(),
            ),
          ),
        ],
      ),
    );
  }
}
