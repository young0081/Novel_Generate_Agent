// studio_screen.dart — AI 创作主屏（独立版 v2）
// 修复：逐字打字机流式 + 推理/等待动画 + 历史会话管理

import 'dart:async';
import 'dart:collection';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../main.dart' show InkPalette;
import '../motion.dart';
import '../ai_client.dart';
import '../storage.dart';
import '../rich_text.dart';

// ── 消息角色 ──────────────────────────────────────────────────────
enum _Role { user, assistant }

class _Message {
  final _Role role;
  final String text;           // 正式回复文本（逐字打出的）
  final bool streaming;        // AI 正在生成中
  final bool reasoning;        // 等待第一个 token（显示脉冲动画）
  final String reasoningText;  // 思维链内容（DeepSeek-R1 / o1 等）
  const _Message({
    required this.role,
    required this.text,
    this.streaming = false,
    this.reasoning = false,
    this.reasoningText = '',
  });
  _Message copyWith({
    String? text,
    bool? streaming,
    bool? reasoning,
    String? reasoningText,
  }) =>
      _Message(
        role: role,
        text: text ?? this.text,
        streaming: streaming ?? this.streaming,
        reasoning: reasoning ?? this.reasoning,
        reasoningText: reasoningText ?? this.reasoningText,
      );
}

// ── 打字机控制器 ──────────────────────────────────────────────────
// 把收到的 token 拆成单个字符，以固定间隔逐字打出，给 setState 通知。
class _TypewriterController {
  final Queue<String> _queue = Queue();
  Timer? _timer;
  String displayed = '';
  bool active = false;

  // 每字间隔（ms）— 中文字符约25ms，高频密字时自动加速
  static const _baseMs = 22;

  void feed(String token) {
    for (final ch in token.characters) {
      _queue.add(ch);
    }
  }

  void start(void Function() tick) {
    active = true;
    _timer ??= Timer.periodic(
      const Duration(milliseconds: _baseMs),
      (_) {
        if (_queue.isEmpty) return;
        // 积压超40字时加速，避免流式结束后拖尾太久
        final batch = _queue.length > 40 ? 3 : 1;
        for (var i = 0; i < batch && _queue.isNotEmpty; i++) {
          displayed += _queue.removeFirst();
        }
        tick();
      },
    );
  }

  /// 等待队列打完（最多1秒）
  Future<void> flush(void Function() tick) async {
    final deadline = DateTime.now().add(const Duration(seconds: 2));
    while (_queue.isNotEmpty && DateTime.now().isBefore(deadline)) {
      await Future<void>.delayed(const Duration(milliseconds: _baseMs));
      while (_queue.isNotEmpty) {
        displayed += _queue.removeFirst();
      }
      tick();
    }
  }

  void dispose() {
    _timer?.cancel();
    _timer = null;
    active = false;
  }

  void reset() {
    _timer?.cancel();
    _timer = null;
    displayed = '';
    _queue.clear();
    active = false;
  }
}

// ── 快捷指令 ──────────────────────────────────────────────────────
const _quickPrompts = [
  '续写这一段，保持人物性格一致',
  '把这段改写得更有张力',
  '给主角加一段内心独白',
  '为这一章写一个转折点',
  '检查前后设定是否矛盾',
];

// ── StudioScreen ──────────────────────────────────────────────────
class StudioScreen extends StatefulWidget {
  final AiProvider? provider;
  final VoidCallback onGoSettings;
  const StudioScreen({
    super.key,
    required this.provider,
    required this.onGoSettings,
  });

  @override
  State<StudioScreen> createState() => _StudioScreenState();
}

class _StudioScreenState extends State<StudioScreen> {
  // 当前会话
  ConversationRecord? _record;
  final List<_Message> _messages = [];
  final TextEditingController _input = TextEditingController();
  final ScrollController _scroll = ScrollController();
  bool _sending = false;

  // 打字机
  final _TypewriterController _typer = _TypewriterController();

  // 历史抽屉
  bool _historyOpen = false;
  List<ConversationRecord> _history = [];
  bool _historyLoading = false;

