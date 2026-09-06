// storage.dart — 本地存储层：章节、记忆、快照，全部存为 JSON 文件
// 不依赖后端，所有数据存在设备本地 (path_provider)。

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:path_provider/path_provider.dart';
import 'ai_client.dart';
import 'provider_config_store.dart';

class StorageCorruptionException implements Exception {
  final String filename;
  final Object cause;

  const StorageCorruptionException(this.filename, this.cause);

  @override
  String toString() => '本地数据文件 $filename 已损坏: $cause';
}

class _AsyncSerial {
  Future<void> _tail = Future<void>.value();

  Future<T> run<T>(Future<T> Function() operation) async {
    final previous = _tail;
    final release = Completer<void>();
    _tail = release.future;

    await previous;
    try {
      return await operation();
    } finally {
      release.complete();
    }
  }
}

abstract interface class JsonListPersistence {
  Future<List<Map<String, dynamic>>> readList(String filename);

  Future<void> writeList(String filename, List<Map<String, dynamic>> data);
}

int _lastGeneratedId = 0;

String _nextLocalId(String prefix) {
  final timestamp = DateTime.now().microsecondsSinceEpoch;
  _lastGeneratedId = timestamp > _lastGeneratedId
      ? timestamp
      : _lastGeneratedId + 1;
  return '${prefix}_$_lastGeneratedId';
}

/// JSON list persistence with a validated temp file and last-known-good backup.
class JsonListStore implements JsonListPersistence {
  final Directory directory;
  final _serial = _AsyncSerial();

  JsonListStore(this.directory);

  File _file(String name) =>
      File('${directory.path}${Platform.pathSeparator}$name');

  List<Map<String, dynamic>> _decode(String filename, String content) {
    try {
      final decoded = jsonDecode(content);
      if (decoded is! List) throw const FormatException('根节点必须是数组');
      return decoded
          .map((item) {
            if (item is! Map) throw const FormatException('数组元素必须是对象');
            return Map<String, dynamic>.from(item);
          })
          .toList(growable: false);
    } catch (error) {
      throw StorageCorruptionException(filename, error);
    }
  }

  Future<List<Map<String, dynamic>>> _readFile(
    String filename,
    File file,
  ) async {
    return _decode(filename, await file.readAsString());
  }

  Future<void> _restoreBackup(File target, File backup) async {
    final recovery = _file('${target.uri.pathSegments.last}.recovering');
    if (await recovery.exists()) await recovery.delete();
    await backup.copy(recovery.path);
    await recovery.open(mode: FileMode.append).then((handle) async {
      await handle.flush();
      await handle.close();
    });
    if (await target.exists()) await target.delete();
    await recovery.rename(target.path);
  }

  @override
  Future<List<Map<String, dynamic>>> readList(String filename) {
    return _serial.run(() => _readList(filename));
  }

  Future<List<Map<String, dynamic>>> _readList(String filename) async {
    await directory.create(recursive: true);
    final target = _file(filename);
    final backup = _file('$filename.bak');

    if (!await target.exists()) {
      if (!await backup.exists()) return [];
      final recovered = await _readFile(filename, backup);
      await _restoreBackup(target, backup);
      return recovered;
    }

    try {
      return await _readFile(filename, target);
    } on StorageCorruptionException catch (primaryError) {
      if (!await backup.exists()) rethrow;
      try {
        final recovered = await _readFile(filename, backup);
        await _restoreBackup(target, backup);
        return recovered;
      } on StorageCorruptionException catch (backupError) {
        throw StorageCorruptionException(
          filename,
          '主文件与备份均不可读取（$primaryError；$backupError）',
        );
      }
    }
  }

  @override
  Future<void> writeList(String filename, List<Map<String, dynamic>> data) {
    return _serial.run(() => _writeList(filename, data));
  }

