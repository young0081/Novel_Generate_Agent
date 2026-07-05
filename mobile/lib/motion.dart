import 'package:flutter/material.dart';

/// Centralised motion language for the app.
///
/// Everything animation-related funnels through here so durations and easing
/// stay consistent across the five screens. All helpers honour the platform
/// "reduce motion" accessibility setting (`MediaQuery.disableAnimations`):
/// when it is on, durations collapse to zero and transforms snap to their
/// resting state, so the UI stays usable for motion-sensitive users.
class Motion {
  Motion._();

  // ---- Durations (a deliberately small, reusable set) --------------------
  /// Tiny taps: button press / ripple settle.
  static const Duration fast = Duration(milliseconds: 140);

  /// Standard UI transitions: cards revealing, switchers, nav.
  static const Duration normal = Duration(milliseconds: 260);

  /// Larger / page-level transitions.
  static const Duration slow = Duration(milliseconds: 360);

  /// Base delay between consecutive items in a staggered list.
  static const Duration stagger = Duration(milliseconds: 55);

  /// Cap on cumulative stagger delay so long lists never feel laggy.
  static const Duration staggerCap = Duration(milliseconds: 360);

  // ---- Curves ------------------------------------------------------------
  /// The everyday easing — gentle, emphasised deceleration.
  static const Curve standard = Curves.easeOutCubic;

  /// Slightly springy settle for press-release feedback.
  static const Curve emphasized = Curves.easeOutBack;

  /// Symmetric ease for things that move both directions (e.g. switchers).
  static const Curve smooth = Curves.easeInOutCubic;

  /// Whether the user asked the OS to minimise non-essential motion.
  static bool reduced(BuildContext context) =>
      MediaQuery.maybeDisableAnimationsOf(context) ?? false;

  /// Collapses [d] to [Duration.zero] when reduced motion is requested.
  static Duration dur(BuildContext context, Duration d) =>
      reduced(context) ? Duration.zero : d;
}

/// A page route that fades + gently slides its child in from below.
///
/// Used for any pushed routes so navigation feels of-a-piece. Falls back to a
/// plain (instant) transition when reduced motion is on.
class FadeThroughPageRoute<T> extends PageRouteBuilder<T> {
  FadeThroughPageRoute({required WidgetBuilder builder, super.settings})
    : super(
        transitionDuration: Motion.normal,
        reverseTransitionDuration: Motion.fast,
        pageBuilder: (context, animation, secondaryAnimation) =>
            builder(context),
        transitionsBuilder: (context, animation, secondaryAnimation, child) {
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
        },
      );
}

/// Fades + lifts [child] into place once, on first build.
///
/// Give consecutive items an increasing [index] to get a staggered cascade.
/// The cumulative delay is capped (see [Motion.staggerCap]) so the last item
/// in a long list still appears promptly. Honours reduced motion.
class StaggeredEntrance extends StatefulWidget {
  final int index;
  final Widget child;

  /// Vertical travel distance (logical px) the child rises through.
  final double offsetY;

  const StaggeredEntrance({
    super.key,
    required this.child,
    this.index = 0,
    this.offsetY = 16,
  });

  @override
  State<StaggeredEntrance> createState() => _StaggeredEntranceState();
}

class _StaggeredEntranceState extends State<StaggeredEntrance>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller = AnimationController(
    vsync: this,
    duration: Motion.normal,
  );
  late final Animation<double> _curved = CurvedAnimation(
    parent: _controller,
    curve: Motion.standard,
  );

  @override
  void initState() {
    super.initState();
    _kick();
  }

  void _kick() {
    final capMs = Motion.staggerCap.inMilliseconds;
    final wantMs = Motion.stagger.inMilliseconds * widget.index;
    final delay = Duration(milliseconds: wantMs.clamp(0, capMs));
    if (delay == Duration.zero) {
      _controller.forward();
    } else {
      Future<void>.delayed(delay, () {
        if (mounted) _controller.forward();
      });
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    if (Motion.reduced(context)) return widget.child;
    return AnimatedBuilder(
      animation: _curved,
      builder: (context, child) {
        return Opacity(
          opacity: _curved.value,
          child: Transform.translate(
            offset: Offset(0, (1 - _curved.value) * widget.offsetY),
            child: child,
          ),
        );
      },
      child: widget.child,
    );
  }
}

/// Wraps a tappable widget so it scales down slightly while pressed, giving
/// physical "give" to taps. Pair with the widget's own InkWell ripple.
///
/// When [onTap] is null the child is shown at rest (no gesture handling), so
/// it can stand in for disabled controls. Honours reduced motion.
class PressableScale extends StatefulWidget {
  final Widget child;
  final VoidCallback? onTap;

