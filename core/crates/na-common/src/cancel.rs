//! Hierarchical cancellation token.
//!
//! Powers the "user can cancel / interrupt at any time" requirement. A token can
//! be checked synchronously (`is_cancelled`) between agent-loop steps, awaited
//! (`cancelled().await`) inside a `tokio::select!` while a tool is running, and
//! arranged into a tree so cancelling a session cancels every in-flight tool.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use crate::error::{CoreError, Result};

struct Inner {
    cancelled: AtomicBool,
    notify: Notify,
    children: Mutex<Vec<Arc<Inner>>>,
}

/// A cheaply-cloneable handle to a cancellation signal.
///
/// Clones share the same signal. [`CancellationToken::child`] creates a linked
/// token that is cancelled automatically when its parent is.
#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<Inner>,
}

impl CancellationToken {
    /// Create a fresh, un-cancelled root token.
    pub fn new() -> Self {
        CancellationToken {
            inner: Arc::new(Inner {
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
                children: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Signal cancellation. Idempotent. Wakes every waiter and cancels all
    /// descendants.
    pub fn cancel(&self) {
        cancel_inner(&self.inner);
    }

    /// Whether this token (or any ancestor that cancelled it) is cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// Return `Err(Cancelled)` if cancelled — handy as a guard at step
    /// boundaries: `token.check()?;`.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(CoreError::cancelled("operation cancelled by user"))
        } else {
            Ok(())
        }
    }

    /// Resolve as soon as the token is cancelled. Safe to use inside
    /// `tokio::select!` against the real work.
    pub async fn cancelled(&self) {
        loop {
            if self.inner.cancelled.load(Ordering::SeqCst) {
                return;
            }
            // Register the waiter *before* the final check to avoid a lost wakeup.
            let notified = self.inner.notify.notified();
            if self.inner.cancelled.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }

    /// Create a child token. Cancelling the parent cancels the child (and its
    /// own children), but cancelling the child leaves the parent alone.
    pub fn child(&self) -> CancellationToken {
        let child = Arc::new(Inner {
            cancelled: AtomicBool::new(false),
            notify: Notify::new(),
            children: Mutex::new(Vec::new()),
        });
        {
            let mut guard = self
                .inner
                .children
                .lock()
                .expect("cancel children poisoned");
            guard.push(child.clone());
        }
        // If the parent was already cancelled, propagate immediately.
        if self.is_cancelled() {
            cancel_inner(&child);
        }
        CancellationToken { inner: child }
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

fn cancel_inner(inner: &Arc<Inner>) {
    // Only fan out the first time the flag flips false -> true.
    if !inner.cancelled.swap(true, Ordering::SeqCst) {
        inner.notify.notify_waiters();
        let children = {
            let guard = inner.children.lock().expect("cancel children poisoned");
            guard.clone()
        };
        for child in &children {
            cancel_inner(child);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_cancel() {
        let t = CancellationToken::new();
        assert!(!t.is_cancelled());
        assert!(t.check().is_ok());
        t.cancel();
        assert!(t.is_cancelled());
        assert!(t.check().is_err());
    }

    #[test]
    fn clones_share_signal() {
        let a = CancellationToken::new();
        let b = a.clone();
        a.cancel();
        assert!(b.is_cancelled());
    }

    #[test]
    fn child_cancelled_by_parent() {
        let parent = CancellationToken::new();
        let child = parent.child();
        let grandchild = child.child();
        assert!(!child.is_cancelled());
        parent.cancel();
        assert!(child.is_cancelled());
        assert!(grandchild.is_cancelled());
    }

    #[test]
    fn child_cancel_does_not_affect_parent() {
        let parent = CancellationToken::new();
        let child = parent.child();
        child.cancel();
        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled());
    }

    #[test]
    fn child_of_cancelled_parent_starts_cancelled() {
        let parent = CancellationToken::new();
        parent.cancel();
        let child = parent.child();
        assert!(child.is_cancelled());
    }

    #[tokio::test]
    async fn await_resolves_on_cancel() {
        let t = CancellationToken::new();
        let t2 = t.clone();
        let handle = tokio::spawn(async move {
            t2.cancelled().await;
            42
        });
        // Give the task a moment to start waiting, then cancel.
        tokio::task::yield_now().await;
        t.cancel();
        let v = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .expect("did not resolve in time")
            .unwrap();
        assert_eq!(v, 42);
    }

    #[tokio::test]
    async fn await_returns_immediately_if_already_cancelled() {
        let t = CancellationToken::new();
        t.cancel();
        // Should not hang.
        tokio::time::timeout(std::time::Duration::from_secs(1), t.cancelled())
            .await
            .expect("should be immediate");
    }
}
