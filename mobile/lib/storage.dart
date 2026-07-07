// storage.dart — 本地存储层：章节、记忆、快照，全部存为 JSON 文件
// 不依赖后端，所有数据存在设备本地 (path_provider)。

import 'dart:convert';
import 'dart:io';
import 'package:path_provider/path_provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'ai_client.dart';

// ── 数据模型 ────────────────────────────────────────────────────────

class Chapter {
  final String id;
  String title;
  String content;
  final DateTime createdAt;
  DateTime updatedAt;

  Chapter({
    required this.id,
    required this.title,
    required this.content,
    required this.createdAt,
    required this.updatedAt,
  });

  Map<String, dynamic> toJson() => {
    'id': id, 'title': title, 'content': content,
    'createdAt': createdAt.toIso8601String(),
    'updatedAt': updatedAt.toIso8601String(),
  };

  factory Chapter.fromJson(Map<String, dynamic> j) => Chapter(
    id: j['id'] as String,
    title: j['title'] as String? ?? '未命名章节',
    content: j['content'] as String? ?? '',
    createdAt: DateTime.parse(j['createdAt'] as String),
    updatedAt: DateTime.parse(j['updatedAt'] as String),
  );

  factory Chapter.create(String title) {
    final now = DateTime.now();
    return Chapter(
      id: 'ch_${now.millisecondsSinceEpoch}',
      title: title, content: '',
      createdAt: now, updatedAt: now,
    );
  }
}

class Memory {
  final String id;
  String kind; // character / worldbuilding / plot / foreshadow / lore / other
  String title;
  String content;
  final DateTime createdAt;

  Memory({
    required this.id, required this.kind,
    required this.title, required this.content,
    required this.createdAt,
  });

  Map<String, dynamic> toJson() => {
    'id': id, 'kind': kind, 'title': title, 'content': content,
    'createdAt': createdAt.toIso8601String(),
  };

  factory Memory.fromJson(Map<String, dynamic> j) => Memory(
    id: j['id'] as String,
    kind: j['kind'] as String? ?? 'other',
    title: j['title'] as String? ?? '',
    content: j['content'] as String? ?? '',
    createdAt: DateTime.parse(j['createdAt'] as String),
  );

  factory Memory.create({required String kind, required String title,
      required String content}) {
    return Memory(
      id: 'mem_${DateTime.now().millisecondsSinceEpoch}',
      kind: kind, title: title, content: content,
      createdAt: DateTime.now(),
    );
  }
}

class Checkpoint {
  final String id;
  final String chapterId;
  final String chapterTitle;
  final String content;
  String message;
  final DateTime createdAt;

  Checkpoint({
    required this.id, required this.chapterId,
    required this.chapterTitle, required this.content,
    required this.message, required this.createdAt,
  });

  Map<String, dynamic> toJson() => {
    'id': id, 'chapterId': chapterId,
    'chapterTitle': chapterTitle, 'content': content,
    'message': message, 'createdAt': createdAt.toIso8601String(),
  };

  factory Checkpoint.fromJson(Map<String, dynamic> j) => Checkpoint(
    id: j['id'] as String, chapterId: j['chapterId'] as String? ?? '',
    chapterTitle: j['chapterTitle'] as String? ?? '',
    content: j['content'] as String? ?? '',
    message: j['message'] as String? ?? '',
    createdAt: DateTime.parse(j['createdAt'] as String),
  );
}

// ── 本地存储 ────────────────────────────────────────────────────────

class LocalStorage {
  static LocalStorage? _instance;
  static LocalStorage get instance => _instance ??= LocalStorage._();
  LocalStorage._();

  Future<Directory> get _dir async {
    final base = await getApplicationDocumentsDirectory();
    final d = Directory('${base.path}/novel_agent');
    if (!d.existsSync()) d.createSync(recursive: true);
    return d;
  }

  File _file(Directory dir, String name) => File('${dir.path}/$name');

  // ── 通用 JSON 读写 ────────────────────────────────────────────────

  Future<List<Map<String, dynamic>>> _readList(String filename) async {
    final dir = await _dir;
    final f = _file(dir, filename);
    if (!f.existsSync()) return [];
    try {
      return (jsonDecode(f.readAsStringSync()) as List)
          .cast<Map<String, dynamic>>();
    } catch (_) { return []; }
  }

