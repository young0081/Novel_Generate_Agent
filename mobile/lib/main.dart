import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'motion.dart';
import 'ai_client.dart';
import 'storage.dart';
import 'screens/studio_screen.dart';
import 'screens/chapters_screen.dart';
import 'screens/memory_screen.dart';
import 'screens/checkpoint_screen.dart';
import 'screens/settings_screen.dart';

// ── 水墨国风色板 ────────────────────────────────────────────────
class InkPalette {
  InkPalette._();
  static const paper      = Color(0xFFEDE6D7);
  static const paperHi    = Color(0xFFF6F0E6);
  static const paperLo    = Color(0xFFE0D6C1);
  static const paperEdge  = Color(0xFFCFC1A3);
  static const ink        = Color(0xFF21201B);
  static const ink2       = Color(0xFF47433A);
  static const ink3       = Color(0xFF6C6258);
  static const ink4       = Color(0xFF968C7A);
  static const inkGhost   = Color(0xFFB0A490);
  static const cinnabar   = Color(0xFFB43022);
  static const cinnabarHi = Color(0xFFC5422D);
  static const cinnabarWash = Color(0x14B43022);
  static const teal       = Color(0xFF4E6560);
  static const gold       = Color(0xFFC4955A);
  static const line       = Color(0xFFD2C6AF);
}

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  SystemChrome.setPreferredOrientations([
    DeviceOrientation.portraitUp,
    DeviceOrientation.portraitDown,
  ]);
  runApp(const NovelApp());
}

class NovelApp extends StatelessWidget {
  const NovelApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: '墨·创作',
      debugShowCheckedModeBanner: false,
      theme: _buildTheme(),
      home: const HomePage(),
    );
  }
}

ThemeData _buildTheme() {
  final scheme = ColorScheme(
    brightness: Brightness.light,
    primary: InkPalette.cinnabar,
    onPrimary: InkPalette.paperHi,
    primaryContainer: const Color(0xFFFFDAD5),
    onPrimaryContainer: const Color(0xFF410001),
    secondary: InkPalette.teal,
    onSecondary: InkPalette.paperHi,
    secondaryContainer: const Color(0xFFCBE8E2),
    onSecondaryContainer: const Color(0xFF002021),
    tertiary: InkPalette.gold,
    onTertiary: InkPalette.paperHi,
    tertiaryContainer: const Color(0xFFFFDDB4),
    onTertiaryContainer: const Color(0xFF271900),
    error: const Color(0xFFBA1A1A),
    onError: Colors.white,
    errorContainer: const Color(0xFFFFDAD6),
    onErrorContainer: const Color(0xFF410002),
    surface: InkPalette.paperHi,
    onSurface: InkPalette.ink,
    surfaceContainerHighest: InkPalette.paperLo,
    onSurfaceVariant: InkPalette.ink3,
    outline: InkPalette.line,
    outlineVariant: InkPalette.paperEdge,
    shadow: const Color(0xFF000000),
    scrim: const Color(0xFF000000),
    inverseSurface: InkPalette.ink2,
    onInverseSurface: InkPalette.paperHi,
    inversePrimary: const Color(0xFFFFB4AB),
    surfaceTint: InkPalette.cinnabar,
  );

  return ThemeData(
    useMaterial3: true,
    colorScheme: scheme,
    scaffoldBackgroundColor: InkPalette.paper,
    pageTransitionsTheme: const PageTransitionsTheme(
      builders: {
        TargetPlatform.android: _InkPageTransitionBuilder(),
        TargetPlatform.iOS:     _InkPageTransitionBuilder(),
      },
    ),
    appBarTheme: AppBarTheme(
      backgroundColor: InkPalette.paperHi,
      foregroundColor: InkPalette.ink,
      elevation: 0,
      scrolledUnderElevation: 1,
      shadowColor: InkPalette.line,
      centerTitle: false,
      titleTextStyle: const TextStyle(
        fontSize: 16,
        fontWeight: FontWeight.w600,
        color: InkPalette.ink,
        letterSpacing: 0.4,
      ),
      systemOverlayStyle: const SystemUiOverlayStyle(
        statusBarColor: Colors.transparent,
        statusBarIconBrightness: Brightness.dark,
        statusBarBrightness: Brightness.light,
      ),
    ),
    navigationBarTheme: NavigationBarThemeData(
      backgroundColor: InkPalette.paperHi,
      indicatorColor: InkPalette.cinnabarWash,
      iconTheme: WidgetStateProperty.resolveWith((states) {
        if (states.contains(WidgetState.selected)) {
          return const IconThemeData(color: InkPalette.cinnabar, size: 22);
        }
        return const IconThemeData(color: InkPalette.ink4, size: 22);
      }),
      labelTextStyle: WidgetStateProperty.resolveWith((states) {
        if (states.contains(WidgetState.selected)) {
          return const TextStyle(fontSize: 11, fontWeight: FontWeight.w600,
              color: InkPalette.cinnabar);
        }
        return const TextStyle(fontSize: 11, color: InkPalette.ink4);
      }),
      elevation: 0,
      surfaceTintColor: Colors.transparent,
    ),
    cardTheme: CardThemeData(
      elevation: 0,
      color: InkPalette.paperHi,
      margin: EdgeInsets.zero,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(12),
        side: const BorderSide(color: InkPalette.line, width: 0.8),
      ),
    ),
    dividerTheme: const DividerThemeData(
      color: InkPalette.line, thickness: 0.8, space: 0,
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: InkPalette.paperHi,
      isDense: true,
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(10),
        borderSide: const BorderSide(color: InkPalette.line),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(10),
        borderSide: const BorderSide(color: InkPalette.line),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(10),
        borderSide: const BorderSide(color: InkPalette.cinnabar, width: 1.6),
      ),
      labelStyle: const TextStyle(color: InkPalette.ink3, fontSize: 13.5),
      hintStyle: const TextStyle(color: InkPalette.inkGhost, fontSize: 13.5),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        backgroundColor: InkPalette.cinnabar,
        foregroundColor: InkPalette.paperHi,
        padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 11),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
        textStyle: const TextStyle(
          fontSize: 14, fontWeight: FontWeight.w600, letterSpacing: 0.3),
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        foregroundColor: InkPalette.cinnabar,
        side: const BorderSide(color: InkPalette.cinnabar),
        padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 11),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
      ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(
        foregroundColor: InkPalette.cinnabar,
      ),
    ),
    snackBarTheme: SnackBarThemeData(
      behavior: SnackBarBehavior.floating,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
    ),
  );
}

