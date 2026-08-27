//! FSRS-6 (Free Spaced Repetition Scheduler) implementation
//!
//! Based on the FSRS-6 algorithm: https://github.com/open-spaced-repetition/fsrs4anki
//! Power law forgetting curve optimized on 700M+ Anki reviews.
//!
//! Key concepts:
//! - Stability (S): Time (in days) for retrievability to drop to 90%
//! - Retrievability (R): Probability of successful recall (0-1)
//! - Difficulty (D): Inherent difficulty of the memory (1-10)

use serde::{Deserialize, Serialize};

/// The 21 FSRS weights, in FSRS-6's slot order.
///
/// [`Default`] is **not** FSRS-6's published weight table — only `w20` is (see
/// its field doc). The rest are FSRS-5 values with six slots zeroed, and only
/// `w6..=w12` and `w20` are read at all. Adopting the published table for the
/// others is an open decision (ROADMAP P13), not an oversight: Nanna's
/// stability update is not FSRS's, so transcribing its constants into a
/// differently-shaped formula would be cargo-culting, not correctness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsrsParameters {
    /// Initial stability for first review (w0-w3 for different ratings)
    pub w0: f32, // Again
    pub w1: f32, // Hard
    pub w2: f32, // Good
    pub w3: f32, // Easy
    
    /// Difficulty parameters
    pub w4: f32, // Initial difficulty mean
    pub w5: f32, // Initial difficulty variance
    pub w6: f32, // Difficulty change on failure
    pub w7: f32, // Difficulty change on success
    
    /// Stability parameters
    pub w8: f32,  // Stability increase factor
    pub w9: f32,  // Stability decrease on failure
    pub w10: f32, // Stability modifier
    pub w11: f32, // Stability hard penalty
    pub w12: f32, // Stability easy bonus
    pub w13: f32, // Stability relearn factor
    pub w14: f32, // Hard interval factor
    pub w15: f32, // Easy interval factor
    
    /// Forgetting curve parameters
    pub w16: f32, // Short-term stability factor
    pub w17: f32, // Long-term stability factor
    pub w18: f32, // Forgetting curve decay
    pub w19: f32, // Reserved
    /// Forgetting-curve decay exponent — the only weight in this struct that
    /// feeds a formula shaped exactly like FSRS's own, so it is the only one
    /// whose published default transfers directly.
    ///
    /// Defaults to [`FSRS6_DEFAULT_DECAY`] (`0.1542`).
    ///
    /// Two corrections are folded into that number, in order:
    /// 1. It was `0.5` — FSRS-4.5/5's hardcoded `DECAY` — paired with the
    ///    FSRS-6 curve, which decayed retrievability far too fast and made
    ///    aged-but-valid memories fall below the recall weight gate and vanish.
    /// 2. The fix for (1) took `0.0658`, which is **`w[19]` of the FSRS-6
    ///    parameter vector, not `w[20]`** — an off-by-one into the published
    ///    array. `fsrs-rs` names the real value `FSRS6_DEFAULT_DECAY = 0.1542`
    ///    and clamps this parameter to [`DECAY_MIN`]`..=`[`DECAY_MAX`]
    ///    (`0.1..=0.8`), a range `0.0658` sits **below** — so the optimizer
    ///    could never have produced it.
    ///
    /// Both flips are gated on the retention harness ([`crate::retention`]);
    /// its `w20` experiment measures aged recall at all three exponents so the
    /// correction is shown not to reintroduce (1). Making this exponent
    /// *trainable* (fit from access history) is the eventual goal — ROADMAP P13.
    pub w20: f32,
}

/// FSRS-6's published forgetting-curve decay, matching `fsrs-rs`'s
/// `FSRS6_DEFAULT_DECAY` — the last element of its 21-value `DEFAULT_PARAMETERS`.
pub const FSRS6_DEFAULT_DECAY: f32 = 0.1542;

/// FSRS-5's hardcoded decay, kept named so the comparison in
/// [`crate::retention`] reads as the documented constant it is rather than a
/// magic number.
pub const FSRS5_DEFAULT_DECAY: f32 = 0.5;

/// Lower clamp `fsrs-rs`'s optimizer applies to the decay exponent. A default
/// outside this range cannot be a fitted FSRS parameter.
pub const DECAY_MIN: f32 = 0.1;

/// Upper clamp `fsrs-rs`'s optimizer applies to the decay exponent.
pub const DECAY_MAX: f32 = 0.8;