  Future<void> _writeList(
    String filename,
    List<Map<String, dynamic>> data,
  ) async {
    await directory.create(recursive: true);
    final target = _file(filename);
    final backup = _file('$filename.bak');
    final temporary = _file('$filename.tmp');

    // Never rotate a corrupt primary over the last-known-good backup.
    if (await target.exists()) await _readList(filename);

    if (await temporary.exists()) await temporary.delete();
    await temporary.writeAsString(jsonEncode(data), flush: true);
    await _readFile(filename, temporary);

    if (await backup.exists()) await backup.delete();
    if (await target.exists()) await target.rename(backup.path);
    try {
      await temporary.rename(target.path);
    } catch (_) {
      if (!await target.exists() && await backup.exists()) {
        await backup.copy(target.path);
      }
      rethrow;
    }
  }
}

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
    'id': id,
    'title': title,
    'content': content,
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
      id: _nextLocalId('ch'),
      title: title,
      content: '',
      createdAt: now,
      updatedAt: now,
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
    required this.id,
    required this.kind,
    required this.title,
    required this.content,
    required this.createdAt,
  });

  Map<String, dynamic> toJson() => {
    'id': id,
    'kind': kind,
    'title': title,
    'content': content,
    'createdAt': createdAt.toIso8601String(),
  };

  factory Memory.fromJson(Map<String, dynamic> j) => Memory(
    id: j['id'] as String,
    kind: j['kind'] as String? ?? 'other',
    title: j['title'] as String? ?? '',
    content: j['content'] as String? ?? '',
    createdAt: DateTime.parse(j['createdAt'] as String),
  );

  factory Memory.create({
    required String kind,
    required String title,
    required String content,
  }) {
    return Memory(
      id: _nextLocalId('mem'),
      kind: kind,
      title: title,
      content: content,
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
    required this.id,
    required this.chapterId,
    required this.chapterTitle,
    required this.content,
    required this.message,
    required this.createdAt,
  });

  Map<String, dynamic> toJson() => {
    'id': id,
    'chapterId': chapterId,
    'chapterTitle': chapterTitle,
    'content': content,
    'message': message,
    'createdAt': createdAt.toIso8601String(),
  };

  factory Checkpoint.fromJson(Map<String, dynamic> j) => Checkpoint(
    id: j['id'] as String,
    chapterId: j['chapterId'] as String? ?? '',
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

  LocalStorage._({
    ProviderConfigStore? providerConfig,
    JsonListPersistence? jsonStore,
  }) : _providerConfig = providerConfig ?? ProviderConfigStore.platform(),
       _storeFuture = jsonStore == null
           ? null
           : Future<JsonListPersistence>.value(jsonStore);

  factory LocalStorage.forTesting({
    required JsonListPersistence jsonStore,
    ProviderConfigStore? providerConfig,
  }) => LocalStorage._(providerConfig: providerConfig, jsonStore: jsonStore);

  final ProviderConfigStore _providerConfig;
  final _transactions = _AsyncSerial();
  Future<JsonListPersistence>? _storeFuture;

  Future<Directory> get _dir async {
    final base = await getApplicationDocumentsDirectory();
    final d = Directory('${base.path}/novel_agent');
    if (!await d.exists()) await d.create(recursive: true);
    return d;
  }

  Future<JsonListPersistence> _createStore() async => JsonListStore(await _dir);

  Future<JsonListPersistence> get _store => _storeFuture ??= _createStore();

  // ── 通用 JSON 读写 ────────────────────────────────────────────────

  Future<List<Map<String, dynamic>>> _readList(String filename) async {
    return (await _store).readList(filename);
  }

  Future<void> _writeList(
    String filename,
    List<Map<String, dynamic>> data,
  ) async {
    await (await _store).writeList(filename, data);
  }

  Future<List<T>> _readModels<T>(
    String filename,
    T Function(Map<String, dynamic>) decode,
  ) async {
    final raw = await _readList(filename);
    try {
      return raw.map(decode).toList();
    } catch (error) {
      throw StorageCorruptionException(filename, error);
    }
  }

  // ── 章节 CRUD ─────────────────────────────────────────────────────

  Future<List<Chapter>> _listChapters() async {
    return await _readModels('chapters.json', Chapter.fromJson)
      ..sort((a, b) => b.updatedAt.compareTo(a.updatedAt));
  }

  Future<List<Chapter>> listChapters() {
    return _transactions.run(_listChapters);
  }

  Future<void> saveChapter(Chapter ch) {
    return _transactions.run(() async {
      final all = await _listChapters();
      final idx = all.indexWhere((c) => c.id == ch.id);
      ch.updatedAt = DateTime.now();
      if (idx >= 0) {
        all[idx] = ch;
      } else {
        all.insert(0, ch);
      }
      await _writeList('chapters.json', all.map((c) => c.toJson()).toList());
    });
  }

  Future<void> deleteChapter(String id) {
    return _transactions.run(() async {
      final all = await _listChapters();
      all.removeWhere((c) => c.id == id);
      await _writeList('chapters.json', all.map((c) => c.toJson()).toList());

      // Hold the same transaction through the cross-file cascade.
      final checkpoints = await _listCheckpoints();
      final remaining = checkpoints.where((c) => c.chapterId != id).toList();
      await _writeList(
        'checkpoints.json',
        remaining.map((c) => c.toJson()).toList(),
      );
    });
  }

  // ── 记忆 CRUD ─────────────────────────────────────────────────────

  Future<List<Memory>> _listMemories() async {
    return await _readModels('memories.json', Memory.fromJson)
      ..sort((a, b) => b.createdAt.compareTo(a.createdAt));
  }

  Future<List<Memory>> listMemories() {
    return _transactions.run(_listMemories);
  }

  Future<void> saveMemory(Memory memory) {
    return _transactions.run(() async {
      final all = await _listMemories();
      final idx = all.indexWhere((item) => item.id == memory.id);
      if (idx >= 0) {
        all[idx] = memory;
      } else {
        all.insert(0, memory);
      }
      await _writeList(
        'memories.json',
        all.map((item) => item.toJson()).toList(),
      );
    });
  }

  Future<void> deleteMemory(String id) {
    return _transactions.run(() async {
      final all = await _listMemories();
      all.removeWhere((memory) => memory.id == id);
      await _writeList(
        'memories.json',
        all.map((memory) => memory.toJson()).toList(),
      );
    });
  }

  // ── 快照 CRUD ─────────────────────────────────────────────────────

  Future<List<Checkpoint>> _listCheckpoints() async {
    return await _readModels('checkpoints.json', Checkpoint.fromJson)
      ..sort((a, b) => b.createdAt.compareTo(a.createdAt));
  }

  Future<List<Checkpoint>> listCheckpoints() {
    return _transactions.run(_listCheckpoints);
  }

  Future<Checkpoint> createCheckpoint(Chapter chapter, String message) {
    return _transactions.run(() async {
      final checkpoint = Checkpoint(
        id: _nextLocalId('cp'),
        chapterId: chapter.id,
        chapterTitle: chapter.title,
        content: chapter.content,
        message: message,
        createdAt: DateTime.now(),
      );
      final all = await _listCheckpoints();
      all.insert(0, checkpoint);
      await _writeList(
        'checkpoints.json',
        all.map((item) => item.toJson()).toList(),
      );
      return checkpoint;
    });
  }

  Future<void> restoreCheckpoint(Checkpoint checkpoint) {
    return _transactions.run(() async {
      final chapters = await _listChapters();
      final idx = chapters.indexWhere(
        (item) => item.id == checkpoint.chapterId,
      );
      if (idx < 0) return;
      chapters[idx]
        ..content = checkpoint.content
        ..updatedAt = DateTime.now();
      await _writeList(
        'chapters.json',
        chapters.map((item) => item.toJson()).toList(),
      );
    });
  }

  Future<void> deleteCheckpoint(String id) {
    return _transactions.run(() async {
      final all = await _listCheckpoints();
      all.removeWhere((checkpoint) => checkpoint.id == id);
      await _writeList(
        'checkpoints.json',
        all.map((checkpoint) => checkpoint.toJson()).toList(),
      );
    });
  }

  // ── AI 供应商设置 ─────────────────────────────────────────────────

  Future<AiProvider?> loadProvider() {
    return _transactions.run(_providerConfig.load);
  }

  Future<void> saveProvider(AiProvider provider) {
    return _transactions.run(() => _providerConfig.save(provider));
  }

  // ── 历史会话 CRUD ─────────────────────────────────────────────────

  Future<List<ConversationRecord>> _listConversations() async {
    return await _readModels('conversations.json', ConversationRecord.fromJson)
      ..sort((a, b) => b.updatedAt.compareTo(a.updatedAt));
  }

  Future<List<ConversationRecord>> listConversations() {
    return _transactions.run(_listConversations);
  }

  Future<void> saveConversation(ConversationRecord conversation) {
    return _transactions.run(() async {
      final all = await _listConversations();
      conversation.updatedAt = DateTime.now();
      final idx = all.indexWhere((item) => item.id == conversation.id);
      if (idx >= 0) {
        all[idx] = conversation;
      } else {
        all.insert(0, conversation);
      }
      final trimmed = all.take(100).toList();
      await _writeList(
        'conversations.json',
        trimmed.map((item) => item.toJson()).toList(),
      );
    });
  }

  Future<void> deleteConversation(String id) {
    return _transactions.run(() async {
      final all = await _listConversations();
      all.removeWhere((conversation) => conversation.id == id);
      await _writeList(
        'conversations.json',
        all.map((conversation) => conversation.toJson()).toList(),
      );
    });
  }
}

