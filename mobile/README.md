# Novel Generate Agent Mobile

Flutter client for the Novel Generate Agent project. The mobile app stores
chapters, memories, checkpoints, and conversation history on the device and
connects directly to the model provider selected by the user.

## Requirements

- Flutter 3.44 or newer
- Dart 3.12 or newer
- Android Studio and an Android SDK for Android builds
- macOS, Xcode, and an Apple signing identity for iOS builds

Install dependencies and run the checks from this directory:

```bash
flutter pub get
flutter test
dart analyze lib test
```

## Local Data

Content data is stored under the app documents directory in `novel_agent` as
JSON files. Writes use a validated temporary file and retain the previous valid
file as a `.bak` recovery copy. Reads automatically restore that backup when a
primary file is missing or contains invalid JSON. If both copies are damaged,
the app reports the corruption instead of overwriting the remaining evidence.

All logical content mutations are serialized. A read/modify/write update, and
the chapter deletion cascade across chapter and checkpoint files, run as one
local transaction so overlapping UI actions cannot silently drop each other.

The chapters, memories, checkpoints, and conversation-history screens support
`批量管理`: select visible records, confirm deletion once, and watch progress.
Each record is deleted through the same serialized storage transaction used by
single-item actions; failures are reported without hiding successful deletions.

## API Key Storage

Provider metadata is stored in platform preferences. API keys are stored
separately with `flutter_secure_storage`:

- Android: encrypted storage backed by Android Keystore. App backup is disabled
  so encrypted preferences are not restored without their device-bound key.
- iOS: Keychain, enabled by the Runner entitlements for Debug, Profile, and
  Release configurations.

Older releases stored the API key in the `ai_provider` preference. The first
successful load stages that key in a versioned secure slot, commits a metadata
pointer to the slot, and only then removes legacy copies. Provider switches use
the same staged-slot protocol, so a process interruption cannot combine one
provider's endpoint with another provider's key. Unreferenced old slots are
cleaned up on a best-effort basis.

The network security configuration blocks general cleartext traffic. Local
network exceptions exist only for local model workflows such as Ollama; normal
provider endpoints should use HTTPS.

## Build

Android debug and release builds:

```bash
flutter build apk --debug
flutter build apk --release
```

iOS builds must run on macOS with a configured signing team:

```bash
flutter build ios --release
```
