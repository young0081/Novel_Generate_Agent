import 'package:flutter/material.dart';

import 'motion.dart';

/// A titled section card used across the screens.
///
/// [trailing] renders an optional widget aligned to the right of the title
/// (e.g. a small icon button). [subtitle] adds a muted line below the title.
Widget sectionCard(
  String title,
  List<Widget> children, {
  IconData? icon,
  String? subtitle,
  Widget? trailing,
}) {
  return Card(
    margin: const EdgeInsets.only(bottom: 16),
    clipBehavior: Clip.antiAlias,
    child: Padding(
      padding: const EdgeInsets.fromLTRB(18, 16, 18, 18),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              if (icon != null) ...[
                Icon(icon, size: 20),
                const SizedBox(width: 8),
              ],
              Expanded(
                child: Text(
                  title,
                  style: const TextStyle(
                    fontSize: 15.5,
                    fontWeight: FontWeight.w700,
                  ),
                ),
              ),
              ?trailing,
            ],
          ),
          if (subtitle != null) ...[
            const SizedBox(height: 4),
            Builder(
              builder: (context) => Text(
                subtitle,
                style: TextStyle(
                  fontSize: 12.5,
                  height: 1.4,
                  color: Theme.of(context).colorScheme.onSurfaceVariant,
                ),
              ),
            ),
          ],
          const SizedBox(height: 14),
          ...children,
        ],
      ),
    ),
  );
}

/// A monospaced, selectable output box for command results.
///
/// New results cross-fade in over the previous text (keyed on [text]) so a
/// refreshed listing or recall result reads as a deliberate update rather than
/// a flicker. Honours reduced motion via [CrossFadeSwitcher].
Widget outputBox(String text) {
  return Builder(
    builder: (context) {
      final scheme = Theme.of(context).colorScheme;
      return CrossFadeSwitcher(
        child: Container(
          key: ValueKey<String>(text),
          margin: const EdgeInsets.only(top: 14),
          padding: const EdgeInsets.all(14),
          width: double.infinity,
          decoration: BoxDecoration(
            color: const Color(0xFF0B0D11),
            border: Border.all(color: scheme.outlineVariant),
            borderRadius: BorderRadius.circular(10),
          ),
          child: SelectableText(
            text,
            style: const TextStyle(
              fontFamily: 'monospace',
              fontSize: 12.5,
              height: 1.55,
            ),
          ),
        ),
      );
    },
  );
}

/// A button label that swaps in a small spinner while [busy].
///
/// Sized so the button keeps its width when toggling between the label and
/// the spinner. Use as the `child:` of a FilledButton / OutlinedButton.
class BusyLabel extends StatelessWidget {
  final bool busy;
  final String label;
  final IconData? icon;

  const BusyLabel({
    super.key,
    required this.busy,
    required this.label,
    this.icon,
  });

  @override
  Widget build(BuildContext context) {
    final Widget content;
    if (busy) {
      content = const SizedBox(
        key: ValueKey('busy'),
        height: 18,
        width: 18,
        child: CircularProgressIndicator(strokeWidth: 2),
      );
    } else if (icon == null) {
      content = Text(label, key: const ValueKey('label'));
    } else {
      content = Row(
        key: const ValueKey('label-icon'),
        mainAxisSize: MainAxisSize.min,
        children: [Icon(icon, size: 18), const SizedBox(width: 8), Text(label)],
      );
    }
    // Cross-fade between states so the spinner doesn't pop in/out abruptly.
    return CrossFadeSwitcher(duration: Motion.fast, slideY: 0, child: content);
  }
}

/// An icon for `FilledButton.icon` that cross-fades to a small spinner while
/// [busy]. Keeps a constant footprint so the button doesn't jump on toggle.
class BusyIcon extends StatelessWidget {
  final bool busy;
  final IconData icon;

  const BusyIcon({super.key, required this.busy, required this.icon});

  @override
  Widget build(BuildContext context) {
    return CrossFadeSwitcher(
      duration: Motion.fast,
      slideY: 0,
      child: busy
          ? const SizedBox(
              key: ValueKey('spin'),
              height: 16,
              width: 16,
              child: CircularProgressIndicator(strokeWidth: 2),
            )
          : Icon(icon, key: const ValueKey('icon')),
    );
  }
}

/// A friendly placeholder for empty lists / before-first-load states.
///
/// Fades + lifts into view on appearance, and the icon breathes gently
/// (slow scale loop) so empty space feels alive rather than broken. Both
/// effects are suppressed under reduced motion.
class EmptyState extends StatelessWidget {
  final IconData icon;
  final String message;
  final String? hint;