// ── 历史会话记录 ─────────────────────────────────────────────────────

class ConversationMessage {
  final String role; // 'user' | 'assistant'
  final String content;
  final DateTime timestamp;

  const ConversationMessage({
    required this.role,
    required this.content,
    required this.timestamp,
  });

  Map<String, dynamic> toJson() => {
    'role': role,
    'content': content,
    'timestamp': timestamp.toIso8601String(),
  };

  factory ConversationMessage.fromJson(Map<String, dynamic> j) =>
      ConversationMessage(
        role: j['role'] as String? ?? 'user',
        content: j['content'] as String? ?? '',
        timestamp:
            DateTime.tryParse(j['timestamp'] as String? ?? '') ??
            DateTime.now(),
      );
}

class ConversationRecord {
  final String id;
  String title;
  List<ConversationMessage> messages;
  final DateTime createdAt;
  DateTime updatedAt;
  String providerName;
  String modelName;

  ConversationRecord({
    required this.id,
    required this.title,
    required this.messages,
    required this.createdAt,
    required this.updatedAt,
    required this.providerName,
    required this.modelName,
  });

  /// 自动从第一条用户消息提取标题
  static String titleFrom(List<ConversationMessage> msgs) {
    final first = msgs.firstWhere(
      (m) => m.role == 'user' && m.content.isNotEmpty,
      orElse: () => ConversationMessage(
        role: 'user',
        content: '新对话',
        timestamp: DateTime.now(),
      ),
    );
    final text = first.content.replaceAll('\n', ' ').trim();
    return text.length > 18 ? '${text.substring(0, 18)}…' : text;
  }