  @override
  void dispose() {
    _typer.dispose();
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

  // ── 历史会话 ───────────────────────────────────────────────────
  Future<void> _loadHistory() async {
    setState(() => _historyLoading = true);
    _history = await LocalStorage.instance.listConversations();
    if (mounted) setState(() => _historyLoading = false);
  }

  void _openHistory() {
    _loadHistory();
    setState(() => _historyOpen = true);
  }

  void _resumeConversation(ConversationRecord rec) {
    setState(() {
      _historyOpen = false;
      _record = rec;
      _messages.clear();
      for (final m in rec.messages) {
        _messages.add(_Message(
          role: m.role == 'user' ? _Role.user : _Role.assistant,
          text: m.content,
        ));
      }
    });
  }

  void _newConversation() {
    setState(() {
      _historyOpen = false;
      _record = null;
      _messages.clear();
      _typer.reset();
    });
  }

  Future<void> _saveConversation() async {
    final provider = widget.provider;
    if (provider == null || _messages.isEmpty) return;

    final convMsgs = _messages
        .where((m) => !m.streaming && !m.reasoning && m.text.isNotEmpty)
        .map((m) => ConversationMessage(
              role: m.role == _Role.user ? 'user' : 'assistant',
              content: m.text,
              timestamp: DateTime.now(),
            ))
        .toList();
    if (convMsgs.isEmpty) return;

    final rec = _record ??
        ConversationRecord.create(
          providerName: provider.name,
          modelName: provider.model,
        );
    rec
      ..title = ConversationRecord.titleFrom(convMsgs)
      ..messages = convMsgs
      ..providerName = provider.name
      ..modelName = provider.model;
    _record = rec;
    await LocalStorage.instance.saveConversation(rec);
  }

  // ── 系统提示（注入记忆库上下文） ───────────────────────────────
  Future<String> _buildSystemPrompt() async {
    final memories = await LocalStorage.instance.listMemories();
    final buf = StringBuffer(
        '你是一位专业的小说创作助手，文笔精炼而富有文学性。用简体中文回复。');
    if (memories.isNotEmpty) {
      buf.writeln('\n\n# 当前作品设定（必须严格遵守）');
      for (final m in memories.take(12)) {
        final label = switch (m.kind) {
          'character' => '人物',
          'worldbuilding' => '世界观',
          'plot' => '情节',
          'foreshadow' => '伏笔',
          'lore' => '设定',
          _ => '其他',
        };
        buf.writeln(
            '- 【$label】${m.title}: '
            '${m.content.length > 120 ? m.content.substring(0, 120) : m.content}');
      }
      buf.writeln('\n续写或创作时必须与以上设定保持一致。');
    }
    return buf.toString();
  }

  // ── 发送消息 ───────────────────────────────────────────────────
  Future<void> _send([String? override]) async {
    final text = (override ?? _input.text).trim();
    if (text.isEmpty || _sending) return;
    final provider = widget.provider;
    if (provider == null || !provider.isConfigured) {
      _showConfigPrompt(); return;
    }
    _input.clear();
    HapticFeedback.lightImpact();

    // 1. 加用户消息 + 占位推理气泡
    setState(() {
      _messages.add(_Message(role: _Role.user, text: text));
      _messages.add(const _Message(
          role: _Role.assistant, text: '', streaming: true, reasoning: true));
      _sending = true;
    });
    _typer.reset();
    _scrollToBottom();

    try {
      final systemPrompt = await _buildSystemPrompt();
      final history = _messages
          .where((m) => !m.streaming && !m.reasoning && m.text.isNotEmpty)
          .toList().reversed.take(8).toList().reversed
          .map((m) => AiMessage(
                role: m.role == _Role.user ? 'user' : 'assistant',
                content: m.text))
          .toList();

      final client = AiClient(provider);

      // 2. 思维链累积缓冲（reasoning_content 字段）
      bool firstToken = true;
      final reasoningBuf = StringBuffer();

      // 启动打字机（定时器）
      _typer.start(() {
        if (!mounted) return;
        setState(() {
          final idx = _messages.lastIndexWhere((m) => m.streaming);
          if (idx != -1) {
            _messages[idx] = _messages[idx].copyWith(
              text: _typer.displayed,
              reasoning: false,
            );
          }
        });
        _scrollToBottom();
      });

      await client.chatStream(
        [AiMessage(role: 'system', content: systemPrompt), ...history],
        onReasoning: (chunk) {
          if (!mounted) return;
          reasoningBuf.write(chunk);
          // 实时更新思维链展示（保持 reasoning:true，仅更新 reasoningText）
          setState(() {
            final idx = _messages.lastIndexWhere((m) => m.streaming);
            if (idx != -1) {
              _messages[idx] = _messages[idx].copyWith(
                reasoningText: reasoningBuf.toString(),
              );
            }
          });
          _scrollToBottom();
        },
        onToken: (token) {
          if (!mounted) return;
          if (firstToken) {
            firstToken = false;
            setState(() {
              final idx = _messages.lastIndexWhere((m) => m.streaming);
              if (idx != -1) {
                _messages[idx] =
                    _messages[idx].copyWith(reasoning: false);
              }
            });
          }
          _typer.feed(token);
        },
      );

      // 3. 等打字机队列全部打完
      await _typer.flush(() {
        if (!mounted) return;
        setState(() {
          final idx = _messages.lastIndexWhere((m) => m.streaming);
          if (idx != -1) {
            _messages[idx] = _messages[idx].copyWith(text: _typer.displayed);
          }
        });
      });

      if (!mounted) return;
      final finalText = _typer.displayed;
      final finalReasoning = reasoningBuf.toString();
      setState(() {
        final idx = _messages.lastIndexWhere((m) => m.streaming);
        if (idx != -1) {
          _messages[idx] = _messages[idx].copyWith(
            text: finalText,
            streaming: false,
            reasoning: false,
            reasoningText: finalReasoning,
          );
        }
      });
      _typer.reset();

      // 4. 自动保存会话
      await _saveConversation();
    } catch (e) {
      if (!mounted) return;
      _typer.dispose();
      setState(() {
        final idx = _messages.lastIndexWhere((m) => m.streaming);
        if (idx != -1) {
          _messages[idx] = _messages[idx].copyWith(
            text: '调用失败：$e\n\n请检查「设置」中的 API Key 与网络。',
            streaming: false, reasoning: false);
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
            '前往「设置」填入 API Key（支持 DeepSeek / OpenAI / Kimi / Claude 等）即可开始创作。',
            style: TextStyle(
                fontSize: 13.5, color: InkPalette.ink3, height: 1.6)),
        actions: [
          TextButton(
              onPressed: () => Navigator.of(ctx).pop(),
              child: const Text('稍后')),
          FilledButton(
              onPressed: () {
                Navigator.of(ctx).pop();
                widget.onGoSettings();
              },
              child: const Text('去配置')),
        ],
      ),
    );
  }

  Future<void> _saveAsChapter() async {
    final lastAi = _messages.lastWhere(
      (m) => m.role == _Role.assistant && !m.streaming && m.text.isNotEmpty,
      orElse: () =>
          const _Message(role: _Role.assistant, text: ''),
    );
    if (lastAi.text.isEmpty) return;
    final ctrl = TextEditingController(text: '新章节');
    final title = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: InkPalette.paperHi,
        title: const Text('存为章节',
            style: TextStyle(fontSize: 16)),
        content: TextField(controller: ctrl, autofocus: true,
            decoration: const InputDecoration(labelText: '章节标题')),
        actions: [
          TextButton(
              onPressed: () => Navigator.of(ctx).pop(),
              child: const Text('取消')),
          FilledButton(
              onPressed: () => Navigator.of(ctx).pop(ctrl.text.trim()),
              child: const Text('保存')),
        ],
      ),
    );
    if (title == null || title.isEmpty) return;
    final ch = Chapter.create(title)..content = lastAi.text;
    await LocalStorage.instance.saveChapter(ch);
    if (!mounted) return;
    ScaffoldMessenger.of(context)
        .showSnackBar(SnackBar(content: Text('已保存章节「$title」')));
  }