impl Default for FsrsParameters {
    fn default() -> Self {
        // NOT the FSRS-6 weight table, despite the module heading — `w0..w18`
        // are FSRS-5 values and six entries are zeroed, matching neither
        // published table. Left as-is on purpose: only `w20` feeds a formula
        // shaped like FSRS's own, so only `w20`'s published default transfers
        // without an A/B on the retention harness. Adopting the rest is its
        // own decision, open in ROADMAP P13.
        Self {
            w0: 0.4072,
            w1: 1.1829,
            w2: 3.1262,
            w3: 15.4722,
            w4: 7.2102,
            w5: 0.5316,
            w6: 1.0651,
            w7: 0.0234,
            w8: 1.616,
            w9: 0.1544,
            w10: 1.0,
            w11: 1.9395,
            w12: 0.1176,
            w13: 0.0,
            w14: 0.0,
            w15: 0.0,
            w16: 2.2035,
            w17: 0.0,
            w18: 0.0,
            w19: 0.0,
            // FSRS-6's published decay. Was 0.5 (the FSRS-5 constant), then
            // 0.0658 (w[19] of the FSRS-6 vector, read one slot short) — see
            // the field doc for both corrections.
            w20: FSRS6_DEFAULT_DECAY,
        }
    }
}

/// Memory state based on accessibility
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryState {
    /// High retention (≥70%), immediately retrievable
    Active,
    /// Medium retention (40-70%), retrievable with effort
    Dormant,
    /// Low retention (10-40%), rarely surfaces
    Silent,
    /// Below threshold (<10%), effectively forgotten
    Unavailable,
}

impl MemoryState {
    /// Determine state from accessibility score
    #[must_use]
    pub fn from_accessibility(accessibility: f32) -> Self {
        if accessibility >= 0.7 {
            Self::Active
        } else if accessibility >= 0.4 {
            Self::Dormant
        } else if accessibility >= 0.1 {
            Self::Silent
        } else {
            Self::Unavailable
        }
    }
}

/// FSRS state for a memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsrsState {
    /// Stability: days until retrievability drops to 90%
    pub stability: f32,
    /// Difficulty: inherent difficulty (1-10)
    pub difficulty: f32,
    /// Last access timestamp (Unix seconds)
    pub last_access: i64,
    /// Number of times this memory has been accessed
    pub access_count: u32,
    /// Importance multiplier (default 1.0, can be boosted)
    pub importance: f32,
    /// Storage strength (only increases, never decreases)
    pub storage_strength: f32,
    /// Number of consolidation generations
    pub generation: u32,
}

impl Default for FsrsState {
    fn default() -> Self {
        Self {
            stability: 1.0,      // 1 day initial stability
            difficulty: 5.0,    // Medium difficulty
            last_access: now(),
            access_count: 0,
            importance: 1.0,
            storage_strength: 0.1,
            generation: 0,
        }
    }
}

impl FsrsState {
    /// Create new FSRS state
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Calculate retrievability (probability of recall)
    /// 
    /// Uses power law forgetting: R(t, S) = (1 + factor × t / S)^(-decay)
    #[must_use]
    pub fn retrievability(&self, params: &FsrsParameters) -> f32 {
        let elapsed_days = self.elapsed_days();
        power_law_retrievability(elapsed_days, self.stability, params.w20)
    }

    /// Calculate retrieval strength (decays, restored by access)
    #[must_use]
    pub fn retrieval_strength(&self, params: &FsrsParameters) -> f32 {
        // Retrieval strength decays faster than stability
        let r = self.retrievability(params);
        // Weight by access frequency (more accesses = stronger retrieval paths)
        let access_factor = (self.access_count as f32 / 10.0).min(1.0);
        r * (0.5 + 0.5 * access_factor)
    }

    /// Calculate overall accessibility score
    /// 
    /// accessibility = 0.5 × retention + 0.3 × retrieval_strength + 0.2 × storage_strength
    #[must_use]
    pub fn accessibility(&self, params: &FsrsParameters) -> f32 {
        let retention = self.retrievability(params);
        let retrieval = self.retrieval_strength(params);
        let storage = self.storage_strength.min(1.0);
        
        0.5 * retention + 0.3 * retrieval + 0.2 * storage
    }

    /// Get memory state based on accessibility
    #[must_use]
    pub fn state(&self, params: &FsrsParameters) -> MemoryState {
        MemoryState::from_accessibility(self.accessibility(params))
    }

    /// Calculate weight for summarization
    /// 
    /// weight < 0.2: compress to essence
    /// weight 0.2-0.8: moderate compression
    /// weight > 0.8: full detail
    /// weight > 1.0: expand/research
    #[must_use]
    pub fn weight(&self, params: &FsrsParameters) -> f32 {
        self.retrievability(params) * self.importance
    }

    /// Days elapsed since last access
    #[must_use]
    pub fn elapsed_days(&self) -> f32 {
        let elapsed_secs = now() - self.last_access;
        elapsed_secs as f32 / 86400.0
    }

