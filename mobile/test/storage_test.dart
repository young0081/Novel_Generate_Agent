import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:novel_mobile/storage.dart';

void main() {
  late Directory directory;
  late JsonListStore store;

  setUp(() async {
    directory = await Directory.systemTemp.createTemp('novel_mobile_store_');
    store = JsonListStore(directory);
  });

  tearDown(() async {
    if (await directory.exists()) await directory.delete(recursive: true);
  });

  test(
    'writes atomically and keeps the previous valid value as backup',
    () async {
      await store.writeList('chapters.json', [
        {'id': 'first'},
      ]);
      await store.writeList('chapters.json', [
        {'id': 'second'},
      ]);

      expect(await store.readList('chapters.json'), [
        {'id': 'second'},
      ]);
      final backup = File(
        '${directory.path}${Platform.pathSeparator}chapters.json.bak',
      );
      expect(jsonDecode(await backup.readAsString()), [
        {'id': 'first'},
      ]);
    },
  );

  test('recovers a corrupt primary from the last-known-good backup', () async {
    await store.writeList('memories.json', [
      {'id': 'safe'},
    ]);
    await store.writeList('memories.json', [
      {'id': 'newer'},
    ]);
    final primary = File(
      '${directory.path}${Platform.pathSeparator}memories.json',
    );
    await primary.writeAsString('{truncated', flush: true);

    expect(await store.readList('memories.json'), [
      {'id': 'safe'},
    ]);
    expect(await store.readList('memories.json'), [
      {'id': 'safe'},
    ]);
  });

  test(
    'corrupt primary and backup are reported and never overwritten',
    () async {
      final primary = File(
        '${directory.path}${Platform.pathSeparator}data.json',
      );
      final backup = File(
        '${directory.path}${Platform.pathSeparator}data.json.bak',
      );
      await primary.writeAsString('[broken', flush: true);
      await backup.writeAsString('{also-broken', flush: true);

      await expectLater(
        store.readList('data.json'),
        throwsA(isA<StorageCorruptionException>()),
      );
      await expectLater(
        store.writeList('data.json', [
          {'id': 'would-destroy-evidence'},
        ]),
        throwsA(isA<StorageCorruptionException>()),
      );
      expect(await primary.readAsString(), '[broken');
      expect(await backup.readAsString(), '{also-broken');
    },
  );

  test('serializes overlapping temp and backup file rotations', () async {
    await Future.wait(
      List.generate(
        12,
        (index) => store.writeList('conversations.json', [
          {'id': '$index'},
        ]),
      ),
    );

    expect(await store.readList('conversations.json'), [
      {'id': '11'},
    ]);
    final backup = File(
      '${directory.path}${Platform.pathSeparator}conversations.json.bak',
    );
    expect(jsonDecode(await backup.readAsString()), [
      {'id': '10'},
    ]);
  });

  test(
    'serializes the complete chapter read-modify-write transaction',
    () async {
      final persistence = BlockingJsonListPersistence('chapters.json');
      final storage = LocalStorage.forTesting(jsonStore: persistence);
      final first = chapter('first');
      final second = chapter('second');

      final firstSave = storage.saveChapter(first);
      await persistence.blockedWriteStarted.future;
      expect(persistence.readCount('chapters.json'), 1);

      final secondSave = storage.saveChapter(second);
      await Future<void>.delayed(Duration.zero);
      expect(
        persistence.readCount('chapters.json'),
        1,
        reason: 'the second mutation must not read a stale snapshot',
      );

      persistence.releaseBlockedWrite();
      await Future.wait([firstSave, secondSave]);

      final saved = await storage.listChapters();
      expect(saved.map((item) => item.id).toSet(), {'first', 'second'});
    },
  );

  test('holds deleteChapter across chapter and checkpoint files', () async {
    final deletedChapter = chapter('deleted');
    final removedCheckpoint = checkpoint('removed', deletedChapter.id);
    final retainedCheckpoint = checkpoint('retained', 'other');
    final persistence = BlockingJsonListPersistence(
      'checkpoints.json',
      initial: {
        'chapters.json': [deletedChapter.toJson()],
        'checkpoints.json': [
          removedCheckpoint.toJson(),
          retainedCheckpoint.toJson(),
        ],
      },
    );
    final storage = LocalStorage.forTesting(jsonStore: persistence);

    final deleteChapter = storage.deleteChapter(deletedChapter.id);
    await persistence.blockedWriteStarted.future;
    expect(persistence.readCount('checkpoints.json'), 1);

    final deleteOtherCheckpoint = storage.deleteCheckpoint(
      retainedCheckpoint.id,
    );
    await Future<void>.delayed(Duration.zero);
    expect(
      persistence.readCount('checkpoints.json'),
      1,
      reason: 'the cascade must retain the transaction gate between files',
    );

    persistence.releaseBlockedWrite();
    await Future.wait([deleteChapter, deleteOtherCheckpoint]);

    expect(await storage.listChapters(), isEmpty);
    expect(await storage.listCheckpoints(), isEmpty);
  });
}

Chapter chapter(String id) {
  final createdAt = DateTime.utc(2026, 1, 1);
  return Chapter(
    id: id,
    title: id,
    content: 'content-$id',
    createdAt: createdAt,
    updatedAt: createdAt,
  );
}

Checkpoint checkpoint(String id, String chapterId) => Checkpoint(
  id: id,
  chapterId: chapterId,
  chapterTitle: chapterId,
  content: 'checkpoint-$id',
  message: id,
  createdAt: DateTime.utc(2026, 1, 1),
);

class BlockingJsonListPersistence implements JsonListPersistence {
  final String blockedFilename;
  final Map<String, List<Map<String, dynamic>>> _files;
  final Map<String, int> _reads = {};
  final Completer<void> blockedWriteStarted = Completer<void>();
  final Completer<void> _release = Completer<void>();
  bool _blocked = false;

  BlockingJsonListPersistence(
    this.blockedFilename, {
    Map<String, List<Map<String, dynamic>>> initial = const {},
  }) : _files = {
         for (final entry in initial.entries) entry.key: _copy(entry.value),
       };

  int readCount(String filename) => _reads[filename] ?? 0;

  void releaseBlockedWrite() {
    if (!_release.isCompleted) _release.complete();
  }

  @override
  Future<List<Map<String, dynamic>>> readList(String filename) async {
    _reads.update(filename, (value) => value + 1, ifAbsent: () => 1);
    return _copy(_files[filename] ?? const []);
  }

  @override
  Future<void> writeList(
    String filename,
    List<Map<String, dynamic>> data,
  ) async {
    if (!_blocked && filename == blockedFilename) {
      _blocked = true;
      blockedWriteStarted.complete();
      await _release.future;
    }
    _files[filename] = _copy(data);
  }

  static List<Map<String, dynamic>> _copy(List<Map<String, dynamic>> source) =>
      source.map((item) => Map<String, dynamic>.from(item)).toList();
}
