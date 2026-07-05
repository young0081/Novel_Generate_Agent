import 'package:shared_preferences/shared_preferences.dart';

/// On-device persistence for the backend server URL.
class Settings {
  static const _key = 'server_url';

  /// Default points at the Android emulator's alias for the host machine's
  /// localhost (10.0.2.2). On a real device, set the desktop's LAN IP in Settings.
  static const defaultUrl = 'http://10.0.2.2:3000';

  static Future<String> loadServerUrl() async {
    final prefs = await SharedPreferences.getInstance();
    return prefs.getString(_key) ?? defaultUrl;
  }

  static Future<void> saveServerUrl(String url) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_key, url);
  }
}