    /// Record an access (the testing effect)
    /// 
    /// Every retrieval strengthens the memory
    pub fn record_access(&mut self, params: &FsrsParameters, rating: Rating) {
        // Update stability based on current state and rating
        let r = self.retrievability(params);
        
        // Stability increase formula from FSRS
        let stability_modifier = match rating {
            Rating::Again => params.w9, // Decrease on failure
            Rating::Hard => 1.0 + params.w8 * params.w11.powf(-self.difficulty / 10.0),
            Rating::Good => 1.0 + params.w8 * (1.0 - r).powf(params.w10),
            Rating::Easy => 1.0 + params.w8 * params.w12.powf(self.difficulty / 10.0),
        };
        
        self.stability = (self.stability * stability_modifier).max(0.1);
        
        // Update difficulty
        let difficulty_delta = match rating {
            Rating::Again => params.w6,
            Rating::Hard => params.w6 * 0.5,
            Rating::Good => -params.w7,
            Rating::Easy => -params.w7 * 2.0,
        };
        self.difficulty = (self.difficulty + difficulty_delta).clamp(1.0, 10.0);
        
        // Storage strength only increases
        self.storage_strength = (self.storage_strength + 0.1).min(1.0);
        
        // Update access tracking
        self.last_access = now();
        self.access_count += 1;
    }

    /// Promote memory (mark as helpful/important)
    pub fn promote(&mut self, boost: f32) {
        self.importance = (self.importance + boost).min(3.0);
        self.storage_strength = (self.storage_strength + 0.2).min(1.0);
    }

    /// Demote memory (mark as wrong/unhelpful)
    pub fn demote(&mut self, penalty: f32) {
        self.importance = (self.importance - penalty).max(0.1);
    }
}

/// Rating for memory access quality
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rating {
    /// Complete failure to recall
    Again,
    /// Recalled with significant difficulty
    Hard,
    /// Recalled correctly
    Good,
    /// Recalled easily
    Easy,
}

impl Default for Rating {
    fn default() -> Self {
        Self::Good
    }
}

/// Calculate retrievability using power law forgetting curve
/// 
/// R(t, S) = (1 + factor × t / S)^(-decay)
/// where factor = 0.9^(-1/decay) - 1
#[must_use]
pub fn power_law_retrievability(elapsed_days: f32, stability: f32, decay: f32) -> f32 {
    if elapsed_days <= 0.0 {
        return 1.0;
    }
    if stability <= 0.0 {
        return 0.0;
    }
    
    let factor = 0.9_f32.powf(-1.0 / decay) - 1.0;
    (1.0 + factor * elapsed_days / stability).powf(-decay).clamp(0.0, 1.0)
}

/// Calculate similarity threshold for prediction error gating
/// 
/// Returns action based on similarity:
/// - >0.92: Reinforce existing
/// - >0.75: Update existing  
/// - <0.75: Create new
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestAction {
    /// Almost identical - just strengthen existing memory
    Reinforce,
    /// Related - merge/update existing memory
    Update,
    /// Novel - create new memory
    Create,
}

impl IngestAction {
    #[must_use]
    pub fn from_similarity(similarity: f32) -> Self {
        if similarity > 0.92 {
            Self::Reinforce
        } else if similarity > 0.75 {
            Self::Update
        } else {
            Self::Create
        }
    }
}