  /// 最后一条助手消息的文本预览
  String get preview {
    final last = messages.lastWhere(
      (m) => m.role == 'assistant' && m.content.isNotEmpty,
      orElse: () => ConversationMessage(
        role: 'assistant',
        content: '',
        timestamp: DateTime.now(),
      ),
    );
    final text = last.content.replaceAll('\n', ' ').trim();
    return text.length > 40 ? '${text.substring(0, 40)}…' : text;
  }

  Map<String, dynamic> toJson() => {
    'id': id,
    'title': title,
    'messages': messages.map((m) => m.toJson()).toList(),
    'createdAt': createdAt.toIso8601String(),
    'updatedAt': updatedAt.toIso8601String(),
    'providerName': providerName,
    'modelName': modelName,
  };

  factory ConversationRecord.fromJson(Map<String, dynamic> j) {
    final msgs = (j['messages'] as List? ?? [])
        .map((e) => ConversationMessage.fromJson(e as Map<String, dynamic>))
        .toList();
    return ConversationRecord(
      id: j['id'] as String? ?? '',
      title: j['title'] as String? ?? '对话',
      messages: msgs,
      createdAt:
          DateTime.tryParse(j['createdAt'] as String? ?? '') ?? DateTime.now(),
      updatedAt:
          DateTime.tryParse(j['updatedAt'] as String? ?? '') ?? DateTime.now(),
      providerName: j['providerName'] as String? ?? '',
      modelName: j['modelName'] as String? ?? '',
    );
  }

  factory ConversationRecord.create({
    required String providerName,
    required String modelName,
  }) {
    final now = DateTime.now();
    return ConversationRecord(
      id: _nextLocalId('conv'),
      title: '新对话',
      messages: [],
      createdAt: now,
      updatedAt: now,
      providerName: providerName,
      modelName: modelName,
    );
  }
}