  bool get _hasSaveableReply => _messages.any(
      (m) => m.role == _Role.assistant && !m.streaming && m.text.isNotEmpty);

  @override
  Widget build(BuildContext context) {
    return Stack(
      children: [
        Column(
          children: [
            _HeaderBar(
              provider: widget.provider,
              canSave: _hasSaveableReply,
              hasHistory: true,
              onSave: _saveAsChapter,
              onHistory: _openHistory,
              onNew: _newConversation,
            ),
            Expanded(
              child: _messages.isEmpty
                  ? _EmptyArea(onChip: _send)
                  : _MessageList(messages: _messages, controller: _scroll),
            ),
            _InputBar(
              controller: _input,
              sending: _sending,
              onSend: _send,
            ),
          ],
        ),
        // 历史会话抽屉（从右滑入）
        if (_historyOpen)
          _HistoryDrawer(
            records: _history,
            loading: _historyLoading,
            onClose: () => setState(() => _historyOpen = false),
            onResume: _resumeConversation,
            onDelete: (id) async {
              await LocalStorage.instance.deleteConversation(id);
              await _loadHistory();
            },
          ),
      ],
    );
  }
}

// ── 顶部栏 ────────────────────────────────────────────────────────
class _HeaderBar extends StatelessWidget {
  final AiProvider? provider;
  final bool canSave;
  final bool hasHistory;
  final VoidCallback onSave;
  final VoidCallback onHistory;
  final VoidCallback onNew;
  const _HeaderBar({
    required this.provider, required this.canSave,
    required this.hasHistory, required this.onSave,
    required this.onHistory, required this.onNew,
  });

