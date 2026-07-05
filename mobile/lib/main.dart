import 'package:flutter/material.dart';

import 'motion.dart';
import 'rpc.dart';
import 'settings.dart';
import 'screens/checkpoint_screen.dart';
import 'screens/files_screen.dart';
import 'screens/memory_screen.dart';
import 'screens/settings_screen.dart';
import 'screens/tools_screen.dart';

void main() {
  runApp(const NovelApp());
}

class NovelApp extends StatelessWidget {
  const NovelApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Novel Generate Team',
      debugShowCheckedModeBanner: false,
      theme: _buildTheme(),
      home: const HomePage(),
    );
  }
}

ThemeData _buildTheme() {
  final scheme = ColorScheme.fromSeed(
    seedColor: const Color(0xFF7C9CFF),
    brightness: Brightness.dark,
  );
  return ThemeData(
    useMaterial3: true,
    brightness: Brightness.dark,
    colorScheme: scheme,
    scaffoldBackgroundColor: const Color(0xFF0F1115),
    // Any pushed route fades + slides through, consistent with in-app motion.
    pageTransitionsTheme: const PageTransitionsTheme(
      builders: {
        TargetPlatform.android: _FadeThroughTransitionsBuilder(),
        TargetPlatform.iOS: _FadeThroughTransitionsBuilder(),
      },
    ),
    cardTheme: CardThemeData(
      elevation: 0,
      color: const Color(0xFF161A21),
      margin: EdgeInsets.zero,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(16),
        side: BorderSide(color: scheme.outlineVariant.withValues(alpha: 0.5)),
      ),
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: const Color(0xFF11151C),
      isDense: true,
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide(color: scheme.outlineVariant),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide(color: scheme.outlineVariant),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide(color: scheme.primary, width: 1.6),
      ),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
      ),
    ),
    snackBarTheme: const SnackBarThemeData(behavior: SnackBarBehavior.floating),
  );
}

/// Drives [MaterialApp] route transitions through the same fade + lift used by
/// [FadeThroughPageRoute], so navigation matches the rest of the motion. Snaps
/// instantly under reduced motion.
class _FadeThroughTransitionsBuilder extends PageTransitionsBuilder {
  const _FadeThroughTransitionsBuilder();

  @override
  Widget buildTransitions<T>(
    PageRoute<T> route,
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
    Widget child,
  ) {
    if (Motion.reduced(context)) return child;
    final curved = CurvedAnimation(
      parent: animation,
      curve: Motion.standard,
      reverseCurve: Motion.smooth,
    );
    return FadeTransition(
      opacity: curved,
      child: SlideTransition(
        position: Tween<Offset>(
          begin: const Offset(0, 0.035),
          end: Offset.zero,
        ).animate(curved),
        child: child,
      ),
    );
  }
}

class HomePage extends StatefulWidget {
  const HomePage({super.key});

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  final RpcService _rpc = RpcService(Settings.defaultUrl);
  int _index = 0;
  // Direction of the last tab change: +1 = moved right, -1 = moved left.
  // Drives which way the body slides during the cross-fade.
  int _direction = 1;
  bool _loaded = false;

  @override
  void initState() {
    super.initState();
    Settings.loadServerUrl().then((url) {
      if (!mounted) return;
      setState(() {
        _rpc.baseUrl = url;
        _loaded = true;
      });
    });
  }

  void _onUrlChanged(String url) {
    setState(() => _rpc.baseUrl = url);
    Settings.saveServerUrl(url);
  }

  void _selectTab(int i) {
    if (i == _index) return;
    setState(() {
      _direction = i > _index ? 1 : -1;
      _index = i;
    });
  }

  static const _titles = ['文件工作台', '记忆库', '检查点', '工具目录', '设置'];

  @override
  Widget build(BuildContext context) {
    if (!_loaded) {
      return Scaffold(
        body: Center(
          child: TweenAnimationBuilder<double>(
            tween: Tween(begin: 0, end: 1),
            duration: Motion.dur(context, Motion.slow),
            curve: Motion.standard,
            builder: (context, t, child) => Opacity(opacity: t, child: child),
            child: const Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                CircularProgressIndicator(),
                SizedBox(height: 16),
                Text('正在启动…'),
              ],
            ),
          ),
        ),
      );
    }

    final screens = <Widget>[
      FilesScreen(rpc: _rpc),
      MemoryScreen(rpc: _rpc),
      CheckpointScreen(rpc: _rpc),
      ToolsScreen(rpc: _rpc),
      SettingsScreen(rpc: _rpc, onChanged: _onUrlChanged),
    ];

    return Scaffold(
      appBar: AppBar(
        title: AnimatedSwitcher(
          duration: Motion.dur(context, Motion.normal),
          switchInCurve: Motion.standard,
          switchOutCurve: Motion.smooth,
          transitionBuilder: (child, animation) => FadeTransition(
            opacity: animation,
            child: SlideTransition(
              position: Tween<Offset>(
                begin: const Offset(0, 0.25),
                end: Offset.zero,
              ).animate(animation),
              child: child,
            ),
          ),
          child: Text(
            'Novel Generate Team · ${_titles[_index]}',
            key: ValueKey<int>(_index),
          ),
        ),
      ),
      body: AnimatedSwitcher(
        duration: Motion.dur(context, Motion.normal),
        switchInCurve: Motion.standard,
        switchOutCurve: Motion.smooth,
        transitionBuilder: (child, animation) {
          // The incoming child keys its own slide direction via _direction at
          // build time; reverse children fade without sliding to avoid clashes.
          final isIncoming = child.key == ValueKey<int>(_index);
          final dx = isIncoming ? 0.06 * _direction : 0.0;
          return FadeTransition(
            opacity: animation,
            child: SlideTransition(
              position: Tween<Offset>(
                begin: Offset(dx, 0),
                end: Offset.zero,
              ).animate(animation),
              child: child,
            ),
          );
        },
        layoutBuilder: (currentChild, previousChildren) => Stack(
          alignment: Alignment.topCenter,
          children: [...previousChildren, ?currentChild],
        ),
        child: KeyedSubtree(key: ValueKey<int>(_index), child: screens[_index]),
      ),
      bottomNavigationBar: NavigationBar(
        selectedIndex: _index,
        onDestinationSelected: _selectTab,
        destinations: const [
          NavigationDestination(icon: Icon(Icons.edit_document), label: '文件'),
          NavigationDestination(icon: Icon(Icons.psychology_alt), label: '记忆'),
          NavigationDestination(icon: Icon(Icons.save_outlined), label: '快照'),
          NavigationDestination(
            icon: Icon(Icons.handyman_outlined),
            label: '工具',
          ),
          NavigationDestination(
            icon: Icon(Icons.settings_outlined),
            label: '设置',
          ),
        ],
      ),
    );
  }
}