/// Get current Unix timestamp
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retrievability_decay() {
        let params = FsrsParameters::default();
        let mut state = FsrsState::new();
        state.stability = 1.0; // 1 day stability
        
        // Simulate time passing by manipulating last_access
        state.last_access = now() - 86400; // 1 day ago
        let r1 = state.retrievability(&params);
        
        state.last_access = now() - 86400 * 7; // 7 days ago
        let r7 = state.retrievability(&params);
        
        state.last_access = now() - 86400 * 30; // 30 days ago
        let r30 = state.retrievability(&params);
        
        // Should decay over time
        assert!(r1 > r7);
        assert!(r7 > r30);
        
        // Should follow power law (not exponential - decay is slower)
        println!("R at 1 day: {:.3}", r1);
        println!("R at 7 days: {:.3}", r7);
        println!("R at 30 days: {:.3}", r30);
    }

    #[test]
    fn test_memory_states() {
        assert_eq!(MemoryState::from_accessibility(0.9), MemoryState::Active);
        assert_eq!(MemoryState::from_accessibility(0.5), MemoryState::Dormant);
        assert_eq!(MemoryState::from_accessibility(0.2), MemoryState::Silent);
        assert_eq!(MemoryState::from_accessibility(0.05), MemoryState::Unavailable);
    }

    #[test]
    fn test_testing_effect() {
        let params = FsrsParameters::default();
        let mut state = FsrsState::new();
        state.last_access = now() - 86400; // 1 day ago
        
        let stability_before = state.stability;
        state.record_access(&params, Rating::Good);
        let stability_after = state.stability;
        
        // Stability should increase after successful recall
        assert!(stability_after > stability_before);
        assert_eq!(state.access_count, 1);
    }

    #[test]
    fn test_weight_calculation() {
        let params = FsrsParameters::default();
        let mut state = FsrsState::new();
        
        // Fresh memory should have high weight
        let weight_fresh = state.weight(&params);
        assert!(weight_fresh > 0.8);
        
        // Old memory should have low weight
        state.last_access = now() - 86400 * 30;
        let weight_old = state.weight(&params);
        assert!(weight_old < weight_fresh);
        
        // Promoted memory should have boosted weight
        state.promote(1.0);
        let weight_promoted = state.weight(&params);
        assert!(weight_promoted > weight_old);
    }

    #[test]
    fn test_ingest_action() {
        assert_eq!(IngestAction::from_similarity(0.95), IngestAction::Reinforce);
        assert_eq!(IngestAction::from_similarity(0.85), IngestAction::Update);
        assert_eq!(IngestAction::from_similarity(0.5), IngestAction::Create);
    }

    /// The default decay must be a value the reference optimizer could have
    /// produced. This is the check that would have caught reading `w[19]`
    /// (`0.0658`) as `w[20]`: it is below `fsrs-rs`'s clamp floor, so no fitted
    /// FSRS-6 parameter set can contain it in that slot.
    #[test]
    fn default_decay_is_inside_the_reference_clamp_range() {
        let default_decay = FsrsParameters::default().w20;
        assert_eq!(default_decay, FSRS6_DEFAULT_DECAY);
        assert!(
            default_decay >= DECAY_MIN,
            "decay {default_decay} is below the optimizer's floor {DECAY_MIN} — \
             a value this small is not a fitted FSRS parameter"
        );
        assert!(
            default_decay <= DECAY_MAX,
            "decay {default_decay} is above the optimizer's ceiling {DECAY_MAX}"
        );
        // Negative space: the two values this default has wrongly held are
        // exactly the ones the range rejects and accepts-but-is-not.
        assert!(
            0.0658_f32 < DECAY_MIN,
            "the w[19] misread must be out of range"
        );
        assert!(
            FSRS5_DEFAULT_DECAY <= DECAY_MAX,
            "FSRS-5's decay is in range, just wrong for FSRS-6"
        );
        assert!(FSRS5_DEFAULT_DECAY != FSRS6_DEFAULT_DECAY);
    }

    /// The curve is anchored: whatever the exponent, retrievability is 0.9 at
    /// `t == S`. That is the definition of stability, and it is what makes the
    /// `factor = 0.9^(-1/decay) - 1` term correct rather than a fudge — so a
    /// decay change moves the *tail*, never the anchor.
    #[test]
    fn every_decay_keeps_the_ninety_percent_anchor_at_one_stability() {
        for decay in [
            FSRS5_DEFAULT_DECAY,
            FSRS6_DEFAULT_DECAY,
            DECAY_MIN,
            DECAY_MAX,
        ] {
            let r = power_law_retrievability(10.0, 10.0, decay);
            assert!(
                (r - 0.9).abs() < 1e-3,
                "decay {decay} broke the anchor: R(t=S) = {r}, expected 0.9"
            );
        }
    }

    /// A smaller exponent is a flatter tail. Stated as an ordering rather than
    /// as fixed numbers so it documents the *property* the decay controls.
    #[test]
    fn smaller_decay_retains_aged_memories_longer() {
        // 800 days at 1-day stability — the retention harness's aged corpus.
        let aged = |decay: f32| power_law_retrievability(800.0, 1.0, decay);
        let fsrs5 = aged(FSRS5_DEFAULT_DECAY);
        let fsrs6 = aged(FSRS6_DEFAULT_DECAY);
        let misread = aged(0.0658);
        assert!(
            fsrs5 < fsrs6,
            "FSRS-5's 0.5 must decay faster: {fsrs5} vs {fsrs6}"
        );
        assert!(
            fsrs6 < misread,
            "the w[19] misread was flatter still: {fsrs6} vs {misread}"
        );
        // The correction must not undo what the first flip bought: the default
        // recall gate is `min_weight = 0.1`, and FSRS-6's exponent clears it at
        // this age while FSRS-5's does not.
        assert!(
            fsrs5 < 0.1,
            "FSRS-5's decay drops an 800-day memory below the gate: {fsrs5}"
        );
        assert!(
            fsrs6 > 0.1,
            "FSRS-6's decay must keep it retrievable: {fsrs6}"
        );
    }
}