  @override
  Widget build(BuildContext context) {
    final configured = provider?.isConfigured ?? false;
    return Container(
      padding: const EdgeInsets.fromLTRB(14, 10, 10, 10),
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
              borderRadius: BorderRadius.circular(8)),
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
                  style: TextStyle(fontSize: 11,
                    color: configured ? InkPalette.ink4 : InkPalette.cinnabar),
                  overflow: TextOverflow.ellipsis),
              ],
            ),
          ),
          // 新建会话
          IconButton(
            onPressed: onNew, tooltip: '新建对话',
            icon: const Icon(Icons.add_comment_outlined,
              size: 20, color: InkPalette.ink3)),
          // 历史记录
          IconButton(
            onPressed: onHistory, tooltip: '历史对话',
            icon: const Icon(Icons.history_rounded,
              size: 20, color: InkPalette.ink3)),
          // 存为章节
          if (canSave)
            IconButton(
              onPressed: onSave, tooltip: '存为章节',
              icon: const Icon(Icons.bookmark_add_outlined,
                size: 20, color: InkPalette.cinnabar)),
        ],
      ),
    );
  }
}

// ── 历史会话抽屉 ──────────────────────────────────────────────────
class _HistoryDrawer extends StatelessWidget {
  final List<ConversationRecord> records;
  final bool loading;
  final VoidCallback onClose;
  final void Function(ConversationRecord) onResume;
  final void Function(String id) onDelete;
  const _HistoryDrawer({
    required this.records, required this.loading,
    required this.onClose, required this.onResume,
    required this.onDelete,
  });

  String _fmt(DateTime dt) {
    final diff = DateTime.now().difference(dt);
    if (diff.inMinutes < 1) return '刚刚';
    if (diff.inHours < 1) return '${diff.inMinutes}分钟前';
    if (diff.inDays < 1) return '${diff.inHours}小时前';
    if (diff.inDays < 7) return '${diff.inDays}天前';
    return '${dt.month}/${dt.day}';
  }

