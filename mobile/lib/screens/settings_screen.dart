// settings_screen.dart — API 供应商配置（独立版，不需要后端）
import 'package:flutter/material.dart';
import '../main.dart' show InkPalette;
import '../motion.dart';
import '../ai_client.dart';
import '../storage.dart';
import '../widgets.dart';

class SettingsScreen extends StatefulWidget {
  final AiProvider? provider;
  final void Function(AiProvider) onProviderChanged;
  const SettingsScreen({
    super.key,
    required this.provider,
    required this.onProviderChanged,
  });

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen> {
  late AiProvider _draft;
  bool _showKey = false;
  bool _testing = false;
  bool? _testOk;
  String _testMsg = '';

  @override
  void initState() {
    super.initState();
    _draft = widget.provider ?? kProviderPresets.first;
  }

  void _applyPreset(AiProvider preset) {
    setState(() {
      _draft = AiProvider(
        name: preset.name,
        protocol: preset.protocol,
        baseUrl: preset.baseUrl,
        apiKey: _draft.apiKey, // 保留已填的 key
        model: preset.model,
      );
      _testOk = null;
    });
  }

  Future<void> _testConnection() async {
    if (_draft.apiKey.isEmpty) {
      setState(() { _testOk = false; _testMsg = '请先填写 API Key'; });
      return;
    }
    setState(() { _testing = true; _testOk = null; });
    try {
      final reply = await AiClient(_draft).testConnection();
      if (!mounted) return;
      setState(() { _testOk = true; _testMsg = '连接成功，模型回复：$reply'; });
    } catch (e) {
      if (!mounted) return;
      setState(() { _testOk = false; _testMsg = '连接失败：$e'; });
    } finally {
      if (mounted) setState(() => _testing = false);
    }
  }

  Future<void> _save() async {
    await LocalStorage.instance.saveProvider(_draft);
    widget.onProviderChanged(_draft);
    if (!mounted) return;
    showSuccessSnack(context, '已保存，模型切换为 ${_draft.model}');
  }

  @override
  Widget build(BuildContext context) {
    return ListView(
      padding: const EdgeInsets.all(16),
      children: [
        // ── 预设快填 ──
        StaggeredEntrance(
          index: 0,
          child: sectionCard('快速填充预设', icon: Icons.bolt_outlined, [
            Text('选择一个服务商预设后，只需填写你自己的 API Key 即可。',
              style: TextStyle(fontSize: 12.5, color: InkPalette.ink3, height: 1.5)),
            const SizedBox(height: 12),
            SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Row(
                children: kProviderPresets.map((p) {
                  final active = _draft.baseUrl == p.baseUrl;
                  return Padding(
                    padding: const EdgeInsets.only(right: 8),
                    child: GestureDetector(
                      onTap: () => _applyPreset(p),
                      child: AnimatedContainer(
                        duration: Motion.fast,
                        padding: const EdgeInsets.symmetric(
                          horizontal: 12, vertical: 6),
                        decoration: BoxDecoration(
                          color: active
                              ? InkPalette.cinnabarWash
                              : InkPalette.paperLo,
                          borderRadius: BorderRadius.circular(20),
                          border: Border.all(
                            color: active ? InkPalette.cinnabar : InkPalette.line,
                            width: active ? 1.2 : 0.8,
                          ),
                        ),
                        child: Text(p.name,
                          style: TextStyle(
                            fontSize: 12.5,
                            fontWeight: active
                                ? FontWeight.w600 : FontWeight.normal,
                            color: active
                                ? InkPalette.cinnabar : InkPalette.ink2,
                          ),
                        ),
                      ),
                    ),
                  );
                }).toList(),
              ),
            ),
          ]),
        ),

        // ── API Key ──
        StaggeredEntrance(
          index: 1,
          child: sectionCard('API Key', icon: Icons.key_rounded,
            subtitle: 'Key 只存在本设备，不经过任何服务器。',
            [
              TextField(
                obscureText: !_showKey,
                onChanged: (v) => setState(() {
                  _draft = AiProvider(
                    name: _draft.name, protocol: _draft.protocol,
                    baseUrl: _draft.baseUrl, apiKey: v, model: _draft.model,
                  );
                }),
                controller: TextEditingController.fromValue(
                  TextEditingValue(
                    text: _draft.apiKey,
                    selection: TextSelection.collapsed(offset: _draft.apiKey.length),
                  ),
                ),
                decoration: InputDecoration(
                  labelText: 'API Key',
                  hintText: 'sk-...',
                  suffixIcon: IconButton(
                    icon: Icon(_showKey
                        ? Icons.visibility_off_rounded
                        : Icons.visibility_rounded,
                      size: 20),
                    onPressed: () => setState(() => _showKey = !_showKey),
                  ),
                ),
              ),
            ]),
        ),

        // ── 接入地址 & 模型 ──
        StaggeredEntrance(
          index: 2,
          child: sectionCard('接口设置', icon: Icons.settings_ethernet_rounded, [
            TextField(
              onChanged: (v) => setState(() {
                _draft = AiProvider(
                  name: _draft.name, protocol: _draft.protocol,
                  baseUrl: v.trim(), apiKey: _draft.apiKey, model: _draft.model,
                );
              }),
              controller: TextEditingController.fromValue(
                TextEditingValue(
                  text: _draft.baseUrl,
                  selection: TextSelection.collapsed(offset: _draft.baseUrl.length),
                ),
              ),
              keyboardType: TextInputType.url,
              decoration: const InputDecoration(
                labelText: '接口地址 (Base URL)',
                hintText: 'https://api.deepseek.com/v1',
              ),
            ),
            const SizedBox(height: 12),
            TextField(
              onChanged: (v) => setState(() {
                _draft = AiProvider(
                  name: _draft.name, protocol: _draft.protocol,
                  baseUrl: _draft.baseUrl, apiKey: _draft.apiKey,
                  model: v.trim(),
                );
              }),
              controller: TextEditingController.fromValue(
                TextEditingValue(
                  text: _draft.model,
                  selection: TextSelection.collapsed(offset: _draft.model.length),
                ),
              ),
              decoration: const InputDecoration(
                labelText: '模型名称',
                hintText: 'deepseek-chat',
              ),
            ),
          ]),
        ),

        // ── 操作按钮 ──
        StaggeredEntrance(
          index: 3,
          child: sectionCard('', [
            Row(
              children: [
                Expanded(
                  child: FilledButton.icon(
                    onPressed: _save,
                    icon: const Icon(Icons.save_rounded, size: 18),
                    label: const Text('保存配置'),
                  ),
                ),
                const SizedBox(width: 10),
                OutlinedButton(
                  onPressed: _testing ? null : _testConnection,
                  child: BusyLabel(busy: _testing, label: '测试连接'),
                ),
              ],
            ),
            if (_testOk != null) ...[
              const SizedBox(height: 10),
              Container(
                padding: const EdgeInsets.all(10),
                decoration: BoxDecoration(
                  color: _testOk!
                      ? const Color(0xFFE8F5EE) : InkPalette.cinnabarWash,
                  borderRadius: BorderRadius.circular(8),
                  border: Border.all(
                    color: _testOk!
                        ? const Color(0xFF4CAF50) : InkPalette.cinnabar,
                    width: 0.8),
                ),
                child: Row(
                  children: [
                    Icon(
                      _testOk!
                          ? Icons.check_circle_rounded : Icons.error_rounded,
                      size: 16,
                      color: _testOk!
                          ? const Color(0xFF2E7D32) : InkPalette.cinnabar,
                    ),
                    const SizedBox(width: 8),
                    Expanded(child: Text(_testMsg,
                      style: TextStyle(
                        fontSize: 12.5,
                        color: _testOk!
                            ? const Color(0xFF2E7D32) : InkPalette.cinnabar,
                      ))),
                  ],
                ),
              ),
            ],
          ]),
        ),

        // ── 关于 ──
        StaggeredEntrance(
          index: 4,
          child: sectionCard('关于', icon: Icons.info_outline_rounded, const [
            Text('墨·创作 — AI 驱动的小说创作平台',
              style: TextStyle(fontSize: 13.5, color: InkPalette.ink)),
            SizedBox(height: 4),
            Text('移动端（Flutter）· 完全本地运行，无需后端。\n'
                 '支持 OpenAI / DeepSeek / Kimi / 智谱 / Anthropic / Ollama。',
              style: TextStyle(fontSize: 12.5, color: InkPalette.ink3, height: 1.5)),
            SizedBox(height: 8),
            Text('v0.3.0', style: TextStyle(fontSize: 12, color: InkPalette.inkGhost)),
          ]),
        ),
      ],
    );
  }
}
