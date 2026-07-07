// memory_screen.dart — 本地记忆库（本地存储版，不依赖后端）
import 'package:flutter/material.dart';
import '../main.dart' show InkPalette;
import '../motion.dart';
import '../storage.dart';
import '../widgets.dart';

const _kinds = <String, String>{
  'character': '人物',
  'worldbuilding': '世界观',
  'plot': '情节',
  'foreshadow': '伏笔',
  'lore': '设定',
  'other': '其他',
};

const _kindIcons = <String, IconData>{
  'character': Icons.person_outline_rounded,
  'worldbuilding': Icons.public_rounded,
  'plot': Icons.timeline_rounded,
  'foreshadow': Icons.visibility_off_outlined,
  'lore': Icons.menu_book_rounded,
  'other': Icons.label_outline_rounded,
};

class MemoryScreen extends StatefulWidget {
  const MemoryScreen({super.key});
  @override
  State<MemoryScreen> createState() => _MemoryScreenState();
}

class _MemoryScreenState extends State<MemoryScreen> {
  List<Memory> _memories = [];
  bool _loading = true;
  String _filter = 'all';

  @override
  void initState() { super.initState(); _load(); }

  Future<void> _load() async {
    setState(() => _loading = true);
    _memories = await LocalStorage.instance.listMemories();
    if (mounted) setState(() => _loading = false);
  }

  List<Memory> get _visible => _filter == 'all'
      ? _memories
      : _memories.where((m) => m.kind == _filter).toList();

  Future<void> _edit([Memory? existing]) async {
    final result = await showModalBottomSheet<Memory>(
      context: context,
      isScrollControlled: true,
      backgroundColor: InkPalette.paperHi,
      shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(18))),
      builder: (ctx) => _MemoryEditSheet(existing: existing),
    );
    if (result == null) return;
    await LocalStorage.instance.saveMemory(result);
    await _load();
  }

  Future<void> _delete(Memory m) async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: InkPalette.paperHi,
        title: const Text('删除记忆', style: TextStyle(fontSize: 16)),
        content: Text('确认删除「${m.title}」？',
          style: const TextStyle(fontSize: 13.5, color: InkPalette.ink3)),
        actions: [
          TextButton(onPressed: () => Navigator.of(ctx).pop(false), child: const Text('取消')),
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(true),
            style: TextButton.styleFrom(foregroundColor: InkPalette.cinnabar),
            child: const Text('删除')),
        ],
      ),
    ) ?? false;
    if (!ok) return;
    await LocalStorage.instance.deleteMemory(m.id);
    await _load();
  }

  @override
  Widget build(BuildContext context) {
    if (_loading) {
      return const Center(child: CircularProgressIndicator(
        valueColor: AlwaysStoppedAnimation<Color>(InkPalette.cinnabar)));
    }
    return Scaffold(
      backgroundColor: InkPalette.paper,
      floatingActionButton: FloatingActionButton(
        onPressed: () => _edit(),
        backgroundColor: InkPalette.cinnabar,
        foregroundColor: InkPalette.paperHi,
        child: const Icon(Icons.add_rounded),
      ),
      body: Column(
        children: [
          // 分类过滤条
          Container(
            height: 46,
            decoration: const BoxDecoration(
              color: InkPalette.paperHi,
              border: Border(bottom: BorderSide(color: InkPalette.line, width: 0.8))),
            child: ListView(
              scrollDirection: Axis.horizontal,
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
              children: [
                _Chip(label: '全部', active: _filter == 'all',
                  onTap: () => setState(() => _filter = 'all')),
                ..._kinds.entries.map((e) => _Chip(
                  label: e.value, active: _filter == e.key,
                  onTap: () => setState(() => _filter = e.key))),
              ],
            ),
          ),
          Expanded(
            child: _visible.isEmpty
                ? Center(child: EmptyState(
                    icon: Icons.psychology_outlined,
                    message: '暂无记忆',
                    hint: '录入人物、世界观等设定，AI 创作时会自动带着这些设定写作。'))
                : RefreshIndicator(
                    onRefresh: _load, color: InkPalette.cinnabar,
                    child: ListView.separated(
                      padding: const EdgeInsets.fromLTRB(16, 12, 16, 80),
                      itemCount: _visible.length,
                      separatorBuilder: (_, __) => const SizedBox(height: 8),
                      itemBuilder: (context, i) {
                        final m = _visible[i];
                        return StaggeredEntrance(index: i,
                          child: GestureDetector(
                            onTap: () => _edit(m),
                            child: Container(
                              padding: const EdgeInsets.fromLTRB(14, 12, 8, 12),
                              decoration: BoxDecoration(
                                color: InkPalette.paperHi,
                                borderRadius: BorderRadius.circular(10),
                                border: Border.all(color: InkPalette.line, width: 0.8)),
                              child: Row(
                                crossAxisAlignment: CrossAxisAlignment.start,
                                children: [
                                  Container(
                                    padding: const EdgeInsets.all(6),
                                    decoration: BoxDecoration(
                                      color: InkPalette.cinnabarWash,
                                      borderRadius: BorderRadius.circular(7)),
                                    child: Icon(_kindIcons[m.kind] ?? Icons.label_outline,
                                      size: 17, color: InkPalette.cinnabar)),
                                  const SizedBox(width: 10),
                                  Expanded(
                                    child: Column(
                                      crossAxisAlignment: CrossAxisAlignment.start,
                                      children: [
                                        Row(children: [
                                          Flexible(child: Text(m.title,
                                            style: const TextStyle(fontSize: 14,
                                              fontWeight: FontWeight.w600,
                                              color: InkPalette.ink),
                                            maxLines: 1, overflow: TextOverflow.ellipsis)),
                                          const SizedBox(width: 8),
                                          Container(
                                            padding: const EdgeInsets.symmetric(
                                              horizontal: 6, vertical: 1.5),
                                            decoration: BoxDecoration(
                                              color: InkPalette.paperLo,
                                              borderRadius: BorderRadius.circular(6)),
                                            child: Text(_kinds[m.kind] ?? '其他',
                                              style: const TextStyle(
                                                fontSize: 10.5, color: InkPalette.ink3))),
                                        ]),
                                        const SizedBox(height: 4),
                                        Text(m.content,
                                          style: const TextStyle(fontSize: 12.5,
                                            color: InkPalette.ink3, height: 1.45),
                                          maxLines: 2,
                                          overflow: TextOverflow.ellipsis),
                                      ],
                                    ),
                                  ),
                                  IconButton(
                                    icon: const Icon(Icons.delete_outline_rounded,
                                      size: 19, color: InkPalette.ink4),
                                    onPressed: () => _delete(m)),
                                ],
                              ),
                            ),
                          ),
                        );
                      },
                    ),
                  ),
          ),
        ],
      ),
    );
  }
}

