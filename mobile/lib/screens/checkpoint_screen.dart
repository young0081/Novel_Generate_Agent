// checkpoint_screen.dart — 本地快照管理
import 'package:flutter/material.dart';
import '../main.dart' show InkPalette;
import '../motion.dart';
import '../storage.dart';
import '../widgets.dart';

class CheckpointScreen extends StatefulWidget {
  const CheckpointScreen({super.key});
  @override
  State<CheckpointScreen> createState() => _CheckpointScreenState();
}

class _CheckpointScreenState extends State<CheckpointScreen> {
  List<Checkpoint> _checkpoints = [];
  bool _loading = true;
  bool _manageMode = false;
  bool _batchDeleting = false;
  int _batchProgress = 0;
  final Set<String> _selectedIds = <String>{};

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    setState(() => _loading = true);
    _checkpoints = await LocalStorage.instance.listCheckpoints();
    final valid = _checkpoints.map((checkpoint) => checkpoint.id).toSet();
    _selectedIds.removeWhere((id) => !valid.contains(id));
    if (mounted) setState(() => _loading = false);
  }

  Future<void> _restore(Checkpoint cp) async {
    final ok =
        await showDialog<bool>(
          context: context,
          builder: (ctx) => AlertDialog(
            backgroundColor: InkPalette.paperHi,
            title: const Text('回滚到此快照', style: TextStyle(fontSize: 16)),
            content: Text(
              '将把「${cp.chapterTitle}」恢复到快照时的内容，'
              '当前内容会被覆盖。继续？',
              style: const TextStyle(
                fontSize: 13.5,
                height: 1.5,
                color: InkPalette.ink3,
              ),
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.of(ctx).pop(false),
                child: const Text('取消'),
              ),
              FilledButton(
                onPressed: () => Navigator.of(ctx).pop(true),
                child: const Text('回滚'),
              ),
            ],
          ),
        ) ??
        false;
    if (!ok) return;
    await LocalStorage.instance.restoreCheckpoint(cp);
    if (!mounted) return;
    showSuccessSnack(context, '已回滚到「${cp.message}」');
  }

  Future<void> _delete(Checkpoint cp) async {
    await LocalStorage.instance.deleteCheckpoint(cp.id);
    await _load();
  }

  void _toggleSelected(String id) {
    setState(() {
      if (_selectedIds.contains(id)) {
        _selectedIds.remove(id);
      } else {
        _selectedIds.add(id);
      }
    });
  }

  void _toggleAll() {
    final ids = _checkpoints.map((checkpoint) => checkpoint.id).toSet();
    setState(() {
      if (ids.isNotEmpty && ids.every(_selectedIds.contains)) {
        _selectedIds.removeAll(ids);
      } else {
        _selectedIds.addAll(ids);
      }
    });
  }

  Future<void> _deleteSelected() async {
    final selected = _checkpoints
        .where((cp) => _selectedIds.contains(cp.id))
        .toList();
    if (selected.isEmpty || _batchDeleting) return;
    final ok =
        await showDialog<bool>(
          context: context,
          builder: (ctx) => AlertDialog(
            backgroundColor: InkPalette.paperHi,
            title: Text(
              '删除选中的 ${selected.length} 个快照',
              style: const TextStyle(fontSize: 16),
            ),
            content: const Text(
              '快照删除后无法恢复，是否继续？',
              style: TextStyle(fontSize: 13.5, color: InkPalette.ink3),
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.of(ctx).pop(false),
                child: const Text('取消'),
              ),
              TextButton(
                onPressed: () => Navigator.of(ctx).pop(true),
                style: TextButton.styleFrom(
                  foregroundColor: InkPalette.cinnabar,
                ),
                child: const Text('删除'),
              ),
            ],
          ),
        ) ??
        false;
    if (!ok) return;
    setState(() {
      _batchDeleting = true;
      _batchProgress = 0;
    });
    var deleted = 0;
    var failed = 0;
    for (final checkpoint in selected) {
      try {
        await LocalStorage.instance.deleteCheckpoint(checkpoint.id);
        deleted++;
      } catch (_) {
        failed++;
      }
      if (mounted) setState(() => _batchProgress++);
    }
    await _load();
    if (!mounted) return;
    setState(() {
      _batchDeleting = false;
      _batchProgress = 0;
      _selectedIds.removeWhere((id) => selected.any((cp) => cp.id == id));
    });
    showSuccessSnack(
      context,
      failed == 0 ? '已删除 $deleted 个快照' : '已删除 $deleted 个，$failed 个失败',
    );
  }

  String _fmt(DateTime dt) {
    return '${dt.month}/${dt.day}  ${dt.hour.toString().padLeft(2, '0')}:'
        '${dt.minute.toString().padLeft(2, '0')}';
  }

  @override
  Widget build(BuildContext context) {
    if (_loading) {
      return const Center(
        child: CircularProgressIndicator(
          valueColor: AlwaysStoppedAnimation<Color>(InkPalette.cinnabar),
        ),
      );
    }
    if (_checkpoints.isEmpty) {
      return Center(
        child: EmptyState(
          icon: Icons.bookmark_border_rounded,
          message: '暂无快照',
          hint: '在「章节」页编辑文章时，点击相机图标可创建快照。',
        ),
      );
    }
    return Column(
      children: [
        if (_checkpoints.length > 1)
          Container(
            color: InkPalette.paperHi,
            padding: const EdgeInsets.fromLTRB(16, 7, 10, 3),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    '${_checkpoints.length} 个快照',
                    style: const TextStyle(
                      fontSize: 12,
                      color: InkPalette.ink3,
                    ),
                  ),
                ),
                TextButton(
                  onPressed: _batchDeleting
                      ? null
                      : () => setState(() {
                          _manageMode = !_manageMode;
                          _selectedIds.clear();
                        }),
                  child: Text(_manageMode ? '完成' : '批量管理'),
                ),
              ],
            ),
          ),
        if (_manageMode)
          BatchActionsBar(
            selectedCount: _selectedIds.length,
            totalCount: _checkpoints.length,
            allSelected:
                _checkpoints.isNotEmpty &&
                _checkpoints.every((cp) => _selectedIds.contains(cp.id)),
            busy: _batchDeleting,
            progress: _batchDeleting ? _batchProgress : null,
            onToggleAll: _toggleAll,
            onClear: () => setState(() => _selectedIds.clear()),
            onDelete: _deleteSelected,
          ),
        Expanded(
          child: RefreshIndicator(
            onRefresh: _load,
            color: InkPalette.cinnabar,
            child: ListView.separated(
              padding: const EdgeInsets.fromLTRB(16, 12, 16, 20),
              itemCount: _checkpoints.length,
              separatorBuilder: (_, _) => const SizedBox(height: 8),
              itemBuilder: (context, i) {
                final cp = _checkpoints[i];
                final wordCount = cp.content
                    .replaceAll(RegExp(r'\s'), '')
                    .length;
                return StaggeredEntrance(
                  index: i,
                  child: Container(
                    padding: const EdgeInsets.fromLTRB(14, 12, 8, 12),
                    decoration: BoxDecoration(
                      color: InkPalette.paperHi,
                      borderRadius: BorderRadius.circular(10),
                      border: Border.all(color: InkPalette.line, width: 0.8),
                    ),
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        if (_manageMode)
                          Checkbox(
                            value: _selectedIds.contains(cp.id),
                            onChanged: (_) => _toggleSelected(cp.id),
                            activeColor: InkPalette.cinnabar,
                            visualDensity: VisualDensity.compact,
                          ),
                        Container(
                          padding: const EdgeInsets.all(7),
                          decoration: BoxDecoration(
                            color: InkPalette.cinnabarWash,
                            borderRadius: BorderRadius.circular(8),
                          ),
                          child: const Icon(
                            Icons.bookmark_rounded,
                            size: 17,
                            color: InkPalette.cinnabar,
                          ),
                        ),
                        const SizedBox(width: 10),
                        Expanded(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Text(
                                cp.message,
                                style: const TextStyle(
                                  fontSize: 14,
                                  fontWeight: FontWeight.w600,
                                  color: InkPalette.ink,
                                ),
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                              ),
                              const SizedBox(height: 3),
                              Text(
                                '${cp.chapterTitle}  ·  $wordCount 字',
                                style: const TextStyle(
                                  fontSize: 12,
                                  color: InkPalette.ink3,
                                ),
                              ),
                              Text(
                                _fmt(cp.createdAt),
                                style: const TextStyle(
                                  fontSize: 11.5,
                                  color: InkPalette.inkGhost,
                                ),
                              ),
                            ],
                          ),
                        ),
                        Column(
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            IconButton(
                              icon: const Icon(
                                Icons.restore_rounded,
                                size: 20,
                                color: InkPalette.teal,
                              ),
                              tooltip: '回滚',
                              onPressed: () => _restore(cp),
                            ),
                            IconButton(
                              icon: const Icon(
                                Icons.delete_outline_rounded,
                                size: 19,
                                color: InkPalette.ink4,
                              ),
                              onPressed: () => _delete(cp),
                            ),
                          ],
                        ),
                      ],
                    ),
                  ),
                );
              },
            ),
          ),
        ),
      ],
    );
  }
}
