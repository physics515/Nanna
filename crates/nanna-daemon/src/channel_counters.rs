//! Per-channel message counters for `/metrics`.
//!
//! Every channel message went in and out with no count anywhere, so "is the
//! Telegram bot answering at all" had no answer short of reading logs.
//! Counted at the two places a channel message crosses the daemon boundary:
//! `ChannelManager::process_message` (inbound, and immediate replies) and the
//! reply forwarder (turn answers and reminders).

use std::collections::BTreeMap;
use std::sync::Mutex;

/// Most distinct channel names counted before the rest fold into `other`.
///
/// Channel names are provider identifiers the daemon registers (telegram,
/// discord, slack, signal, whatsapp, generic) — not user input — so this is a
/// backstop against a future caller passing something unbounded, sized well
/// above the real set.
pub const CHANNELS_COUNTED_MAX: usize = 16;

const OTHER_CHANNEL: &str = "other";

/// Counts for one channel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelCount {
    pub received: u64,
    pub sent: u64,
    pub send_failures: u64,
}

/// Counters for every channel, safe to share.
#[derive(Debug, Default)]
pub struct ChannelCounters {
    counts: Mutex<BTreeMap<String, ChannelCount>>,
}

impl ChannelCounters {
    fn bump(&self, channel: &str, apply: impl FnOnce(&mut ChannelCount)) {
        let mut counts = self
            .counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = if counts.contains_key(channel) || counts.len() < CHANNELS_COUNTED_MAX {
            channel
        } else {
            OTHER_CHANNEL
        };
        apply(counts.entry(key.to_string()).or_default());
        debug_assert!(counts.len() <= CHANNELS_COUNTED_MAX + 1);
        drop(counts);
    }

    /// A message arrived from `channel`.
    pub fn received(&self, channel: &str) {
        self.bump(channel, |c| c.received = c.received.saturating_add(1));
    }

    /// A reply was sent to `channel`.
    pub fn sent(&self, channel: &str) {
        self.bump(channel, |c| c.sent = c.sent.saturating_add(1));
    }

    /// A reply to `channel` could not be sent.
    pub fn send_failed(&self, channel: &str) {
        self.bump(channel, |c| {
            c.send_failures = c.send_failures.saturating_add(1);
        });
    }

    /// Every channel's counts, by name.
    #[must_use]
    pub fn snapshot(&self) -> Vec<(String, ChannelCount)> {
        self.counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|(name, count)| (name.clone(), *count))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_accumulate_per_channel() {
        let counters = ChannelCounters::default();
        counters.received("telegram");
        counters.received("telegram");
        counters.sent("telegram");
        counters.send_failed("slack");
        let snapshot = counters.snapshot();
        assert_eq!(
            snapshot,
            vec![
                (
                    "slack".to_string(),
                    ChannelCount {
                        received: 0,
                        sent: 0,
                        send_failures: 1
                    }
                ),
                (
                    "telegram".to_string(),
                    ChannelCount {
                        received: 2,
                        sent: 1,
                        send_failures: 0
                    }
                ),
            ]
        );
    }

    #[test]
    fn channels_past_the_limit_fold_into_other() {
        let counters = ChannelCounters::default();
        for index in 0..CHANNELS_COUNTED_MAX + 5 {
            counters.received(&format!("c{index}"));
        }
        let snapshot = counters.snapshot();
        assert_eq!(snapshot.len(), CHANNELS_COUNTED_MAX + 1);
        let other = snapshot
            .iter()
            .find(|(name, _)| name == "other")
            .expect("other");
        assert_eq!(other.1.received, 5);
        // A channel counted before the limit keeps its own series.
        counters.received("c0");
        assert_eq!(
            counters
                .snapshot()
                .iter()
                .find(|(n, _)| n == "c0")
                .expect("c0")
                .1
                .received,
            2
        );
    }
}