// 水墨淡入过渡动画
class _InkPageTransitionBuilder extends PageTransitionsBuilder {
  const _InkPageTransitionBuilder();

  @override
  Widget buildTransitions<T>(
    PageRoute<T> route, BuildContext context,
    Animation<double> animation, Animation<double> secondaryAnimation,
    Widget child,
  ) {
    if (Motion.reduced(context)) return child;
    final curved = CurvedAnimation(
      parent: animation, curve: Motion.standard, reverseCurve: Motion.smooth,
    );
    return FadeTransition(
      opacity: curved,
      child: SlideTransition(
        position: Tween<Offset>(
          begin: const Offset(0, 0.028), end: Offset.zero,
        ).animate(curved),
        child: child,
      ),
    );
  }
}

// ── 首页 ─────────────────────────────────────────────────────────

class HomePage extends StatefulWidget {
  const HomePage({super.key});
  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  AiProvider? _provider;
  int _index = 0;
  int _direction = 1;
  bool _loaded = false;

  @override
  void initState() {
    super.initState();
    LocalStorage.instance.loadProvider().then((p) {
      if (!mounted) return;
      setState(() { _provider = p; _loaded = true; });
    });
  }

  void _onProviderChanged(AiProvider p) {
    setState(() => _provider = p);
  }

  void _selectTab(int i) {
    if (i == _index) return;
    setState(() { _direction = i > _index ? 1 : -1; _index = i; });
  }

  static const _titles = ['创作', '章节', '记忆', '快照', '设置'];

