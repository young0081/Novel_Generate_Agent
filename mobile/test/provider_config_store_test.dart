import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:novel_mobile/ai_client.dart';
import 'package:novel_mobile/provider_config_store.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  late FakePreferencesStore preferences;
  late FakeSecretStore secrets;
  late ProviderConfigStore store;
  late int slotSequence;

  setUp(() {
    preferences = FakePreferencesStore();
    secrets = FakeSecretStore();
    slotSequence = 0;
    store = ProviderConfigStore(
      preferences: preferences,
      secrets: secrets,
      secureSlotFactory: () => testSlot(++slotSequence),
    );
  });

  test(
    'saves only provider metadata in preferences and key securely',
    () async {
      await store.save(testProvider);

      final metadata = decodeMetadata(preferences);
      expect(metadata, providerMetadata(testProvider, slot: testSlot(1)));
      expect(secrets.values[testSlot(1)], testProvider.apiKey);
      expect(metadata, isNot(contains('apiKey')));
      expect(
        secrets.values,
        isNot(contains(ProviderConfigStore.secureApiKeyKey)),
      );
    },
  );

  test('migrates a legacy plaintext key exactly once', () async {
    preferences.value = jsonEncode(testProvider.toJson());

    final firstLoad = await store.load();
    final secondLoad = await store.load();

    expect(firstLoad?.apiKey, testProvider.apiKey);
    expect(secondLoad?.apiKey, testProvider.apiKey);
    expect(secrets.values[testSlot(1)], testProvider.apiKey);
    expect(secrets.writeCount, 1);
    expect(preferences.writeCount, 1);
    expect(
      decodeMetadata(preferences),
      providerMetadata(testProvider, slot: testSlot(1)),
    );
  });

  test(
    'migrates the legacy secure key and prefers it to stale plaintext',
    () async {
      preferences.value = jsonEncode(testProvider.toJson());
      secrets.values[ProviderConfigStore.secureApiKeyKey] = 'sk-current-secure';

      final provider = await store.load();

      expect(provider?.apiKey, 'sk-current-secure');
      expect(secrets.values[testSlot(1)], 'sk-current-secure');
      expect(
        secrets.values,
        isNot(contains(ProviderConfigStore.secureApiKeyKey)),
      );
      expect(
        decodeMetadata(preferences),
        providerMetadata(testProvider, slot: testSlot(1)),
      );
    },
  );

  test(
    'reads the same legacy preferences backend used by older releases',
    () async {
      SharedPreferences.setMockInitialValues({
        ProviderConfigStore.preferencesKey: jsonEncode(testProvider.toJson()),
      });
      final platformPreferencesStore = ProviderConfigStore(
        preferences: SharedPreferencesProviderStore(),
        secrets: secrets,
        secureSlotFactory: () => testSlot(++slotSequence),
      );

      final provider = await platformPreferencesStore.load();

      expect(provider?.apiKey, testProvider.apiKey);
      expect(secrets.values[testSlot(1)], testProvider.apiKey);
      final legacyPreferences = await SharedPreferences.getInstance();
      final migrated =
          jsonDecode(
                legacyPreferences.getString(
                  ProviderConfigStore.preferencesKey,
                )!,
              )
              as Map<String, dynamic>;
      expect(migrated, providerMetadata(testProvider, slot: testSlot(1)));
    },
  );

  test('keeps plaintext metadata when secure migration fails', () async {
    preferences.value = jsonEncode(testProvider.toJson());
    secrets.writeError = StateError('secure storage unavailable');

    await expectLater(store.load(), throwsA(isA<ProviderStorageException>()));

    final legacy = decodeMetadata(preferences);
    expect(legacy['apiKey'], testProvider.apiKey);
    expect(preferences.writeCount, 0);
    expect(secrets.values, isEmpty);
  });

  test(
    'keeps plaintext and an orphan stage when pointer commit fails',
    () async {
      preferences.value = jsonEncode(testProvider.toJson());
      preferences.writeErrorBeforeCommit = StateError(
        'preferences unavailable',
      );

      await expectLater(store.load(), throwsA(isA<ProviderStorageException>()));

      final legacy = decodeMetadata(preferences);
      expect(legacy['apiKey'], testProvider.apiKey);
      expect(secrets.values[testSlot(1)], testProvider.apiKey);
    },
  );

  test('a post-commit migration failure still loads the staged pair', () async {
    preferences.value = jsonEncode(testProvider.toJson());
    preferences.writeErrorAfterCommit = StateError('reply lost after commit');

    await expectLater(store.load(), throwsA(isA<ProviderStorageException>()));

    preferences.writeErrorAfterCommit = null;
    final loaded = await store.load();
    expectProviderPair(loaded, testProvider);
    expect(
      decodeMetadata(preferences)[ProviderConfigStore.apiKeySlotField],
      testSlot(1),
    );
  });

  test(
    'surfaces secure reads instead of treating failures as an empty key',
    () async {
      preferences.value = jsonEncode(
        providerMetadata(testProvider, slot: testSlot(1)),
      );
      secrets.values[testSlot(1)] = testProvider.apiKey;
      secrets.readErrors.add(testSlot(1));

      await expectLater(store.load(), throwsA(isA<ProviderStorageException>()));
    },
  );

  test('does not update metadata when staging the new key fails', () async {
    preferences.value = jsonEncode(
      providerMetadata(oldProvider, slot: oldSlot),
    );
    secrets.values[oldSlot] = oldProvider.apiKey;
    secrets.writeError = StateError('keystore write failed');

    await expectLater(
      store.save(testProvider),
      throwsA(isA<ProviderStorageException>()),
    );

    expect(
      decodeMetadata(preferences),
      providerMetadata(oldProvider, slot: oldSlot),
    );
    expect(secrets.values[oldSlot], oldProvider.apiKey);
    expect(preferences.writeCount, 0);
  });

  test('interruption before pointer commit preserves the old pair', () async {
    preferences.value = jsonEncode(
      providerMetadata(oldProvider, slot: oldSlot),
    );
    secrets.values[oldSlot] = oldProvider.apiKey;
    preferences.writeErrorBeforeCommit = StateError(
      'process stopped before commit',
    );

    await expectLater(
      store.save(testProvider),
      throwsA(isA<ProviderStorageException>()),
    );

    expect(secrets.values[testSlot(1)], testProvider.apiKey);
    expect(
      decodeMetadata(preferences),
      providerMetadata(oldProvider, slot: oldSlot),
    );

    preferences.writeErrorBeforeCommit = null;
    final loaded = await store.load();
    expectProviderPair(loaded, oldProvider);
    await expectRequestUsesPair(loaded!, oldProvider);
  });

  test('interruption after pointer commit preserves the new pair', () async {
    preferences.value = jsonEncode(
      providerMetadata(oldProvider, slot: oldSlot),
    );
    secrets.values[oldSlot] = oldProvider.apiKey;
    preferences.writeErrorAfterCommit = StateError(
      'process stopped after commit',
    );

    await expectLater(
      store.save(testProvider),
      throwsA(isA<ProviderStorageException>()),
    );

    expect(
      decodeMetadata(preferences),
      providerMetadata(testProvider, slot: testSlot(1)),
    );
    expect(secrets.values[testSlot(1)], testProvider.apiKey);
    expect(secrets.values[oldSlot], oldProvider.apiKey);

    preferences.writeErrorAfterCommit = null;
    final loaded = await store.load();
    expectProviderPair(loaded, testProvider);
    await expectRequestUsesPair(loaded!, testProvider);
  });

  test('confirmed pointer commit cleans the previous slot', () async {
    preferences.value = jsonEncode(
      providerMetadata(oldProvider, slot: oldSlot),
    );
    secrets.values[oldSlot] = oldProvider.apiKey;

    await store.save(testProvider);

    expect(secrets.values, isNot(contains(oldSlot)));
    expect(secrets.values[testSlot(1)], testProvider.apiKey);
    expectProviderPair(await store.load(), testProvider);
  });

  test(
    'clearing a key commits first and tolerates stale-slot cleanup failure',
    () async {
      preferences.value = jsonEncode(
        providerMetadata(oldProvider, slot: oldSlot),
      );
      secrets.values[oldSlot] = oldProvider.apiKey;
      secrets.deleteErrors.add(oldSlot);
      final cleared = AiProvider(
        name: oldProvider.name,
        protocol: oldProvider.protocol,
        baseUrl: oldProvider.baseUrl,
        apiKey: '',
        model: oldProvider.model,
      );

      await store.save(cleared);

      final metadata = decodeMetadata(preferences);
      expect(metadata, providerMetadata(cleared));
      expect(metadata, isNot(contains(ProviderConfigStore.apiKeySlotField)));
      expect(secrets.values[oldSlot], oldProvider.apiKey);
      expectProviderPair(await store.load(), cleared);
    },
  );

  test(
    'cleared v2 metadata never resurrects an undeleted legacy key',
    () async {
      preferences.value = jsonEncode(preSlotMetadata(oldProvider));
      secrets.values[ProviderConfigStore.secureApiKeyKey] = oldProvider.apiKey;
      secrets.deleteErrors.add(ProviderConfigStore.secureApiKeyKey);
      final cleared = AiProvider(
        name: oldProvider.name,
        protocol: oldProvider.protocol,
        baseUrl: oldProvider.baseUrl,
        apiKey: '',
        model: oldProvider.model,
      );

      await store.save(cleared);

      expectProviderPair(await store.load(), cleared);
      expect(slotSequence, 0);
      expect(
        secrets.values[ProviderConfigStore.secureApiKeyKey],
        oldProvider.apiKey,
      );
    },
  );
}

