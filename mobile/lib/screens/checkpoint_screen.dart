import 'package:flutter/material.dart';

import '../motion.dart';
import '../rpc.dart';
import '../widgets.dart';

class CheckpointScreen extends StatefulWidget {
  final RpcService rpc;
  const CheckpointScreen({super.key, required this.rpc});

  @override
  State<CheckpointScreen> createState() => _CheckpointScreenState();
}

class _CheckpointScreenState extends State<CheckpointScreen> {
  final _label = TextEditingController();
  final _restoreId = TextEditingController();
  String _list = '';
  bool _listed = false;
  bool _listError = false;
  bool _busyCreate = false;
  bool _busyRefresh = false;
  bool _busyRestore = false;

  @override
  void initState() {
    super.initState();
    _restoreId.addListener(_onRestoreIdChanged);
    _refresh();
  }

  @override
  void dispose() {
    _restoreId.removeListener(_onRestoreIdChanged);
    _label.dispose();
    _restoreId.dispose();
    super.dispose();
  }

  void _onRestoreIdChanged() => setState(() {});

  Future<void> _create() async {
    setState(() => _busyCreate = true);
    try {
      final label = _label.text.trim().isEmpty ? '未命名快照' : _label.text.trim();
      final r = await widget.rpc.invokeTool('checkpoint_create', {
        'label': label,
      });
      if (!mounted) return;
      if (r['ok'] == true) {
        showSuccessSnack(context, '已创建：${r['summary'] ?? r['content']}');
      } else {
        showErrorSnack(context, (r['content'] ?? '创建失败').toString());
      }
    } catch (e) {
      if (mounted) showErrorSnack(context, '创建失败：$e');
    } finally {
      if (mounted) setState(() => _busyCreate = false);
    }
    await _refresh();
  }

  Future<void> _refresh() async {
    setState(() => _busyRefresh = true);
    try {
      final r = await widget.rpc.invokeTool('checkpoint_list', {});
      if (!mounted) return;
      setState(() {
        _list = (r['content'] ?? '').toString();
        _listed = true;
        _listError = false;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _listed = true;
        _listError = true;
      });
      showErrorSnack(context, '获取快照列表失败：$e');
    } finally {
      if (mounted) setState(() => _busyRefresh = false);
    }
  }

  Future<void> _restore() async {
    final id = _restoreId.text.trim();
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('确认回滚'),
        content: Text('回滚会把工作区恢复到快照 $id 的状态，当前未保存的改动可能丢失。确定继续吗？'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx, false),
            child: const Text('取消'),
          ),
          FilledButton(
            style: FilledButton.styleFrom(
              backgroundColor: Theme.of(ctx).colorScheme.error,
            ),
            onPressed: () => Navigator.pop(ctx, true),
            child: const Text('回滚'),
          ),
        ],
      ),
    );
    if (confirmed != true || !mounted) return;

    setState(() => _busyRestore = true);
    try {
      final r = await widget.rpc.invokeTool('checkpoint_restore', {'id': id});
      if (!mounted) return;
      if (r['ok'] == true) {
        showSuccessSnack(context, '已回滚到 $id');
      } else {
        showErrorSnack(context, (r['content'] ?? '回滚失败').toString());
      }
    } catch (e) {
      if (mounted) showErrorSnack(context, '回滚失败：$e');
    } finally {
      if (mounted) setState(() => _busyRestore = false);
    }
  }

  bool get _anyBusy => _busyCreate || _busyRefresh || _busyRestore;

  @override
  Widget build(BuildContext context) {
    return RefreshIndicator(
      onRefresh: _refresh,
      child: ListView(
        padding: const EdgeInsets.all(16),
        physics: const AlwaysScrollableScrollPhysics(),
        children: [
          StaggeredEntrance(
            index: 0,
            child: sectionCard('创建快照', icon: Icons.add_a_photo_outlined, [
              TextField(
                controller: _label,
                decoration: const InputDecoration(
                  labelText: '快照标签',
                  prefixIcon: Icon(Icons.bookmark_add_outlined),
                ),
              ),
              const SizedBox(height: 12),
              Row(
                children: [
                  FilledButton(
                    onPressed: _anyBusy ? null : _create,
                    child: BusyLabel(busy: _busyCreate, label: '创建快照'),
                  ),
                  const SizedBox(width: 12),
                  OutlinedButton(
                    onPressed: _anyBusy ? null : _refresh,
                    child: BusyLabel(busy: _busyRefresh, label: '刷新列表'),
                  ),
                ],
              ),
              if (_listed && _listError)
                const Padding(
                  padding: EdgeInsets.only(top: 14),
                  child: EmptyState(
                    icon: Icons.error_outline_rounded,
                    message: '获取快照列表失败',
                    hint: '检查后端连接后下拉刷新重试。',
                  ),
                )
              else if (_listed && _list.isEmpty)
                const Padding(
                  padding: EdgeInsets.only(top: 14),
                  child: EmptyState(
                    icon: Icons.history_toggle_off_rounded,
                    message: '还没有任何快照',
                    hint: '填写标签后点“创建快照”保存当前进度。',
                  ),
                )
              else if (_list.isNotEmpty)
                outputBox(_list),
            ]),
          ),
          StaggeredEntrance(
            index: 1,
            child: sectionCard(
              '回滚到快照',
              icon: Icons.settings_backup_restore_rounded,
              [
                TextField(
                  controller: _restoreId,
                  decoration: const InputDecoration(
                    labelText: 'checkpoint id，例如 ckpt_...',
                    prefixIcon: Icon(Icons.tag_rounded),
                  ),
                ),
                const SizedBox(height: 12),
                FilledButton.icon(
                  style: FilledButton.styleFrom(
                    backgroundColor: Theme.of(context).colorScheme.error,
                    foregroundColor: Theme.of(context).colorScheme.onError,
                  ),
                  onPressed: _anyBusy || _restoreId.text.trim().isEmpty
                      ? null
                      : _restore,
                  icon: BusyIcon(
                    busy: _busyRestore,
                    icon: Icons.restore_rounded,
                  ),
                  label: const Text('回滚'),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
