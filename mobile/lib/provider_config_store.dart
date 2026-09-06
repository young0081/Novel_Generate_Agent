import 'dart:convert';
import 'dart:math';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'ai_client.dart';

String _createSecureApiKeySlot() {
  final random = Random.secure();
  final timestamp = DateTime.now().microsecondsSinceEpoch.toRadixString(16);
  final first = random.nextInt(0x7fffffff).toRadixString(16).padLeft(8, '0');
  final second = random.nextInt(0x7fffffff).toRadixString(16).padLeft(8, '0');
  return '${ProviderConfigStore.secureApiKeySlotPrefix}'
      '${timestamp}_$first$second';
}

abstract interface class ProviderPreferencesStore {
  Future<String?> read(String key);

  Future<void> write(String key, String value);
}

abstract interface class ProviderSecretStore {
  Future<String?> read(String key);

  Future<void> write(String key, String value);

  Future<void> delete(String key);
}

class SharedPreferencesProviderStore implements ProviderPreferencesStore {
  @override
  Future<String?> read(String key) async {
    // Keep the legacy API here: older releases wrote `ai_provider` to its
    // Android SharedPreferences backend, while SharedPreferencesAsync defaults
    // to DataStore and would miss the value that needs migration.
    final preferences = await SharedPreferences.getInstance();
    return preferences.getString(key);
  }

  @override
  Future<void> write(String key, String value) async {
    final preferences = await SharedPreferences.getInstance();
    final saved = await preferences.setString(key, value);
    if (!saved) throw StateError('SharedPreferences rejected the write');
  }
}

class FlutterSecureProviderStore implements ProviderSecretStore {
  static const _defaultStorage = FlutterSecureStorage(
    aOptions: AndroidOptions(resetOnError: false, migrateWithBackup: true),
  );

  final FlutterSecureStorage _storage;

  const FlutterSecureProviderStore([
    FlutterSecureStorage storage = _defaultStorage,
  ]) : _storage = storage;

  @override
  Future<String?> read(String key) => _storage.read(key: key);

  @override
  Future<void> write(String key, String value) =>
      _storage.write(key: key, value: value);

  @override
  Future<void> delete(String key) => _storage.delete(key: key);
}

class ProviderStorageException implements Exception {
  final String message;
  final Object cause;
  final StackTrace causeStackTrace;

  const ProviderStorageException(
    this.message,
    this.cause,
    this.causeStackTrace,
  );

  @override
  String toString() => message;
}

/// Persists provider metadata separately from its API key.
///
/// Older releases stored both in the `ai_provider` preference. On the first
/// successful load, the key is copied to secure storage before the plaintext
/// field is removed. A failed migration leaves the legacy value untouched so
/// it can be retried without losing the key.
class ProviderConfigStore {
  static const preferencesKey = 'ai_provider';
  // Releases before the slot-based format stored the current key here.
  static const secureApiKeyKey = 'ai_provider_api_key';
  static const secureApiKeySlotPrefix = 'ai_provider_api_key_v2_';
  static const apiKeySlotField = 'apiKeySlot';
  static const apiKeyStorageVersionField = 'apiKeyStorageVersion';
  static const apiKeyStorageVersion = 2;

  final ProviderPreferencesStore preferences;
  final ProviderSecretStore secrets;
  final String Function() _secureSlotFactory;

  ProviderConfigStore({
    required this.preferences,
    required this.secrets,
    String Function()? secureSlotFactory,
  }) : _secureSlotFactory = secureSlotFactory ?? _createSecureApiKeySlot;

  factory ProviderConfigStore.platform() => ProviderConfigStore(
    preferences: SharedPreferencesProviderStore(),
    secrets: const FlutterSecureProviderStore(),
  );

  Future<AiProvider?> load() async {
    final serialized = await _readPreferences();
    if (serialized == null) return null;

    final decoded = _decodeProvider(serialized);
    if (decoded == null) return null;

    final activeSlot = _slotPointer(decoded);
    if (activeSlot != null) {
      final apiKey = await _readSecret(
        activeSlot,
        '无法从系统安全存储读取 API Key；原有 Key 未被删除。',
      );

      if (decoded.containsKey('apiKey')) {
        decoded.remove('apiKey');
        await _writePreferences(
          jsonEncode(decoded),
          'API Key 已写入安全存储，但无法移除旧的明文副本，请重试。',
        );
      }

      await _bestEffortDelete(secureApiKeyKey);
      return _providerWithKey(decoded, apiKey ?? '');
    }

    if (decoded[apiKeyStorageVersionField] == apiKeyStorageVersion) {
      if (decoded.containsKey('apiKey')) {
        decoded.remove('apiKey');
        await _writePreferences(
          jsonEncode(decoded),
          '无法移除旧的明文 API Key 副本，请重试。',
        );
      }
      await _bestEffortDelete(secureApiKeyKey);
      return _providerWithKey(decoded, '');
    }

    return _migrateLegacy(decoded);
  }

