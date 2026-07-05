import 'package:flutter/material.dart';

import '../motion.dart';
import '../rpc.dart';
import '../widgets.dart';

class SettingsScreen extends StatefulWidget {
  final RpcService rpc;
  final void Function(String url) onChanged;
  const SettingsScreen({super.key, required this.rpc, required this.onChanged});

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen> {
  late final TextEditingController _url = TextEditingController(
    text: widget.rpc.baseUrl,
  );

  /// null = unknown, true = last test connected, false = last test failed.
  bool? _connected;
  bool _busy = false;

  @override
  void dispose() {
    _url.dispose();
    super.dispose();
  }

  void _save() {
    widget.onChanged(_url.text.trim());
    showSuccessSnack(context, '已保存服务器地址。');
  }

  Future<void> _testConnection() async {
    widget.onChanged(_url.text.trim());
    setState(() {
      _busy = true;
      _connected = null;
    });
    try {
      final pong = await widget.rpc.ping();
      if (!mounted) return;
      final ok = pong == 'pong';
      setState(() => _connected = ok);
      if (ok) {
        showSuccessSnack(context, '已连接 Rust 核心。');
      } else {
        showErrorSnack(context, '收到非预期响应：$pong');
      }
    } catch (e) {
      if (!mounted) return;
      setState(() => _connected = false);
      showErrorSnack(context, '连接失败：$e');
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Widget _statusChip() {
    final scheme = Theme.of(context).colorScheme;
    late final IconData icon;
    late final String text;
    late final Color color;
    // A stable key per status so the switcher cross-fades on real changes only.
    late final String stateKey;
    if (_busy) {
      icon = Icons.sync_rounded;
      text = '正在测试连接…';
      color = scheme.onSurfaceVariant;
      stateKey = 'busy';
    } else if (_connected == true) {
      icon = Icons.check_circle_rounded;
      text = '已连接 Rust 核心';
      color = const Color(0xFF4ED69A);
      stateKey = 'ok';
    } else if (_connected == false) {
      icon = Icons.error_rounded;
      text = '未连接';
      color = scheme.error;
      stateKey = 'fail';
    } else {
      icon = Icons.help_outline_rounded;
      text = '尚未测试连接';
      color = scheme.onSurfaceVariant;
      stateKey = 'unknown';
    }
    return Padding(
      padding: const EdgeInsets.only(top: 14),
      child: CrossFadeSwitcher(
        child: Row(
          key: ValueKey<String>(stateKey),
          children: [
            // Spin the sync glyph while testing; static otherwise.
            _busy
                ? _SpinningIcon(icon: icon, color: color)
                : Icon(icon, size: 18, color: color),
            const SizedBox(width: 8),
            Text(text, style: TextStyle(fontSize: 13, color: color)),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return ListView(
      padding: const EdgeInsets.all(16),
      children: [
        StaggeredEntrance(
          index: 0,
          child: sectionCard(
            '后端服务器',
            icon: Icons.dns_outlined,
            subtitle:
                '移动端通过 HTTP 连接到后端服务（桌面端 / Next 服务）。'
                '在电脑上启动前端后，把这里填成电脑的局域网地址。',
            [
              TextField(
                controller: _url,
                keyboardType: TextInputType.url,
                decoration: const InputDecoration(
                  labelText: '服务器地址',
                  hintText: 'http://192.168.1.10:3000',
                  prefixIcon: Icon(Icons.link_rounded),
                ),
              ),
              const SizedBox(height: 12),
              Row(
                children: [
                  FilledButton.icon(
                    onPressed: _busy ? null : _save,
                    icon: const Icon(Icons.save_rounded),
                    label: const Text('保存'),
                  ),
                  const SizedBox(width: 12),
                  OutlinedButton(
                    onPressed: _busy ? null : _testConnection,
                    child: BusyLabel(busy: _busy, label: '测试连接'),
                  ),
                ],
              ),
              _statusChip(),
            ],
          ),
        ),
        StaggeredEntrance(
          index: 1,
          child: sectionCard('关于', icon: Icons.info_outline_rounded, const [
            Text('Novel Generate Team —— AI 驱动的小说创作平台'),
            SizedBox(height: 6),
            Text('移动端（Flutter）复用同一个 Rust 核心层。', style: TextStyle(fontSize: 13)),
          ]),
        ),
      ],
    );
  }
}

/// A continuously rotating icon used for the "testing connection" status.
/// Stops at rest under reduced motion.
class _SpinningIcon extends StatefulWidget {
  final IconData icon;
  final Color color;

  const _SpinningIcon({required this.icon, required this.color});

  @override
  State<_SpinningIcon> createState() => _SpinningIconState();
}

class _SpinningIconState extends State<_SpinningIcon>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 1100),
  );

  @override
  void initState() {
    super.initState();
    _controller.repeat();
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final glyph = Icon(widget.icon, size: 18, color: widget.color);
    if (Motion.reduced(context)) return glyph;
    return RotationTransition(turns: _controller, child: glyph);
  }
}