  const EmptyState({
    super.key,
    required this.icon,
    required this.message,
    this.hint,
  });

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return TweenAnimationBuilder<double>(
      tween: Tween(begin: 0, end: 1),
      duration: Motion.dur(context, Motion.slow),
      curve: Motion.standard,
      builder: (context, t, child) => Opacity(
        opacity: t,
        child: Transform.translate(
          offset: Offset(0, (1 - t) * 12),
          child: child,
        ),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 40, horizontal: 24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            _BreathingIcon(icon: icon, color: scheme.onSurfaceVariant),
            const SizedBox(height: 12),
            Text(
              message,
              textAlign: TextAlign.center,
              style: TextStyle(
                fontSize: 14,
                fontWeight: FontWeight.w600,
                color: scheme.onSurfaceVariant,
              ),
            ),
            if (hint != null) ...[
              const SizedBox(height: 6),
              Text(
                hint!,
                textAlign: TextAlign.center,
                style: TextStyle(
                  fontSize: 12.5,
                  color: scheme.onSurfaceVariant,
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

/// A slowly pulsing icon used inside [EmptyState]. The scale loop is subtle
/// (about 4% travel) and stops at rest under reduced motion.
class _BreathingIcon extends StatefulWidget {
  final IconData icon;
  final Color color;

  const _BreathingIcon({required this.icon, required this.color});

  @override
  State<_BreathingIcon> createState() => _BreathingIconState();
}

class _BreathingIconState extends State<_BreathingIcon>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 2600),
  );
  late final Animation<double> _scale = Tween<double>(
    begin: 0.96,
    end: 1.04,
  ).animate(CurvedAnimation(parent: _controller, curve: Curves.easeInOut));

  @override
  void initState() {
    super.initState();
    _controller.repeat(reverse: true);
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final icon = Icon(widget.icon, size: 44, color: widget.color);
    if (Motion.reduced(context)) return icon;
    return ScaleTransition(scale: _scale, child: icon);
  }
}

/// A centered error placeholder with an optional retry button.
class ErrorState extends StatelessWidget {
  final String message;
  final Future<void> Function()? onRetry;

  const ErrorState({super.key, required this.message, this.onRetry});

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Center(
      child: TweenAnimationBuilder<double>(
        tween: Tween(begin: 0, end: 1),
        duration: Motion.dur(context, Motion.slow),
        curve: Motion.standard,
        builder: (context, t, child) => Opacity(
          opacity: t,
          child: Transform.scale(scale: 0.94 + 0.06 * t, child: child),
        ),
        child: Padding(
          padding: const EdgeInsets.all(28),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(Icons.cloud_off_rounded, size: 52, color: scheme.error),
              const SizedBox(height: 14),
              Text(
                message,
                textAlign: TextAlign.center,
                style: const TextStyle(fontSize: 14, height: 1.5),
              ),
              if (onRetry != null) ...[
                const SizedBox(height: 18),
                FilledButton.icon(
                  onPressed: onRetry,
                  icon: const Icon(Icons.refresh),
                  label: const Text('重试'),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

/// Shows a green-tinted success SnackBar. Safe no-op if [context] is unmounted.
void showSuccessSnack(BuildContext context, String message) {
  if (!context.mounted) return;
  _showSnack(
    context,
    message,
    icon: Icons.check_circle_rounded,
    background: const Color(0xFF1E7B52),
  );
}

/// Shows an error-colored SnackBar. Safe no-op if [context] is unmounted.
void showErrorSnack(BuildContext context, String message) {
  if (!context.mounted) return;
  final scheme = Theme.of(context).colorScheme;
  _showSnack(
    context,
    message,
    icon: Icons.error_rounded,
    background: scheme.error,
    foreground: scheme.onError,
  );
}

void _showSnack(
  BuildContext context,
  String message, {
  required IconData icon,
  required Color background,
  Color foreground = Colors.white,
}) {
  final messenger = ScaffoldMessenger.of(context);
  messenger.hideCurrentSnackBar();
  messenger.showSnackBar(
    SnackBar(
      behavior: SnackBarBehavior.floating,
      backgroundColor: background,
      duration: const Duration(seconds: 3),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
      content: Row(
        children: [
          _SnackIcon(icon: icon, color: foreground),
          const SizedBox(width: 10),
          Expanded(
            child: Text(message, style: TextStyle(color: foreground)),
          ),
        ],
      ),
    ),
  );
}

/// The leading SnackBar icon, popping in with a brief scale + spin so the
/// toast lands with a little personality. Static under reduced motion.
class _SnackIcon extends StatelessWidget {
  final IconData icon;
  final Color color;

  const _SnackIcon({required this.icon, required this.color});

  @override
  Widget build(BuildContext context) {
    final glyph = Icon(icon, color: color, size: 20);
    if (Motion.reduced(context)) return glyph;
    return TweenAnimationBuilder<double>(
      tween: Tween(begin: 0, end: 1),
      duration: Motion.slow,
      curve: Curves.easeOutBack,
      builder: (context, t, child) => Transform.rotate(
        angle: (1 - t) * -0.5,
        child: Transform.scale(scale: 0.4 + 0.6 * t, child: child),
      ),
      child: glyph,
    );
  }
}