  Future<AiProvider?> _migrateLegacy(Map<String, dynamic> decoded) async {
    final legacyValue = decoded['apiKey'];
    final legacyApiKey = legacyValue is String ? legacyValue : '';
    final legacySecureApiKey = await _readSecret(
      secureApiKeyKey,
      '无法从系统安全存储读取 API Key；原有 Key 未被删除。',
    );
    final apiKey = legacySecureApiKey != null && legacySecureApiKey.isNotEmpty
        ? legacySecureApiKey
        : legacyApiKey;

    final migrated = Map<String, dynamic>.from(decoded)
      ..remove('apiKey')
      ..remove(apiKeySlotField)
      ..[apiKeyStorageVersionField] = apiKeyStorageVersion;

    if (apiKey.isNotEmpty) {
      final slot = await _allocateSlot();
      await _writeSecret(slot, apiKey, '无法将原有 API Key 迁移到系统安全存储；原有 Key 未被删除。');
      migrated[apiKeySlotField] = slot;
    }

    // A failed or ambiguously committed metadata write deliberately leaves the
    // staged slot intact. Whichever metadata version is durable still points
    // to its matching key, and an unreferenced slot is harmless.
    await _writePreferences(
      jsonEncode(migrated),
      'API Key 已写入安全存储，但无法移除旧的明文副本，请重试。',
    );

    await _bestEffortDelete(secureApiKeyKey);
    return _providerWithKey(migrated, apiKey);
  }

  Future<void> save(AiProvider provider) async {
    final previousSerialized = await _readPreferences();
    final previousMetadata = previousSerialized == null
        ? null
        : _decodeProvider(previousSerialized);
    final previousSlot = previousMetadata == null
        ? null
        : _slotPointer(previousMetadata);

    String? nextSlot;
    if (provider.apiKey.isNotEmpty) {
      nextSlot = await _allocateSlot(excluding: previousSlot);
      await _writeSecret(
        nextSlot,
        provider.apiKey,
        '无法将 API Key 写入系统安全存储；原有 Key 未被删除。',
      );
    }

    final metadata = _metadata(provider, apiKeySlot: nextSlot);
    await _writePreferences(jsonEncode(metadata), 'API Key 已安全保存，但模型设置保存失败。');

    if (previousSlot != null && previousSlot != nextSlot) {
      await _bestEffortDelete(previousSlot);
    }
    await _bestEffortDelete(secureApiKeyKey);
  }

  Map<String, dynamic>? _decodeProvider(String serialized) {
    try {
      final value = jsonDecode(serialized);
      if (value is! Map) return null;
      return Map<String, dynamic>.from(value);
    } catch (_) {
      return null;
    }
  }

  Map<String, dynamic> _metadata(AiProvider provider, {String? apiKeySlot}) => {
    'name': provider.name,
    'protocol': provider.protocol.name,
    'baseUrl': provider.baseUrl,
    'model': provider.model,
    apiKeyStorageVersionField: apiKeyStorageVersion,
    apiKeySlotField: ?apiKeySlot,
  };

  String? _slotPointer(Map<String, dynamic> metadata) {
    final value = metadata[apiKeySlotField];
    if (value is! String || !value.startsWith(secureApiKeySlotPrefix)) {
      return null;
    }
    return value;
  }

  AiProvider? _providerWithKey(Map<String, dynamic> metadata, String apiKey) {
    final decoded = Map<String, dynamic>.from(metadata)..['apiKey'] = apiKey;
    try {
      return AiProvider.fromJson(decoded);
    } catch (_) {
      return null;
    }
  }

  Future<String> _allocateSlot({String? excluding}) async {
    for (var attempt = 0; attempt < 8; attempt++) {
      final candidate = _secureSlotFactory();
      if (!candidate.startsWith(secureApiKeySlotPrefix) ||
          candidate == secureApiKeyKey ||
          candidate == excluding) {
        continue;
      }
      final existing = await _readSecret(
        candidate,
        '无法检查系统安全存储中的 API Key 槽位，请重试。',
      );
      if (existing == null) return candidate;
    }

    final error = StateError('unable to allocate a unique API key slot');
    throw ProviderStorageException(
      '无法创建新的 API Key 安全存储槽位，请重试。',
      error,
      StackTrace.current,
    );
  }

  Future<String?> _readPreferences() async {
    try {
      return await preferences.read(preferencesKey);
    } catch (error, stackTrace) {
      throw ProviderStorageException('无法读取本地模型设置，请重试。', error, stackTrace);
    }
  }

  Future<void> _writePreferences(String value, String message) async {
    try {
      await preferences.write(preferencesKey, value);
    } catch (error, stackTrace) {
      throw ProviderStorageException(message, error, stackTrace);
    }
  }

  Future<String?> _readSecret(String key, String message) async {
    try {
      return await secrets.read(key);
    } catch (error, stackTrace) {
      throw ProviderStorageException(message, error, stackTrace);
    }
  }

  Future<void> _writeSecret(String key, String value, String message) async {
    try {
      await secrets.write(key, value);
    } catch (error, stackTrace) {
      throw ProviderStorageException(message, error, stackTrace);
    }
  }

  Future<void> _bestEffortDelete(String key) async {
    try {
      await secrets.delete(key);
    } catch (_) {}
  }
}
