import 'package:flutter/material.dart';

import '../motion.dart';
import '../rpc.dart';
import '../widgets.dart';

class ToolsScreen extends StatefulWidget {
  final RpcService rpc;
  const ToolsScreen({super.key, required this.rpc});

  @override
  State<ToolsScreen> createState() => _ToolsScreenState();
}

class _ToolsScreenState extends State<ToolsScreen> {
  List<dynamic> _tools = [];
  String? _error;
  bool _loading = true;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final tools = await widget.rpc.listTools();
      if (!mounted) return;
      setState(() {
        _tools = tools;
        _loading = false;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _error = '$e';
        _loading = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    // Cross-fade between loading / error / empty / list so state changes glide.
    return CrossFadeSwitcher(slideY: 0, child: _buildBody(context));
  }

  Widget _buildBody(BuildContext context) {
    if (_loading) {
      return ListView(
        key: const ValueKey('loading'),
        padding: const EdgeInsets.all(16),
        physics: const NeverScrollableScrollPhysics(),
        children: [
          for (int i = 0; i < 5; i++)
            const Padding(
              padding: EdgeInsets.only(bottom: 12),
              child: _ToolSkeletonCard(),
            ),
        ],
      );
    }
    if (_error != null) {
      return KeyedSubtree(
        key: const ValueKey('error'),
        child: ErrorState(message: '连接核心失败：$_error', onRetry: _load),
      );
    }
    return RefreshIndicator(
      key: const ValueKey('list'),
      onRefresh: _load,
      child: _tools.isEmpty
          ? ListView(
              physics: const AlwaysScrollableScrollPhysics(),
              children: const [
                SizedBox(height: 60),
                EmptyState(
                  icon: Icons.handyman_outlined,
                  message: '核心没有返回任何工具',
                  hint: '下拉刷新重试。',
                ),
              ],
            )
          : ListView.builder(
              padding: const EdgeInsets.all(16),
              physics: const AlwaysScrollableScrollPhysics(),
              itemCount: _tools.length,
              itemBuilder: (context, i) {
                final t = _tools[i] as Map<String, dynamic>;
                return StaggeredEntrance(
                  index: i,
                  child: _ToolCard(tool: t),
                );
              },
            ),
    );
  }
}

/// A single tool entry. Lifts slightly and shows a ripple on press so the
/// catalogue feels tappable even though it is informational.
class _ToolCard extends StatefulWidget {
  final Map<String, dynamic> tool;

  const _ToolCard({required this.tool});

  @override
  State<_ToolCard> createState() => _ToolCardState();
}

class _ToolCardState extends State<_ToolCard> {
  bool _pressed = false;

  @override
  Widget build(BuildContext context) {
    final t = widget.tool;
    final caps = (t['capabilities'] as List?)?.join('、') ?? '';
    final mutating = t['mutating'] == true;
    final scheme = Theme.of(context).colorScheme;
    final reduced = Motion.reduced(context);
    final lift = (!reduced && _pressed) ? -2.0 : 0.0;

    return AnimatedContainer(
      duration: Motion.fast,
      curve: Motion.standard,
      margin: const EdgeInsets.only(bottom: 12),
      transform: Matrix4.translationValues(0, lift, 0),
      child: Card(
        margin: EdgeInsets.zero,
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          borderRadius: BorderRadius.circular(16),
          onTap: () {},
          onHighlightChanged: (v) {
            if (_pressed != v) setState(() => _pressed = v);
          },
          child: Padding(
            padding: const EdgeInsets.fromLTRB(16, 14, 16, 14),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Flexible(
                      child: Text(
                        t['name']?.toString() ?? '',
                        style: const TextStyle(
                          fontFamily: 'monospace',
                          fontSize: 14,
                          fontWeight: FontWeight.w700,
                          color: Color(0xFF5BE3B3),
                        ),
                      ),
                    ),
                    const Spacer(),
                    if (mutating)
                      Chip(
                        label: const Text('写'),
                        labelStyle: const TextStyle(fontSize: 11),
                        visualDensity: VisualDensity.compact,
                        materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
                        backgroundColor: scheme.errorContainer,
                        side: BorderSide.none,
                      )
                    else
                      Chip(
                        label: const Text('只读'),
                        labelStyle: const TextStyle(fontSize: 11),
                        visualDensity: VisualDensity.compact,
                        materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
                        backgroundColor: scheme.surfaceContainerHighest,
                        side: BorderSide.none,
                      ),
                  ],
                ),
                if ((t['description']?.toString() ?? '').isNotEmpty) ...[
                  const SizedBox(height: 6),
                  Text(
                    t['description'].toString(),
                    style: const TextStyle(fontSize: 13, height: 1.45),
                  ),
                ],
                if (caps.isNotEmpty) ...[
                  const SizedBox(height: 8),
                  Row(
                    children: [
                      Icon(
                        Icons.shield_outlined,
                        size: 14,
                        color: scheme.onSurfaceVariant,
                      ),
                      const SizedBox(width: 6),
                      Expanded(
                        child: Text(
                          caps,
                          style: TextStyle(
                            fontSize: 12,
                            color: scheme.onSurfaceVariant,
                          ),
                        ),
                      ),
                    ],
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// Skeleton placeholder shown while the tool catalogue loads.
class _ToolSkeletonCard extends StatelessWidget {
  const _ToolSkeletonCard();

  @override
  Widget build(BuildContext context) {
    return Card(
      margin: EdgeInsets.zero,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 16, 16, 16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: const [
            Row(
              children: [
                ShimmerBox(width: 120, height: 14),
                Spacer(),
                ShimmerBox(width: 36, height: 18),
              ],
            ),
            SizedBox(height: 12),
            ShimmerLines(lines: 2),
          ],
        ),
      ),
    );
  }
}
