import 'package:flutter/material.dart';

import '../motion.dart';
import '../rpc.dart';
import '../widgets.dart';

class FilesScreen extends StatefulWidget {
  final RpcService rpc;
  const FilesScreen({super.key, required this.rpc});

  @override
  State<FilesScreen> createState() => _FilesScreenState();
}

class _FilesScreenState extends State<FilesScreen> {
  final _dir = TextEditingController();
  final _path = TextEditingController(text: 'book/ch1.md');
  final _content = TextEditingController();
  String _listing = '';
  bool _listed = false;
  bool _listError = false;
  bool _busyList = false;
  bool _busyOpen = false;
  bool _busySave = false;

  @override
  void dispose() {
    _dir.dispose();
    _path.dispose();
    _content.dispose();
    super.dispose();
  }

  Future<void> _list() async {
    setState(() => _busyList = true);
    try {
      final r = await widget.rpc.invokeTool('list_dir', {'path': _dir.text});
      if (!mounted) return;
      setState(() {
        _listing = (r['content'] ?? '(空目录)').toString();
        _listed = true;
        _listError = false;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _listed = true;
        _listError = true;
      });
      showErrorSnack(context, '列目录失败：$e');
    } finally {
      if (mounted) setState(() => _busyList = false);
    }
  }

  Future<void> _open() async {
    setState(() => _busyOpen = true);
    try {
      final r = await widget.rpc.invokeTool('read_file', {'path': _path.text});
      if (!mounted) return;
      if (r['ok'] == true) {
        setState(() => _content.text = (r['content'] ?? '').toString());
        showSuccessSnack(context, '已打开 ${_path.text}');
      } else {
        showErrorSnack(context, (r['content'] ?? '读取失败').toString());
      }
    } catch (e) {
      if (mounted) showErrorSnack(context, '读取失败：$e');
    } finally {
      if (mounted) setState(() => _busyOpen = false);
    }
  }

  Future<void> _save() async {
    setState(() => _busySave = true);
    try {
      final r = await widget.rpc.invokeTool('write_file', {
        'path': _path.text,
        'content': _content.text,
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

  bool get _anyBusy => _busyList || _busyOpen || _busySave;

  @override
  Widget build(BuildContext context) {
    return RefreshIndicator(
      onRefresh: _list,
      child: ListView(
        padding: const EdgeInsets.all(16),
        physics: const AlwaysScrollableScrollPhysics(),
        children: [
          StaggeredEntrance(
            index: 0,
            child: sectionCard('目录浏览', icon: Icons.folder_open_rounded, [
              TextField(
                controller: _dir,
                decoration: const InputDecoration(
                  labelText: '目录路径（留空 = 工作区根目录）',
                  prefixIcon: Icon(Icons.subdirectory_arrow_right_rounded),
                ),
                onSubmitted: (_) => _anyBusy ? null : _list(),
              ),
              const SizedBox(height: 12),
              FilledButton.icon(
                onPressed: _anyBusy ? null : _list,
                icon: BusyIcon(busy: _busyList, icon: Icons.list_rounded),
                label: const Text('列目录'),
              ),
              if (_listed && _listError)
                const Padding(
                  padding: EdgeInsets.only(top: 14),
                  child: EmptyState(
                    icon: Icons.error_outline_rounded,
                    message: '列目录失败',
                    hint: '检查目录路径和后端连接后下拉刷新重试。',
                  ),
                )
              else if (_listed && _listing.isEmpty)
                const Padding(
                  padding: EdgeInsets.only(top: 14),
                  child: EmptyState(
                    icon: Icons.folder_off_rounded,
                    message: '这个目录是空的',
                  ),
                )
              else if (_listing.isNotEmpty)
                outputBox(_listing),
            ]),
          ),
          StaggeredEntrance(
            index: 1,
            child: sectionCard('编辑文件', icon: Icons.edit_note_rounded, [
              TextField(
                controller: _path,
                decoration: const InputDecoration(
                  labelText: '文件路径，例如 book/ch1.md',
                  prefixIcon: Icon(Icons.description_outlined),
                ),
              ),
              const SizedBox(height: 12),
              TextField(
                controller: _content,
                maxLines: 12,
                minLines: 6,
                decoration: const InputDecoration(
                  labelText: '文件内容',
                  alignLabelWithHint: true,
                  border: OutlineInputBorder(),
                ),
              ),
              const SizedBox(height: 12),
              Row(
                children: [
                  OutlinedButton(
                    onPressed: _anyBusy ? null : _open,
                    child: BusyLabel(busy: _busyOpen, label: '打开'),
                  ),
                  const SizedBox(width: 12),
                  FilledButton(
                    onPressed: _anyBusy ? null : _save,
                    child: BusyLabel(busy: _busySave, label: '保存'),
                  ),
                ],
              ),
            ]),
          ),
        ],
      ),
    );
  }
}
