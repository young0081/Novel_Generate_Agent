// chapters_screen.dart — 本地章节管理（查看 / 编辑 / 删除 / 快照）
import 'package:flutter/material.dart';
import '../main.dart' show InkPalette;
import '../motion.dart';
import '../storage.dart';
import '../widgets.dart';

class ChaptersScreen extends StatefulWidget {
  const ChaptersScreen({super.key});
  @override
  State<ChaptersScreen> createState() => _ChaptersScreenState();
}

class _ChaptersScreenState extends State<ChaptersScreen> {
  List<Chapter> _chapters = [];
  bool _loading = true;
  String? _error;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    setState(() { _loading = true; _error = null; });
    try {
      _chapters = await LocalStorage.instance.listChapters();
    } catch (e) {
      _error = '$e';
    } finally {
      if (mounted) setState(() => _loading = false);
    }
  }

  Future<void> _newChapter() async {
    final ctrl = TextEditingController();
    final title = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: InkPalette.paperHi,
        title: const Text('新建章节',
          style: TextStyle(fontSize: 16, color: InkPalette.ink)),
        content: TextField(
          controller: ctrl,
          autofocus: true,
          decoration: const InputDecoration(labelText: '章节标题'),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(),
            child: const Text('取消')),
          FilledButton(
            onPressed: () => Navigator.of(ctx).pop(ctrl.text.trim()),
            child: const Text('创建')),
        ],
      ),
    );
    if (title == null || title.isEmpty) return;
    final ch = Chapter.create(title);
    await LocalStorage.instance.saveChapter(ch);
    await _load();
    if (!mounted) return;
    _openEditor(ch);
  }

  void _openEditor(Chapter ch) {
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => _ChapterEditorPage(chapter: ch, onSaved: _load),
      ),
    );
  }

  Future<void> _delete(Chapter ch) async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: InkPalette.paperHi,
        title: const Text('删除章节',
          style: TextStyle(fontSize: 16, color: InkPalette.ink)),
        content: Text('确认删除「${ch.title}」？同时删除该章节的所有快照，操作不可撤销。',
          style: const TextStyle(fontSize: 13.5, height: 1.5,
            color: InkPalette.ink3)),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(false),
            child: const Text('取消')),
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(true),
            style: TextButton.styleFrom(foregroundColor: InkPalette.cinnabar),
            child: const Text('删除')),
        ],
      ),
    ) ?? false;
    if (!ok) return;
    await LocalStorage.instance.deleteChapter(ch.id);
    await _load();
  }

  String _fmt(DateTime dt) {
    final now = DateTime.now();
    final diff = now.difference(dt);
    if (diff.inMinutes < 1) return '刚刚';
    if (diff.inHours < 1) return '${diff.inMinutes} 分钟前';
    if (diff.inDays < 1) return '${diff.inHours} 小时前';
    if (diff.inDays < 7) return '${diff.inDays} 天前';
    return '${dt.month}/${dt.day}';
  }

  @override
  Widget build(BuildContext context) {
    if (_loading) {
      return const Center(
        child: CircularProgressIndicator(
          valueColor: AlwaysStoppedAnimation<Color>(InkPalette.cinnabar)));
    }
    if (_error != null) {
      return ErrorState(message: _error!, onRetry: _load);
    }
    return Scaffold(
      backgroundColor: InkPalette.paper,
      floatingActionButton: FloatingActionButton(
        onPressed: _newChapter,
        backgroundColor: InkPalette.cinnabar,
        foregroundColor: InkPalette.paperHi,
        child: const Icon(Icons.add_rounded),
      ),
      body: _chapters.isEmpty
          ? Center(
              child: EmptyState(
                icon: Icons.article_outlined,
                message: '暂无章节',
                hint: '点击右下角「+」新建，或在「创作」页让 AI 生成后一键保存。',
              ),
            )
          : RefreshIndicator(
              onRefresh: _load,
              color: InkPalette.cinnabar,
              child: ListView.separated(
                padding: const EdgeInsets.fromLTRB(16, 12, 16, 80),
                itemCount: _chapters.length,
                separatorBuilder: (_, __) => const SizedBox(height: 8),
                itemBuilder: (context, i) {
                  final ch = _chapters[i];
                  final wordCount = ch.content.replaceAll(RegExp(r'\s'), '').length;
                  return StaggeredEntrance(
                    index: i,
                    child: GestureDetector(
                      onTap: () => _openEditor(ch),
                      child: Container(
                        padding: const EdgeInsets.fromLTRB(14, 12, 10, 12),
                        decoration: BoxDecoration(
                          color: InkPalette.paperHi,
                          borderRadius: BorderRadius.circular(10),
                          border: Border.all(
                            color: InkPalette.line, width: 0.8)),
                        child: Row(
                          children: [
                            // 序号印章
                            Container(
                              width: 32, height: 32,
                              decoration: BoxDecoration(
                                color: InkPalette.cinnabarWash,
                                borderRadius: BorderRadius.circular(8)),
                              alignment: Alignment.center,
                              child: Text('${i + 1}',
                                style: const TextStyle(
                                  fontSize: 13, fontWeight: FontWeight.w700,
                                  color: InkPalette.cinnabar)),
                            ),
                            const SizedBox(width: 12),
                            Expanded(
                              child: Column(
                                crossAxisAlignment: CrossAxisAlignment.start,
                                children: [
                                  Text(ch.title,
                                    style: const TextStyle(
                                      fontSize: 14.5, fontWeight: FontWeight.w600,
                                      color: InkPalette.ink),
                                    maxLines: 1,
                                    overflow: TextOverflow.ellipsis),
                                  const SizedBox(height: 3),
                                  Row(
                                    children: [
                                      Text('$wordCount 字',
                                        style: const TextStyle(
                                          fontSize: 11.5, color: InkPalette.ink4)),
                                      const SizedBox(width: 10),
                                      Text(_fmt(ch.updatedAt),
                                        style: const TextStyle(
                                          fontSize: 11.5, color: InkPalette.inkGhost)),
                                    ],
                                  ),
                                ],
                              ),
                            ),
                            IconButton(
                              icon: const Icon(Icons.delete_outline_rounded,
                                size: 20, color: InkPalette.ink4),
                              onPressed: () => _delete(ch),
                            ),
                          ],
                        ),
                      ),
                    ),
                  );
                },
              ),
            ),
    );
  }
}