const oldSlot = '${ProviderConfigStore.secureApiKeySlotPrefix}old';

String testSlot(int sequence) =>
    '${ProviderConfigStore.secureApiKeySlotPrefix}test_$sequence';

const testProvider = AiProvider(
  name: 'Test Provider',
  protocol: AiProtocol.anthropic,
  baseUrl: 'https://example.test/v1',
  apiKey: 'sk-test-secret',
  model: 'test-model',
);

const oldProvider = AiProvider(
  name: 'Old Provider',
  protocol: AiProtocol.openAi,
  baseUrl: 'https://old.example.test/v1',
  apiKey: 'sk-old-secret',
  model: 'old-model',
);

Map<String, dynamic> providerMetadata(AiProvider provider, {String? slot}) => {
  'name': provider.name,
  'protocol': provider.protocol.name,
  'baseUrl': provider.baseUrl,
  'model': provider.model,
  ProviderConfigStore.apiKeyStorageVersionField:
      ProviderConfigStore.apiKeyStorageVersion,
  ProviderConfigStore.apiKeySlotField: ?slot,
};

Map<String, dynamic> preSlotMetadata(AiProvider provider) => {
  'name': provider.name,
  'protocol': provider.protocol.name,
  'baseUrl': provider.baseUrl,
  'model': provider.model,
};

