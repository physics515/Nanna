//! Abortive cancellation for agent runs.
//!
//! The Stop chain used to share a bare `Arc<AtomicBool>` that every layer
//! POLLED — so Stop only took effect when something happened to check it:
//! the next token batch, the next loop iteration, the next step boundary.
//! A model deep in a long silent reasoning stretch produces no token
//! batches, which meant the in-flight HTTP response stayed live for minutes
//! after the user pressed Stop (observed live 2026-07-31).
//!
//! [`CancelToken`] keeps the cheap sync poll (`is_cancelled`) and adds the
//! missing async edge (`cancelled`), so any await point can be raced against
//! cancellation with `tokio::select!` — aborting the in-flight future (and
//! with it the HTTP connection) the moment Stop is pressed instead of
//! waiting for the provider to produce something.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

/// A shared cancellation handle: one `cancel()` call, observable both by
/// polling ([`Self::is_cancelled`]) and by awaiting ([`Self::cancelled`]).
///
/// Clones share the same underlying token. Once cancelled, a token stays
/// cancelled — there is no reset; a new run gets a new token.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    inner: Arc<CancelInner>,
}

#[derive(Debug, Default)]
struct CancelInner {
    flag: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Flip the token. Wakes every task parked in [`Self::cancelled`].
    pub fn cancel(&self) {
        // The store happens BEFORE `notify_waiters`, and `cancelled()`
        // registers its waiter BEFORE re-checking the flag — so a waiter
        // either sees the flag or receives the wakeup, never neither.
        // SeqCst removes any doubt about cross-thread visibility ordering.
        self.inner.flag.store(true, Ordering::SeqCst);
        self.inner.notify.notify_waiters();
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.flag.load(Ordering::SeqCst)
    }

    /// Resolves once the token is cancelled. Level-triggered: returns
    /// immediately when already cancelled, and a `cancel()` racing this
    /// call cannot be lost — `Notified::enable` registers the waiter
    /// before the flag is re-checked.
    pub async fn cancelled(&self) {
        while !self.is_cancelled() {
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            // Register interest first; only then trust a negative check.
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }

    /// Identity comparison: do two handles share one token?
    #[must_use]
    pub fn same_token(a: &Self, b: &Self) -> bool {
        Arc::ptr_eq(&a.inner, &b.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn a_fresh_token_is_not_cancelled() {
        assert!(!CancelToken::new().is_cancelled());
    }

    #[tokio::test]
    async fn clones_share_the_token() {
        let token = CancelToken::new();
        let clone = token.clone();
        clone.cancel();
        assert!(token.is_cancelled());
        assert!(CancelToken::same_token(&token, &clone));
        assert!(!CancelToken::same_token(&token, &CancelToken::new()));
    }

    #[tokio::test]
    async fn cancelled_resolves_immediately_when_already_cancelled() {
        let token = CancelToken::new();
        token.cancel();
        tokio::time::timeout(Duration::from_secs(1), token.cancelled())
            .await
            .expect("an already-cancelled token must resolve without waiting");
    }

    /// The exact shape `call_llm_streaming` uses: a provider stream that
    /// never yields (a model in a long silent reasoning stretch) raced
    /// against the token. The select must return on cancel — promptly, not
    /// at the next token batch, because there is no next token batch.
    #[tokio::test]
    async fn cancel_unblocks_a_select_on_a_never_yielding_stream() {
        use futures::StreamExt;
        let token = CancelToken::new();
        let stop = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            stop.cancel();
        });
        let mut stream = futures::stream::pending::<u8>();
        let event = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! {
                biased;
                () = token.cancelled() => None,
                event = stream.next() => event,
            }
        })
        .await
        .expect("cancel must abort the stalled stream await, not wait for a batch");
        assert!(
            event.is_none(),
            "the cancel arm must win on a silent stream"
        );
    }
}