  Future<void> _writeList(String filename, List<Map<String, dynamic>> data) async {
    final dir = await _dir;
    _file(dir, filename).writeAsStringSync(jsonEncode(data));
  }

  // ── 章节 CRUD ─────────────────────────────────────────────────────

  Future<List<Chapter>> listChapters() async {
    final raw = await _readList('chapters.json');
    return raw.map(Chapter.fromJson).toList()
      ..sort((a, b) => b.updatedAt.compareTo(a.updatedAt));
  }

  Future<void> saveChapter(Chapter ch) async {
    final all = await listChapters();
    final idx = all.indexWhere((c) => c.id == ch.id);
    ch.updatedAt = DateTime.now();
    if (idx >= 0) all[idx] = ch; else all.insert(0, ch);
    await _writeList('chapters.json', all.map((c) => c.toJson()).toList());
  }

  Future<void> deleteChapter(String id) async {
    final all = await listChapters();
    all.removeWhere((c) => c.id == id);
    await _writeList('chapters.json', all.map((c) => c.toJson()).toList());
    // 同时删除该章节的所有快照
    final cps = await listCheckpoints();
    final remaining = cps.where((c) => c.chapterId != id).toList();
    await _writeList('checkpoints.json', remaining.map((c) => c.toJson()).toList());
  }

  // ── 记忆 CRUD ─────────────────────────────────────────────────────

  Future<List<Memory>> listMemories() async {
    final raw = await _readList('memories.json');
    return raw.map(Memory.fromJson).toList()
      ..sort((a, b) => b.createdAt.compareTo(a.createdAt));
  }

  Future<void> saveMemory(Memory m) async {
    final all = await listMemories();
    final idx = all.indexWhere((x) => x.id == m.id);
    if (idx >= 0) all[idx] = m; else all.insert(0, m);
    await _writeList('memories.json', all.map((x) => x.toJson()).toList());
  }

  Future<void> deleteMemory(String id) async {
    final all = await listMemories();
    all.removeWhere((m) => m.id == id);
    await _writeList('memories.json', all.map((m) => m.toJson()).toList());
  }

  // ── 快照 CRUD ─────────────────────────────────────────────────────

  Future<List<Checkpoint>> listCheckpoints() async {
    final raw = await _readList('checkpoints.json');
    return raw.map(Checkpoint.fromJson).toList()
      ..sort((a, b) => b.createdAt.compareTo(a.createdAt));
  }

  Future<Checkpoint> createCheckpoint(Chapter ch, String message) async {
    final cp = Checkpoint(
      id: 'cp_${DateTime.now().millisecondsSinceEpoch}',
      chapterId: ch.id, chapterTitle: ch.title,
      content: ch.content, message: message,
      createdAt: DateTime.now(),
    );
    final all = await listCheckpoints();
    all.insert(0, cp);
    await _writeList('checkpoints.json', all.map((c) => c.toJson()).toList());
    return cp;
  }

  Future<void> restoreCheckpoint(Checkpoint cp) async {
    final chapters = await listChapters();
    final idx = chapters.indexWhere((c) => c.id == cp.chapterId);
    if (idx < 0) return;
    chapters[idx]
      ..content = cp.content
      ..updatedAt = DateTime.now();
    await _writeList('chapters.json', chapters.map((c) => c.toJson()).toList());
  }

  Future<void> deleteCheckpoint(String id) async {
    final all = await listCheckpoints();
    all.removeWhere((c) => c.id == id);
    await _writeList('checkpoints.json', all.map((c) => c.toJson()).toList());
  }

  // ── AI 供应商设置 ─────────────────────────────────────────────────

  Future<AiProvider?> loadProvider() async {
    final prefs = await SharedPreferences.getInstance();
    final json = prefs.getString('ai_provider');
    if (json == null) return null;
    try {
      return AiProvider.fromJson(jsonDecode(json) as Map<String, dynamic>);
    } catch (_) { return null; }
  }

  Future<void> saveProvider(AiProvider p) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString('ai_provider', jsonEncode(p.toJson()));
  }
}