Map<String, dynamic> decodeMetadata(FakePreferencesStore preferences) =>
    jsonDecode(preferences.value!) as Map<String, dynamic>;

void expectProviderPair(AiProvider? actual, AiProvider expected) {
  expect(actual, isNotNull);
  expect(actual?.name, expected.name);
  expect(actual?.protocol, expected.protocol);
  expect(actual?.baseUrl, expected.baseUrl);
  expect(actual?.model, expected.model);
  expect(actual?.apiKey, expected.apiKey);
}

Future<void> expectRequestUsesPair(
  AiProvider actual,
  AiProvider expected,
) async {
  final httpClient = RecordingHttpClient(expected.protocol);
  final client = AiClient(actual, httpClient: httpClient);
  try {
    await client.chat(const [AiMessage(role: 'user', content: 'test')]);
  } finally {
    client.close();
  }

  expect(httpClient.requestUri.toString(), startsWith(expected.baseUrl));
  if (expected.protocol == AiProtocol.openAi) {
    expect(
      httpClient.requestHeaders['authorization'],
      'Bearer ${expected.apiKey}',
    );
  } else {
    expect(httpClient.requestHeaders['x-api-key'], expected.apiKey);
  }
}

class FakePreferencesStore implements ProviderPreferencesStore {
  String? value;
  Object? readError;
  Object? writeErrorBeforeCommit;
  Object? writeErrorAfterCommit;
  int writeCount = 0;

  @override
  Future<String?> read(String key) async {
    if (readError case final error?) throw error;
    return value;
  }

  @override
  Future<void> write(String key, String value) async {
    writeCount++;
    if (writeErrorBeforeCommit case final error?) throw error;
    this.value = value;
    if (writeErrorAfterCommit case final error?) throw error;
  }
}

class FakeSecretStore implements ProviderSecretStore {
  final Map<String, String> values = {};
  final Set<String> readErrors = {};
  final Set<String> deleteErrors = {};
  Object? writeError;
  int writeCount = 0;
  int deleteCount = 0;

  @override
  Future<String?> read(String key) async {
    if (readErrors.contains(key)) throw StateError('secure read failed: $key');
    return values[key];
  }

  @override
  Future<void> write(String key, String value) async {
    writeCount++;
    if (writeError case final error?) throw error;
    values[key] = value;
  }

  @override
  Future<void> delete(String key) async {
    deleteCount++;
    if (deleteErrors.contains(key)) {
      throw StateError('secure delete failed: $key');
    }
    values.remove(key);
  }
}

class RecordingHttpClient extends http.BaseClient {
  final AiProtocol protocol;
  Uri requestUri = Uri();
  Map<String, String> requestHeaders = const {};

  RecordingHttpClient(this.protocol);

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    requestUri = request.url;
    requestHeaders = {
      for (final entry in request.headers.entries)
        entry.key.toLowerCase(): entry.value,
    };
    final responseBody = protocol == AiProtocol.openAi
        ? jsonEncode({
            'choices': [
              {
                'message': {'content': 'ok'},
              },
            ],
          })
        : jsonEncode({
            'content': [
              {'text': 'ok'},
            ],
          });
    return http.StreamedResponse(
      Stream<List<int>>.value(utf8.encode(responseBody)),
      200,
    );
  }
}