  @override
  Widget build(BuildContext context) {
    return Positioned.fill(
      child: Stack(
        children: [
          // 遮罩
          GestureDetector(
            onTap: onClose,
            child: Container(color: const Color(0x66000000)),
          ),
          // 抽屉面板
          Positioned(
            right: 0, top: 0, bottom: 0,
            width: MediaQuery.of(context).size.width * 0.85,
            child: Container(
              decoration: const BoxDecoration(
                color: InkPalette.paperHi,
                border: Border(
                  left: BorderSide(color: InkPalette.line, width: 0.8)),
              ),
              child: SafeArea(
                child: Column(
                  children: [
                    // 抽屉头部
                    Container(
                      padding: const EdgeInsets.fromLTRB(16, 12, 8, 12),
                      decoration: const BoxDecoration(
                        border: Border(
                          bottom: BorderSide(
                            color: InkPalette.line, width: 0.8))),
                      child: Row(
                        children: [
                          const Expanded(
                            child: Text('历史对话',
                              style: TextStyle(fontSize: 16,
                                fontWeight: FontWeight.w700,
                                color: InkPalette.ink)),
                          ),
                          IconButton(
                            onPressed: onClose,
                            icon: const Icon(Icons.close_rounded,
                              size: 22, color: InkPalette.ink3)),
                        ],
                      ),
                    ),
                    // 列表
                    Expanded(
                      child: loading
                          ? const Center(child: CircularProgressIndicator(
                              valueColor: AlwaysStoppedAnimation<Color>(
                                InkPalette.cinnabar)))
                          : records.isEmpty
                              ? const Center(child: Padding(
                                  padding: EdgeInsets.all(24),
                                  child: Text('暂无历史对话',
                                    style: TextStyle(
                                      color: InkPalette.ink3,
                                      fontSize: 13.5))))
                              : ListView.separated(
                                  padding: const EdgeInsets.symmetric(
                                    vertical: 8),
                                  itemCount: records.length,
                                  separatorBuilder: (_, __) =>
                                      const Divider(height: 0,
                                        color: InkPalette.line,
                                        indent: 16, endIndent: 16),
                                  itemBuilder: (context, i) {
                                    final rec = records[i];
                                    return Dismissible(
                                      key: Key(rec.id),
                                      direction: DismissDirection.endToStart,
                                      background: Container(
                                        alignment: Alignment.centerRight,
                                        padding: const EdgeInsets.only(right: 20),
                                        color: InkPalette.cinnabar,
                                        child: const Icon(
                                          Icons.delete_outline_rounded,
                                          color: Colors.white),
                                      ),
                                      onDismissed: (_) => onDelete(rec.id),
                                      child: ListTile(
                                        contentPadding:
                                            const EdgeInsets.symmetric(
                                              horizontal: 16, vertical: 4),
                                        title: Text(rec.title,
                                          style: const TextStyle(
                                            fontSize: 14,
                                            fontWeight: FontWeight.w500,
                                            color: InkPalette.ink),
                                          maxLines: 1,
                                          overflow: TextOverflow.ellipsis),
                                        subtitle: Column(
                                          crossAxisAlignment:
                                              CrossAxisAlignment.start,
                                          children: [
                                            if (rec.preview.isNotEmpty)
                                              Text(rec.preview,
                                                style: const TextStyle(
                                                  fontSize: 12,
                                                  color: InkPalette.ink3),
                                                maxLines: 1,
                                                overflow: TextOverflow.ellipsis),
                                            Text(
                                              '${rec.modelName}  ·  ${_fmt(rec.updatedAt)}',
                                              style: const TextStyle(
                                                fontSize: 11,
                                                color: InkPalette.inkGhost)),
                                          ],
                                        ),
                                        onTap: () => onResume(rec),
                                      ),
                                    );
                                  },
                                ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ],
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
        if (msg.role == _Role.user) return _UserBubble(text: msg.text);
        if (msg.reasoning && msg.reasoningText.isEmpty) return const _ReasoningBubble();
        return _AssistantBubble(
          text: msg.text,
          streaming: msg.streaming,
          reasoningText: msg.reasoningText,
          isReasoning: msg.reasoning,
        );
      },
    );
  }
}

// ── 推理等待动画气泡（Task #8）─────────────────────────────────
class _ReasoningBubble extends StatefulWidget {
  const _ReasoningBubble();
  @override
  State<_ReasoningBubble> createState() => _ReasoningBubbleState();
}

class _ReasoningBubbleState extends State<_ReasoningBubble>
    with SingleTickerProviderStateMixin {
  late final AnimationController _ctrl = AnimationController(
      vsync: this, duration: const Duration(milliseconds: 1500))..repeat();

  @override
  void dispose() { _ctrl.dispose(); super.dispose(); }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 12, right: 60),
      child: Row(crossAxisAlignment: CrossAxisAlignment.end, children: [
        _AiAvatar(),
        const SizedBox(width: 8),
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
          decoration: BoxDecoration(
            color: InkPalette.paperHi,
            borderRadius: const BorderRadius.only(
              topLeft: Radius.circular(3), topRight: Radius.circular(14),
              bottomLeft: Radius.circular(14), bottomRight: Radius.circular(14)),
            border: Border.all(color: InkPalette.line, width: 0.8)),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text('正在运笔…',
                style: TextStyle(fontSize: 11.5,
                  color: InkPalette.ink4, fontStyle: FontStyle.italic)),
              const SizedBox(height: 8),
              AnimatedBuilder(
                animation: _ctrl,
                builder: (_, __) => Row(
                  mainAxisSize: MainAxisSize.min,
                  children: List.generate(3, (i) {
                    final phase = (_ctrl.value + i / 3) % 1.0;
                    final w = 12.0 + 32.0 *
                        (phase < 0.5 ? phase * 2 : (1 - phase) * 2);
                    return Padding(
                      padding: const EdgeInsets.only(right: 5),
                      child: Container(
                        width: w, height: 3.5,
                        decoration: BoxDecoration(
                          color: InkPalette.cinnabar.withValues(alpha: 0.55),
                          borderRadius: BorderRadius.circular(2))),
                    );
                  }),
                ),
              ),
            ],
          ),
        ),
      ]),
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
      padding: const EdgeInsets.only(bottom: 12, left: 48),
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
                  topLeft: Radius.circular(14), topRight: Radius.circular(14),
                  bottomLeft: Radius.circular(14), bottomRight: Radius.circular(3))),
              child: Text(text,
                style: const TextStyle(
                  fontSize: 13.5, color: InkPalette.paperHi, height: 1.55)),
            ),
          ),
          const SizedBox(width: 8),
          const CircleAvatar(
            radius: 14, backgroundColor: InkPalette.cinnabarWash,
            child: Icon(Icons.person_rounded, size: 16, color: InkPalette.cinnabar)),
        ],
      ),
    );
  }
}

