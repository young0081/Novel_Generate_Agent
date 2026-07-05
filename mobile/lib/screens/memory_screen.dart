import 'package:flutter/material.dart';

import '../motion.dart';
import '../rpc.dart';
import '../widgets.dart';

const _kinds = <String, String>{
  'character': '人物',
  'setting': '场景设定',
  'worldbuilding': '世界观',
  'plot': '情节',
  'outline': '大纲',
  'foreshadow': '伏笔',
  'dialogue': '对话',
  'lore': '传说设定',
  'other': '其他',
};

class MemoryScreen extends StatefulWidget {
  final RpcService rpc;
  const MemoryScreen({super.key, required this.rpc});

  @override
  State<MemoryScreen> createState() => _MemoryScreenState();
}

class _MemoryScreenState extends State<MemoryScreen> {
  final _query = TextEditingController();
  final _title = TextEditingController();
  final _summary = TextEditingController();
  final _content = TextEditingController();
  final _tags = TextEditingController();
  String _kind = 'character';
  int _importance = 3;
  String _hits = '';
  bool _recalled = false;
  bool _recallError = false;
  bool _busyRecall = false;
  bool _busySave = false;

  @override
  void initState() {
    super.initState();
    _title.addListener(_onTitleChanged);
  }

  @override
  void dispose() {
    _title.removeListener(_onTitleChanged);
    _query.dispose();
    _title.dispose();
    _summary.dispose();
    _content.dispose();
    _tags.dispose();
    super.dispose();
  }

  void _onTitleChanged() => setState(() {});

  Future<void> _recall() async {
    setState(() => _busyRecall = true);
    try {
      final r = await widget.rpc.invokeTool('memory_recall', {
        'query': _query.text,
        'k': 8,
      });
      if (!mounted) return;
      setState(() {
        _hits = (r['content'] ?? '').toString();
        _recalled = true;
        _recallError = false;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _recalled = true;
        _recallError = true;
      });
      showErrorSnack(context, '检索失败：$e');
    } finally {
      if (mounted) setState(() => _busyRecall = false);
    }
  }

  Future<void> _save() async {
    setState(() => _busySave = true);
    try {
      final tags = _tags.text
          .split(RegExp(r'[,，]'))
          .map((t) => t.trim())
          .where((t) => t.isNotEmpty)
          .toList();
      final r = await widget.rpc.invokeTool('memory_save', {
        'kind': _kind,
        'title': _title.text,
        'summary': _summary.text,
        'content': _content.text,
        'tags': tags,
        'importance': _importance,
      });
      if (!mounted) return;
      if (r['ok'] == true) {
        showSuccessSnack(context, '已保存：${r['summary'] ?? r['content']}');
      } else {
        showErrorSnack(context, (r['content'] ?? '保存失败').toString());
      }
    } catch (e) {
      if (mounted) showErrorSnack(context, '保存失败：$e');
    } finally {
      if (mounted) setState(() => _busySave = false);
    }
  }

  bool get _anyBusy => _busyRecall || _busySave;

  @override
  Widget build(BuildContext context) {
    return RefreshIndicator(
      onRefresh: _recall,
      child: ListView(
        padding: const EdgeInsets.all(16),
        physics: const AlwaysScrollableScrollPhysics(),
        children: [
          StaggeredEntrance(
            index: 0,
            child:
                sectionCard('检索记忆（RAG）', icon: Icons.travel_explore_rounded, [
                  TextField(
                    controller: _query,
                    decoration: const InputDecoration(
                      labelText: '搜索，例如：北境 剑客 主角',
                      prefixIcon: Icon(Icons.search_rounded),
                    ),
                    onSubmitted: (_) => _anyBusy ? null : _recall(),
                  ),
                  const SizedBox(height: 12),
                  FilledButton.icon(
                    onPressed: _anyBusy ? null : _recall,
                    icon: BusyIcon(
                      busy: _busyRecall,
                      icon: Icons.manage_search_rounded,
                    ),
                    label: const Text('检索'),
                  ),
                  if (_recalled && _recallError)
                    const Padding(
                      padding: EdgeInsets.only(top: 14),
                      child: EmptyState(
                        icon: Icons.error_outline_rounded,
                        message: '检索失败',
                        hint: '检查后端连接后下拉刷新重试。',
                      ),
                    )
                  else if (_recalled && _hits.isEmpty)
                    const Padding(
                      padding: EdgeInsets.only(top: 14),
                      child: EmptyState(
                        icon: Icons.search_off_rounded,
                        message: '没有匹配的记忆',
                        hint: '换个关键词，或先在下面新增记忆。',
                      ),
                    )
                  else if (_hits.isNotEmpty)
                    outputBox(_hits),
                ]),
          ),
          StaggeredEntrance(
            index: 1,
            child: sectionCard('新增记忆', icon: Icons.add_box_outlined, [
              DropdownButtonFormField<String>(
                initialValue: _kind,
                decoration: const InputDecoration(
                  labelText: '类型',
                  prefixIcon: Icon(Icons.category_outlined),
                ),
                items: _kinds.entries
                    .map(
                      (e) =>
                          DropdownMenuItem(value: e.key, child: Text(e.value)),
                    )
                    .toList(),
                onChanged: (v) => setState(() => _kind = v ?? 'character'),
              ),
              const SizedBox(height: 12),
              TextField(
                controller: _title,
                decoration: const InputDecoration(
                  labelText: '标题，例如：林惊羽',
                  prefixIcon: Icon(Icons.title_rounded),
                ),
              ),
              const SizedBox(height: 12),
              TextField(
                controller: _summary,
                decoration: const InputDecoration(
                  labelText: '一句话摘要（检索时展示）',
                  prefixIcon: Icon(Icons.short_text_rounded),
                ),
              ),
              const SizedBox(height: 12),
              TextField(
                controller: _content,
                maxLines: 5,
                minLines: 3,
                decoration: const InputDecoration(
                  labelText: '详细内容',
                  alignLabelWithHint: true,
                  border: OutlineInputBorder(),
                ),
              ),
              const SizedBox(height: 12),
              TextField(
                controller: _tags,
                decoration: const InputDecoration(
                  labelText: '标签（逗号分隔）',
                  prefixIcon: Icon(Icons.label_outline_rounded),
                ),
              ),
              const SizedBox(height: 8),
              Row(
                children: [
                  const Text('重要度'),
                  Expanded(
                    child: Slider(
                      value: _importance.toDouble(),
                      min: 1,
                      max: 5,
                      divisions: 4,
                      label: '$_importance',
                      onChanged: (v) => setState(() => _importance = v.round()),
                    ),
                  ),
                  Text('$_importance'),
                ],
              ),
              const SizedBox(height: 4),
              FilledButton(
                onPressed: _anyBusy || _title.text.trim().isEmpty
                    ? null
                    : _save,
                child: BusyLabel(busy: _busySave, label: '保存记忆'),
              ),
            ]),
          ),
        ],
      ),
    );
  }
}