  @override
  Widget build(BuildContext context) {
    if (!_loaded) {
      return Scaffold(
        backgroundColor: InkPalette.paper,
        body: Center(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Container(
                width: 72, height: 72,
                decoration: BoxDecoration(
                  color: InkPalette.cinnabar,
                  borderRadius: BorderRadius.circular(18),
                ),
                alignment: Alignment.center,
                child: const Text('墨',
                  style: TextStyle(
                    fontSize: 36, fontWeight: FontWeight.w700,
                    color: InkPalette.paperHi, letterSpacing: 2,
                  ),
                ),
              ),
              const SizedBox(height: 24),
              const SizedBox(width: 24, height: 24,
                child: CircularProgressIndicator(
                  strokeWidth: 2,
                  valueColor: AlwaysStoppedAnimation<Color>(InkPalette.cinnabar),
                ),
              ),
            ],
          ),
        ),
      );
    }

    final screens = <Widget>[
      StudioScreen(
        key: ValueKey(_provider?.apiKey ?? ''),
        provider: _provider,
        onGoSettings: () => _selectTab(4),
      ),
      const ChaptersScreen(),
      const MemoryScreen(),
      const CheckpointScreen(),
      SettingsScreen(
        provider: _provider,
        onProviderChanged: _onProviderChanged,
      ),
    ];

    return Scaffold(
      backgroundColor: InkPalette.paper,
      appBar: AppBar(
        backgroundColor: InkPalette.paperHi,
        title: AnimatedSwitcher(
          duration: Motion.dur(context, Motion.normal),
          switchInCurve: Motion.standard,
          switchOutCurve: Motion.smooth,
          transitionBuilder: (child, anim) => FadeTransition(
            opacity: anim,
            child: SlideTransition(
              position: Tween<Offset>(
                begin: const Offset(0, 0.25), end: Offset.zero,
              ).animate(anim),
              child: child,
            ),
          ),
          child: Row(
            key: ValueKey<int>(_index),
            mainAxisSize: MainAxisSize.min,
            children: [
              Container(
                width: 26, height: 26,
                decoration: BoxDecoration(
                  color: InkPalette.cinnabar,
                  borderRadius: BorderRadius.circular(6),
                ),
                alignment: Alignment.center,
                child: const Text('墨',
                  style: TextStyle(
                    fontSize: 13, fontWeight: FontWeight.w700,
                    color: InkPalette.paperHi,
                  ),
                ),
              ),
              const SizedBox(width: 8),
              Text(
                '墨·创作  ·  ${_titles[_index]}',
                style: const TextStyle(
                  fontSize: 15.5, fontWeight: FontWeight.w600,
                  color: InkPalette.ink2, letterSpacing: 0.4,
                ),
              ),
            ],
          ),
        ),
        bottom: PreferredSize(
          preferredSize: const Size.fromHeight(0.8),
          child: Container(height: 0.8, color: InkPalette.line),
        ),
      ),
      body: AnimatedSwitcher(
        duration: Motion.dur(context, Motion.normal),
        switchInCurve: Motion.standard,
        switchOutCurve: Motion.smooth,
        transitionBuilder: (child, anim) {
          final isIncoming = child.key == ValueKey<int>(_index);
          final dx = isIncoming ? 0.05 * _direction : 0.0;
          return FadeTransition(
            opacity: anim,
            child: SlideTransition(
              position: Tween<Offset>(
                begin: Offset(dx, 0), end: Offset.zero,
              ).animate(anim),
              child: child,
            ),
          );
        },
        layoutBuilder: (currentChild, previousChildren) => Stack(
          alignment: Alignment.topCenter,
          children: [...previousChildren, ?currentChild],
        ),
        child: KeyedSubtree(
          key: ValueKey<int>(_index), child: screens[_index],
        ),
      ),
      bottomNavigationBar: Container(
        decoration: const BoxDecoration(
          border: Border(top: BorderSide(color: InkPalette.line, width: 0.8)),
        ),
        child: NavigationBar(
          selectedIndex: _index,
          onDestinationSelected: _selectTab,
          animationDuration: Motion.normal,
          height: 62,
          backgroundColor: InkPalette.paperHi,
          destinations: const [
            NavigationDestination(
              icon: Icon(Icons.edit_outlined),
              selectedIcon: Icon(Icons.edit_rounded),
              label: '创作',
            ),
            NavigationDestination(
              icon: Icon(Icons.article_outlined),
              selectedIcon: Icon(Icons.article_rounded),
              label: '章节',
            ),
            NavigationDestination(
              icon: Icon(Icons.psychology_outlined),
              selectedIcon: Icon(Icons.psychology_rounded),
              label: '记忆',
            ),
            NavigationDestination(
              icon: Icon(Icons.bookmark_border_rounded),
              selectedIcon: Icon(Icons.bookmark_rounded),
              label: '快照',
            ),
            NavigationDestination(
              icon: Icon(Icons.settings_outlined),
              selectedIcon: Icon(Icons.settings_rounded),
              label: '设置',
            ),
          ],
        ),
      ),
    );
  }
}