// ── AI 气泡（含思维链 + 逐字光标）─────────────────────────────────
class _AssistantBubble extends StatelessWidget {
  final String text;
  final bool streaming;
  final String reasoningText;
  final bool isReasoning; // 正在输出思维链（尚未开始正式回复）
  const _AssistantBubble({
    required this.text,
    required this.streaming,
    this.reasoningText = '',
    this.isReasoning = false,
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 12, right: 48),
      child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
        _AiAvatar(),
        const SizedBox(width: 8),
        Flexible(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              // ── 思维链（有内容时展示可折叠块）──
              if (reasoningText.isNotEmpty)
                Padding(
                  padding: const EdgeInsets.only(bottom: 6),
                  child: _ReasoningChain(
                    text: reasoningText,
                    streaming: isReasoning,
                  ),
                ),
              // ── 正式回复气泡 ──
              Container(
                padding: const EdgeInsets.symmetric(
                    horizontal: 14, vertical: 10),
                decoration: BoxDecoration(
                  color: InkPalette.paperHi,
                  borderRadius: const BorderRadius.only(
                    topLeft: Radius.circular(3),
                    topRight: Radius.circular(14),
                    bottomLeft: Radius.circular(14),
                    bottomRight: Radius.circular(14)),
                  border: Border.all(color: InkPalette.line, width: 0.8)),
                child: text.isEmpty
                    ? (isReasoning ? const SizedBox.shrink() : const _ThinkingDots())
                    : Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          // 流式期间用纯文本（保证打字机流畅），
                          // 完成后切换为富文本渲染（标题/加粗/列表等）
                          streaming
                              ? SelectableText(text,
                                  style: const TextStyle(
                                    fontSize: 13.5,
                                    color: InkPalette.ink,
                                    height: 1.6))
                              : InkRichText(
                                  text: text,
                                  baseStyle: const TextStyle(
                                    fontSize: 13.5,
                                    color: InkPalette.ink,
                                    height: 1.6)),
                          if (streaming)
                            const Padding(
                              padding: EdgeInsets.only(top: 3),
                              child: _InkCaret()),
                        ],
                      ),
              ),
            ],
          ),
        ),
      ]),
    );
  }
}

// AI 头像（共用）
// ── 思维链可折叠块（Task #10）────────────────────────────────────
class _ReasoningChain extends StatefulWidget {
  final String text;
  final bool streaming; // 思维链还在增长时为 true
  const _ReasoningChain({required this.text, required this.streaming});

  @override
  State<_ReasoningChain> createState() => _ReasoningChainState();
}

class _ReasoningChainState extends State<_ReasoningChain> {
  bool _expanded = true; // 默认展开；思维链结束后自动折叠