class _Chip extends StatelessWidget {
  final String label; final bool active; final VoidCallback onTap;
  const _Chip({required this.label, required this.active, required this.onTap});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(right: 8),
      child: GestureDetector(
        onTap: onTap,
        child: AnimatedContainer(
          duration: Motion.fast,
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 5),
          decoration: BoxDecoration(
            color: active ? InkPalette.cinnabarWash : Colors.transparent,
            borderRadius: BorderRadius.circular(16),
            border: Border.all(
              color: active ? InkPalette.cinnabar : InkPalette.line,
              width: active ? 1.1 : 0.8)),
          child: Text(label,
            style: TextStyle(fontSize: 12.5,
              fontWeight: active ? FontWeight.w600 : FontWeight.normal,
              color: active ? InkPalette.cinnabar : InkPalette.ink3)),
        ),
      ),
    );
  }
}

class _MemoryEditSheet extends StatefulWidget {
  final Memory? existing;
  const _MemoryEditSheet({this.existing});
  @override
  State<_MemoryEditSheet> createState() => _MemoryEditSheetState();
}

class _MemoryEditSheetState extends State<_MemoryEditSheet> {
  late final TextEditingController _title =
      TextEditingController(text: widget.existing?.title ?? '');
  late final TextEditingController _content =
      TextEditingController(text: widget.existing?.content ?? '');
  late String _kind = widget.existing?.kind ?? 'character';

  @override
  void dispose() { _title.dispose(); _content.dispose(); super.dispose(); }

  void _submit() {
    if (_title.text.trim().isEmpty || _content.text.trim().isEmpty) return;
    final m = widget.existing == null
        ? Memory.create(kind: _kind, title: _title.text.trim(),
            content: _content.text.trim())
        : (widget.existing!
            ..kind = _kind
            ..title = _title.text.trim()
            ..content = _content.text.trim());
    Navigator.of(context).pop(m);
  }

  @override
  Widget build(BuildContext context) {
    final bottom = MediaQuery.of(context).viewInsets.bottom;
    return Padding(
      padding: EdgeInsets.fromLTRB(20, 16, 20, bottom + 20),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(widget.existing == null ? '新增记忆' : '编辑记忆',
            style: const TextStyle(fontSize: 16, fontWeight: FontWeight.w700,
              color: InkPalette.ink)),
          const SizedBox(height: 14),
          Wrap(spacing: 8, runSpacing: 8,
            children: _kinds.entries.map((e) {
              final active = _kind == e.key;
              return GestureDetector(
                onTap: () => setState(() => _kind = e.key),
                child: Container(
                  padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
                  decoration: BoxDecoration(
                    color: active ? InkPalette.cinnabarWash : InkPalette.paperLo,
                    borderRadius: BorderRadius.circular(16),
                    border: Border.all(
                      color: active ? InkPalette.cinnabar : InkPalette.line,
                      width: active ? 1.1 : 0.8)),
                  child: Text(e.value,
                    style: TextStyle(fontSize: 12.5,
                      fontWeight: active ? FontWeight.w600 : FontWeight.normal,
                      color: active ? InkPalette.cinnabar : InkPalette.ink3)),
                ),
              );
            }).toList()),
          const SizedBox(height: 14),
          TextField(controller: _title,
            decoration: const InputDecoration(
              labelText: '标题', hintText: '如：林惊羽')),
          const SizedBox(height: 10),
          TextField(controller: _content, minLines: 3, maxLines: 6,
            decoration: const InputDecoration(
              labelText: '内容',
              hintText: '如：主角，冷静重情义，练气九层，目标是找到杀师仇人…')),
          const SizedBox(height: 16),
          SizedBox(width: double.infinity,
            child: FilledButton(
              onPressed: _submit,
              child: Text(widget.existing == null ? '保存' : '更新'))),
        ],
      ),
    );
}
}