  /// Scale applied at the bottom of the press.
  final double pressedScale;
  final BorderRadius? borderRadius;

  const PressableScale({
    super.key,
    required this.child,
    this.onTap,
    this.pressedScale = 0.96,
    this.borderRadius,
  });

  @override
  State<PressableScale> createState() => _PressableScaleState();
}

class _PressableScaleState extends State<PressableScale> {
  bool _down = false;

  void _set(bool v) {
    if (_down != v) setState(() => _down = v);
  }

  @override
  Widget build(BuildContext context) {
    final enabled = widget.onTap != null;
    final reduced = Motion.reduced(context);
    final scale = (!enabled || reduced || !_down) ? 1.0 : widget.pressedScale;
    return GestureDetector(
      onTapDown: enabled ? (_) => _set(true) : null,
      onTapUp: enabled ? (_) => _set(false) : null,
      onTapCancel: enabled ? () => _set(false) : null,
      onTap: widget.onTap,
      child: AnimatedScale(
        scale: scale,
        duration: Motion.fast,
        curve: Motion.emphasized,
        child: widget.child,
      ),
    );
  }
}

/// A drop-in [AnimatedSwitcher] preconfigured with the app's fade + subtle
/// vertical slide. Use for result panels, status rows, loading/loaded swaps.
///
/// Children that should be treated as distinct transitions must carry
/// different [Key]s (the usual AnimatedSwitcher contract).
class CrossFadeSwitcher extends StatelessWidget {
  final Widget child;
  final Duration? duration;

  /// Slide distance for the incoming child. 0 = pure cross-fade.
  final double slideY;

  const CrossFadeSwitcher({
    super.key,
    required this.child,
    this.duration,
    this.slideY = 0.04,
  });

  @override
  Widget build(BuildContext context) {
    return AnimatedSwitcher(
      duration: Motion.dur(context, duration ?? Motion.normal),
      switchInCurve: Motion.standard,
      switchOutCurve: Motion.smooth,
      transitionBuilder: (child, animation) {
        if (slideY == 0) {
          return FadeTransition(opacity: animation, child: child);
        }
        return FadeTransition(
          opacity: animation,
          child: SlideTransition(
            position: Tween<Offset>(
              begin: Offset(0, slideY),
              end: Offset.zero,
            ).animate(animation),
            child: child,
          ),
        );
      },
      child: child,
    );
  }
}

/// A lightweight shimmering placeholder bar used to suggest loading content.
///
/// Pure transform/opacity gradient sweep — cheap on the GPU. The animation
/// stops (showing a flat block) under reduced motion.
class ShimmerBox extends StatefulWidget {
  final double width;
  final double height;
  final BorderRadius? borderRadius;

  const ShimmerBox({
    super.key,
    this.width = double.infinity,
    this.height = 14,
    this.borderRadius,
  });

  @override
  State<ShimmerBox> createState() => _ShimmerBoxState();
}

class _ShimmerBoxState extends State<ShimmerBox>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 1200),
  );

  @override
  void initState() {
    super.initState();
    _controller.repeat();
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final base = scheme.surfaceContainerHighest.withValues(alpha: 0.35);
    final highlight = scheme.surfaceContainerHighest.withValues(alpha: 0.75);
    final radius = widget.borderRadius ?? BorderRadius.circular(8);

    if (Motion.reduced(context)) {
      return Container(
        width: widget.width,
        height: widget.height,
        decoration: BoxDecoration(color: base, borderRadius: radius),
      );
    }

    return AnimatedBuilder(
      animation: _controller,
      builder: (context, _) {
        final t = _controller.value;
        return ClipRRect(
          borderRadius: radius,
          child: Container(
            width: widget.width,
            height: widget.height,
            decoration: BoxDecoration(
              gradient: LinearGradient(
                begin: Alignment(-1 - 2 * (1 - t), 0),
                end: Alignment(1 - 2 * (1 - t) + 1, 0),
                colors: [base, highlight, base],
                stops: const [0.25, 0.5, 0.75],
              ),
            ),
          ),
        );
      },
    );
  }
}

/// A few stacked [ShimmerBox] lines forming a skeleton card body.
class ShimmerLines extends StatelessWidget {
  final int lines;

  const ShimmerLines({super.key, this.lines = 3});

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        for (int i = 0; i < lines; i++) ...[
          ShimmerBox(width: i.isEven ? double.infinity : 180, height: 13),
          if (i != lines - 1) const SizedBox(height: 10),
        ],
      ],
    );
  }
}