  @override
  void didUpdateWidget(_ReasoningChain old) {
    super.didUpdateWidget(old);
    // 模型开始输出正式回复时（streaming → false）自动折叠
    if (old.streaming && !widget.streaming) {
      setState(() => _expanded = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Container(
      decoration: BoxDecoration(
        color: const Color(0xFFF0EAE0), // 比宣纸稍深一点
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: InkPalette.line, width: 0.8),
      ),
      clipBehavior: Clip.antiAlias,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          // 折叠头部
          GestureDetector(
            onTap: () => setState(() => _expanded = !_expanded),
            behavior: HitTestBehavior.opaque,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
              child: Row(
                children: [
                  // 运笔脉冲小点（思维链仍在输出时）
                  if (widget.streaming) ...[
                    _PulsingDot(),
                    const SizedBox(width: 6),
                  ] else ...[
                    const Icon(Icons.psychology_outlined,
                        size: 14, color: InkPalette.ink4),
                    const SizedBox(width: 6),
                  ],
                  Expanded(
                    child: Text(
                      widget.streaming ? '正在推理…' : '查看推理过程',
                      style: const TextStyle(
                        fontSize: 12,
                        fontStyle: FontStyle.italic,
                        color: InkPalette.ink3,
                      ),
                    ),
                  ),
                  Icon(
                    _expanded
                        ? Icons.keyboard_arrow_up_rounded
                        : Icons.keyboard_arrow_down_rounded,
                    size: 16,
                    color: InkPalette.ink4,
                  ),
                ],
              ),
            ),
          ),
          // 思维链文本（展开时可见）
          if (_expanded)
            Container(
              width: double.infinity,
              padding: const EdgeInsets.fromLTRB(12, 0, 12, 10),
              child: SelectableText(
                widget.text,
                style: const TextStyle(
                  fontSize: 12,
                  color: InkPalette.ink3,
                  height: 1.55,
                  fontStyle: FontStyle.italic,
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// 脉冲小点：思维链输出中时显示
class _PulsingDot extends StatefulWidget {
  @override
  State<_PulsingDot> createState() => _PulsingDotState();
}

class _PulsingDotState extends State<_PulsingDot>
    with SingleTickerProviderStateMixin {
  late final AnimationController _c = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 800),
  )..repeat(reverse: true);

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return FadeTransition(
      opacity: Tween<double>(begin: 0.3, end: 1.0).animate(_c),
      child: Container(
        width: 7,
        height: 7,
        decoration: BoxDecoration(
          color: InkPalette.cinnabar,
          shape: BoxShape.circle,
        ),
      ),
    );
  }
}

class _AiAvatar extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    return Container(
      width: 28, height: 28,
      decoration: BoxDecoration(
        color: InkPalette.cinnabar,
        borderRadius: BorderRadius.circular(8)),
      alignment: Alignment.center,
      child: const Text('墨',
        style: TextStyle(fontSize: 13, fontWeight: FontWeight.w700,
          color: InkPalette.paperHi)),
    );
  }
}

// 逐字光标 —— 闪烁的朱砂竖线（Task #9）
class _InkCaret extends StatefulWidget {
  const _InkCaret();
  @override
  State<_InkCaret> createState() => _InkCaretState();
}
class _InkCaretState extends State<_InkCaret>
    with SingleTickerProviderStateMixin {
  late final AnimationController _c = AnimationController(
    vsync: this, duration: const Duration(milliseconds: 600))..repeat(reverse: true);
  @override void dispose() { _c.dispose(); super.dispose(); }
  @override
  Widget build(BuildContext context) {
    return FadeTransition(
      opacity: _c,
      child: Container(width: 2, height: 15, color: InkPalette.cinnabar));
  }
}

// 等待初始 token 时三点跳动
class _ThinkingDots extends StatefulWidget {
  const _ThinkingDots();
  @override State<_ThinkingDots> createState() => _ThinkingDotsState();
}
class _ThinkingDotsState extends State<_ThinkingDots>
    with SingleTickerProviderStateMixin {
  late final AnimationController _c = AnimationController(
    vsync: this, duration: const Duration(milliseconds: 900))..repeat();
  @override void dispose() { _c.dispose(); super.dispose(); }
  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _c,
      builder: (_, __) {
        final t = _c.value;
        return Row(
          mainAxisSize: MainAxisSize.min,
          children: List.generate(3, (i) {
            final phase = (t + i / 3) % 1.0;
            final scale = 0.6 + 0.4 * (phase < 0.5 ? phase * 2 : (1 - phase) * 2);
            return Padding(
              padding: const EdgeInsets.symmetric(horizontal: 2),
              child: Transform.scale(scale: scale,
                child: const CircleAvatar(radius: 4,
                  backgroundColor: InkPalette.ink3)));
          }),
        );
      },
    );
  }
}