// ── 章节编辑器页面 ────────────────────────────────────────────────
class _ChapterEditorPage extends StatefulWidget {
  final Chapter chapter;
  final VoidCallback onSaved;
  const _ChapterEditorPage({required this.chapter, required this.onSaved});

  @override
  State<_ChapterEditorPage> createState() => _ChapterEditorPageState();
}

class _ChapterEditorPageState extends State<_ChapterEditorPage> {
  late final TextEditingController _ctrl;
  bool _dirty = false;
  bool _saving = false;

  @override
  void initState() {
    super.initState();
    _ctrl = TextEditingController(text: widget.chapter.content);
    _ctrl.addListener(() {
      if (!_dirty) setState(() => _dirty = true);
    });
  }

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    setState(() => _saving = true);
    widget.chapter.content = _ctrl.text;
    await LocalStorage.instance.saveChapter(widget.chapter);
    widget.onSaved();
    if (!mounted) return;
    setState(() { _dirty = false; _saving = false; });
    showSuccessSnack(context, '已保存');
  }

  Future<void> _createCheckpoint() async {
    final ctrl = TextEditingController(text: '第${DateTime.now().hour}时存档');
    final msg = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: InkPalette.paperHi,
        title: const Text('创建快照',
          style: TextStyle(fontSize: 16, color: InkPalette.ink)),
        content: TextField(
          controller: ctrl,
          autofocus: true,
          decoration: const InputDecoration(labelText: '备注（可选）'),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(),
            child: const Text('取消')),
          FilledButton(
            onPressed: () => Navigator.of(ctx).pop(ctrl.text.trim()),
            child: const Text('确认')),
        ],
      ),
    );
    if (msg == null) return;
    // 先保存再快照
    if (_dirty) await _save();
    await LocalStorage.instance.createCheckpoint(
      widget.chapter, msg.isEmpty ? '手动快照' : msg);
    if (!mounted) return;
    showSuccessSnack(context, '快照已创建');
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: InkPalette.paper,
      appBar: AppBar(
        backgroundColor: InkPalette.paperHi,
        title: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(widget.chapter.title,
              style: const TextStyle(fontSize: 15.5,
                fontWeight: FontWeight.w600, color: InkPalette.ink)),
            if (_dirty)
              const Text('未保存',
                style: TextStyle(fontSize: 11,
                  color: InkPalette.cinnabar, fontWeight: FontWeight.w500)),
          ],
        ),
        actions: [
          IconButton(
            icon: const Icon(Icons.camera_alt_outlined,
              size: 22, color: InkPalette.ink3),
            tooltip: '创建快照',
            onPressed: _createCheckpoint,
          ),
          Padding(
            padding: const EdgeInsets.only(right: 8),
            child: FilledButton.icon(
              onPressed: _dirty && !_saving ? _save : null,
              icon: _saving
                  ? const SizedBox(width: 14, height: 14,
                      child: CircularProgressIndicator(
                        strokeWidth: 2,
                        valueColor: AlwaysStoppedAnimation<Color>(InkPalette.paperHi)))
                  : const Icon(Icons.save_rounded, size: 16),
              label: const Text('保存'),
              style: FilledButton.styleFrom(
                padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
                textStyle: const TextStyle(fontSize: 13)),
            ),
          ),
        ],
        bottom: PreferredSize(
          preferredSize: const Size.fromHeight(0.8),
          child: Container(height: 0.8, color: InkPalette.line)),
      ),
      body: TextField(
        controller: _ctrl,
        maxLines: null,
        expands: true,
        keyboardType: TextInputType.multiline,
        style: const TextStyle(
          fontSize: 15, color: InkPalette.ink, height: 1.85,
          letterSpacing: 0.3),
        decoration: const InputDecoration(
          contentPadding: EdgeInsets.fromLTRB(20, 20, 20, 20),
          hintText: '在此书写章节内容…',
          hintStyle: TextStyle(color: InkPalette.inkGhost, fontSize: 15),
          border: InputBorder.none,
          enabledBorder: InputBorder.none,
          focusedBorder: InputBorder.none,
          fillColor: InkPalette.paper,
          filled: true,
        ),
      ),
    );
  }
}
