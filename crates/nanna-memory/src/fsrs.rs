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

/// FSRS-6 parameters (21 parameters optimized on 700M+ reviews)
/// These are the default parameters from FSRS-6
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
    /// Forgetting-curve decay exponent.
    ///
    /// NOTE: the default below is `0.5` — FSRS-4.5/5's hardcoded `DECAY`, **not** an
    /// FSRS-6 value (FSRS-6's default is `0.0658`, and making this trainable is the
    /// point of FSRS-6). The curve here is FSRS-6's; only this constant lags. Changing
    /// it moves live memory behavior, so it is gated on the retention harness — see
    /// ROADMAP P13.
    pub w20: f32,
}

impl Default for FsrsParameters {
    fn default() -> Self {
        // Default FSRS-6 parameters
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
            // FSRS-4.5/5 DECAY; FSRS-6 defaults to 0.0658 — see the field doc.
            w20: 0.5,
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
}