// ── 空状态（快捷指令） ────────────────────────────────────────────
class _EmptyArea extends StatelessWidget {
  final void Function(String) onChip;
  const _EmptyArea({required this.onChip});

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      padding: const EdgeInsets.fromLTRB(16, 24, 16, 16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const _SectionLabel('常用指令'),
          const SizedBox(height: 12),
          Wrap(
            spacing: 8, runSpacing: 8,
            children: _quickPrompts.map((p) => _PromptChip(
              label: p, onTap: () => onChip(p))).toList(),
          ),
          const SizedBox(height: 28),
          const _SectionLabel('使用提示'),
          const SizedBox(height: 12),
          const _TipItem(icon: Icons.history_rounded,
            title: '点右上角「时钟」查看历史对话', body: '所有对话自动保存，随时续聊。'),
          const SizedBox(height: 8),
          const _TipItem(icon: Icons.psychology_rounded,
            title: '设定即上下文',
            body: '在「记忆」页录入人物与世界观，创作时 AI 自动带着这些设定写作。'),
          const SizedBox(height: 8),
          const _TipItem(icon: Icons.bookmark_add_rounded,
            title: '一键存章',
            body: 'AI 回复满意后点右上角书签，直接存为章节并在「章节」页继续编辑。'),
        ],
      ),
    );
  }
}

class _SectionLabel extends StatelessWidget {
  final String text;
  const _SectionLabel(this.text);
  @override
  Widget build(BuildContext context) {
    return Row(children: [
      Container(width: 3, height: 15,
        decoration: BoxDecoration(color: InkPalette.cinnabar,
          borderRadius: BorderRadius.circular(2))),
      const SizedBox(width: 8),
      Text(text, style: const TextStyle(fontSize: 13,
        fontWeight: FontWeight.w700, color: InkPalette.ink2,
        letterSpacing: 0.5)),
    ]);
  }
}

class _TipItem extends StatelessWidget {
  final IconData icon; final String title; final String body;
  const _TipItem({required this.icon, required this.title, required this.body});
  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(color: InkPalette.paperHi,
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: InkPalette.line, width: 0.8)),
      child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Container(padding: const EdgeInsets.all(6),
          decoration: BoxDecoration(color: InkPalette.cinnabarWash,
            borderRadius: BorderRadius.circular(7)),
          child: Icon(icon, size: 18, color: InkPalette.cinnabar)),
        const SizedBox(width: 10),
        Expanded(child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(title, style: const TextStyle(fontSize: 13,
              fontWeight: FontWeight.w600, color: InkPalette.ink)),
            const SizedBox(height: 3),
            Text(body, style: const TextStyle(fontSize: 12,
              color: InkPalette.ink3, height: 1.45)),
          ])),
      ]),
    );
  }
}

class _PromptChip extends StatelessWidget {
  final String label; final VoidCallback onTap;
  const _PromptChip({required this.label, required this.onTap});
  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onTap,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 7),
        decoration: BoxDecoration(color: InkPalette.paperHi,
          borderRadius: BorderRadius.circular(20),
          border: Border.all(color: InkPalette.line, width: 0.8)),
        child: Row(mainAxisSize: MainAxisSize.min, children: [
          const Icon(Icons.bolt_rounded, size: 14, color: InkPalette.cinnabar),
          const SizedBox(width: 4),
          Text(label, style: const TextStyle(
            fontSize: 12.5, color: InkPalette.ink2)),
        ]),
      ),
    );
  }
}

// ── 输入栏 ────────────────────────────────────────────────────────
class _InputBar extends StatelessWidget {
  final TextEditingController controller;
  final bool sending;
  final void Function([String?]) onSend;
  const _InputBar({required this.controller, required this.sending,
    required this.onSend});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: EdgeInsets.only(
        left: 12, right: 12, top: 8,
        bottom: MediaQuery.of(context).padding.bottom + 8),
      decoration: const BoxDecoration(
        color: InkPalette.paperHi,
        border: Border(top: BorderSide(color: InkPalette.line, width: 0.8))),
      child: Row(crossAxisAlignment: CrossAxisAlignment.end, children: [
        Expanded(
          child: TextField(
            controller: controller, minLines: 1, maxLines: 5,
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
                borderSide: BorderSide(color: InkPalette.cinnabar, width: 1.4))),
          ),
        ),
        const SizedBox(width: 8),
        AnimatedContainer(
          duration: const Duration(milliseconds: 150),
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
              : const Icon(Icons.send_rounded, size: 20, color: InkPalette.paperHi),
            onPressed: sending ? null : () => onSend()),
        ),
      ]),
    );
  }
}
