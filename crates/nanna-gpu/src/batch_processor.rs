//! Batch processing for GPU vector operations.
//!
//! This module provides functionality for processing vectors in batches,
//! with automatic memory management, dimension validation, and performance tuning.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, warn, info};

/// Errors that can occur during batch processing.
#[derive(Error, Debug)]
pub enum BatchError {
    /// Input vectors are invalid or empty.
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// Vector dimensions do not match.
    #[error("Dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    /// Error during vector processing.
    #[error("Processing error: {0}")]
    ProcessingError(String),

    /// Batch size is too large for available memory.
    #[error("Batch size {batch_size} exceeds available memory {available_memory}")]
    BatchTooLarge {
        batch_size: usize,
        available_memory: usize,
    },
}

/// Processes vectors in batches with automatic memory management.
///
/// The `BatchProcessor` handles:
/// - Batching vectors for efficient GPU processing
/// - Memory tracking and management
/// - Dimension validation
/// - Auto-tuning batch sizes based on available memory
#[derive(Clone)]
pub struct BatchProcessor {
    /// Maximum number of vectors per batch.
    batch_size: usize,

    /// Current memory usage in bytes, tracked across operations.
    current_memory_usage: Arc<AtomicUsize>,

    /// Expected dimension of vectors, validated on first batch.
    vector_dimension: Arc<parking_lot::RwLock<Option<usize>>>,
}

impl BatchProcessor {
    /// Creates a new batch processor with the specified batch size.
    ///
    /// # Arguments
    ///
    /// * `batch_size` - Maximum number of vectors to process in a single batch.
    ///
    /// # Example
    ///
    /// ```
    /// use nanna_gpu::batch_processor::BatchProcessor;
    ///
    /// let processor = BatchProcessor::new(256);
    /// assert_eq!(processor.get_batch_size(), 256);
    /// ```
    pub fn new(batch_size: usize) -> Self {
        debug!("Creating BatchProcessor with batch_size: {}", batch_size);

        Self {
            batch_size,
            current_memory_usage: Arc::new(AtomicUsize::new(0)),
            vector_dimension: Arc::new(parking_lot::RwLock::new(None)),
        }
    }

    /// Processes a slice of vectors in batches.
    ///
    /// This method:
    /// - Validates all input vectors have consistent dimensions
    /// - Splits vectors into batches
    /// - Processes each batch
    /// - Aggregates results
    ///
    /// # Arguments
    ///
    /// * `vectors` - Slice of vectors to process.
    ///
    /// # Returns
    ///
    /// A flattened vector containing all processed values, or an error.
    ///
    /// # Errors
    ///
    /// Returns `BatchError` if:
    /// - Input is empty
    /// - Vector dimensions are inconsistent
    /// - Processing fails
    pub async fn process_vectors(&self, vectors: &[Vec<f32>]) -> Result<Vec<f32>, BatchError> {
        if vectors.is_empty() {
            return Err(BatchError::InvalidInput(
                "Cannot process empty vector slice".to_string(),
            ));
        }

        // Validate dimensions
        let dimension = vectors[0].len();
        if dimension == 0 {
            return Err(BatchError::InvalidInput(
                "Vectors cannot have zero dimension".to_string(),
            ));
        }

        self.validate_dimensions(vectors, dimension)?;
        self.set_vector_dimension(dimension)?;

        debug!(
            "Processing {} vectors of dimension {}",
            vectors.len(),
            dimension
        );

        // Calculate memory requirements
        let batch_memory = self.calculate_batch_memory(dimension);
        if batch_memory > self.batch_size * dimension * std::mem::size_of::<f32>() {
            warn!(
                "Batch memory requirement {} exceeds estimated capacity",
                batch_memory
            );
        }

        let mut results = Vec::new();
        let total_batches = (vectors.len() + self.batch_size - 1) / self.batch_size;

        for (batch_idx, batch) in vectors.chunks(self.batch_size).enumerate() {
            debug!(
                "Processing batch {}/{}: {} vectors",
                batch_idx + 1,
                total_batches,
                batch.len()
            );

            let batch_result = self.process_batch(batch).await?;
            results.extend(batch_result);

            // Update memory tracking
            self.update_memory_usage(batch_memory);
        }

        info!(
            "Successfully processed {} vectors in {} batches",
            vectors.len(),
            total_batches
        );

        Ok(results)
    }

    /// Automatically tunes batch size based on available memory.
    ///
    /// Uses heuristics to determine optimal batch size:
    /// - Assumes 4 bytes per f32 value
    /// - Reserves 20% of memory for overhead
    /// - Minimum batch size of 1, maximum of 65536
    ///
    /// # Arguments
    ///
    /// * `available_memory` - Available GPU/system memory in bytes.
    ///
    /// # Returns
    ///
    /// Recommended batch size.
    ///
    /// # Example
    ///
    /// ```
    /// use nanna_gpu::batch_processor::BatchProcessor;
    ///
    /// let batch_size = BatchProcessor::auto_tune_batch_size(1024 * 1024 * 1024); // 1GB
    /// assert!(batch_size > 0);
    /// assert!(batch_size <= 65536);
    /// ```
    pub fn auto_tune_batch_size(available_memory: usize) -> usize {
        const FLOAT_SIZE: usize = std::mem::size_of::<f32>();
        const MEMORY_OVERHEAD_RATIO: f32 = 0.8; // Use 80% of available memory
        const MIN_BATCH_SIZE: usize = 1;
        const MAX_BATCH_SIZE: usize = 65536;

        let usable_memory = (available_memory as f32 * MEMORY_OVERHEAD_RATIO) as usize;
        let batch_size = usable_memory / FLOAT_SIZE;

        let tuned = batch_size
            .max(MIN_BATCH_SIZE)
            .min(MAX_BATCH_SIZE);

        info!(
            "Auto-tuned batch size: {} (available: {} bytes)",
            tuned, available_memory
        );

        tuned
    }

    /// Returns the current memory usage in bytes.
    ///
    /// # Example
    ///
    /// ```
    /// use nanna_gpu::batch_processor::BatchProcessor;
    ///
    /// let processor = BatchProcessor::new(256);
    /// assert_eq!(processor.get_memory_usage(), 0);
    /// ```
    pub fn get_memory_usage(&self) -> usize {
        self.current_memory_usage.load(Ordering::Relaxed)
    }

    /// Resets memory tracking to zero.
    ///
    /// Useful for starting a new processing cycle or resetting metrics.
    pub fn reset_memory_tracking(&self) {
        self.current_memory_usage.store(0, Ordering::Relaxed);
        debug!("Memory tracking reset");
    }

    /// Returns the current batch size.
    pub fn get_batch_size(&self) -> usize {
        self.batch_size
    }

    /// Returns the vector dimension if set.
    pub fn get_vector_dimension(&self) -> Option<usize> {
        *self.vector_dimension.read()
    }

    // ============================================================================
    // Helper Methods
    // ============================================================================

    /// Validates that all vectors have the expected dimension.
    fn validate_dimensions(&self, vectors: &[Vec<f32>], expected: usize) -> Result<(), BatchError> {
        for (idx, vector) in vectors.iter().enumerate() {
            if vector.len() != expected {
                return Err(BatchError::DimensionMismatch {
                    expected,
                    actual: vector.len(),
                });
            }
        }
        Ok(())
    }

    /// Sets the vector dimension on first use.
    fn set_vector_dimension(&self, dimension: usize) -> Result<(), BatchError> {
        let mut dim_lock = self.vector_dimension.write();
        if let Some(existing) = *dim_lock {
            if existing != dimension {
                return Err(BatchError::DimensionMismatch {
                    expected: existing,
                    actual: dimension,
                });
            }
        } else {
            *dim_lock = Some(dimension);
            debug!("Vector dimension set to: {}", dimension);
        }
        Ok(())
    }

    /// Calculates memory required for a batch of vectors.
    fn calculate_batch_memory(&self, vector_dimension: usize) -> usize {
        let vectors_per_batch = self.batch_size;
        let bytes_per_vector = vector_dimension * std::mem::size_of::<f32>();
        vectors_per_batch * bytes_per_vector
    }

    /// Processes a single batch of vectors.
    async fn process_batch(&self, batch: &[Vec<f32>]) -> Result<Vec<f32>, BatchError> {
        if batch.is_empty() {
            return Ok(Vec::new());
        }

        // Flatten batch into a single vector
        let flattened: Vec<f32> = batch.iter().flat_map(|v| v.iter().copied()).collect();

        // Simulate async processing (in real implementation, this would be GPU work)
        tokio::task::yield_now().await;

        // Return processed result (identity operation for now)
        Ok(flattened)
    }

    /// Updates memory usage tracking.
    fn update_memory_usage(&self, batch_memory: usize) {
        let current = self.current_memory_usage.load(Ordering::Relaxed);
        self.current_memory_usage
            .store(current + batch_memory, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_processor() {
        let processor = BatchProcessor::new(256);
        assert_eq!(processor.get_batch_size(), 256);
        assert_eq!(processor.get_memory_usage(), 0);
        assert_eq!(processor.get_vector_dimension(), None);
    }

    #[test]
    fn test_auto_tune_batch_size() {
        let size_1gb = BatchProcessor::auto_tune_batch_size(1024 * 1024 * 1024);
        let size_512mb = BatchProcessor::auto_tune_batch_size(512 * 1024 * 1024);

        assert!(size_1gb > 0);
        assert!(size_512mb > 0);
        assert!(size_1gb > size_512mb);
        assert!(size_1gb <= 65536);
        assert!(size_512mb <= 65536);
    }

    #[test]
    fn test_auto_tune_batch_size_bounds() {
        let tiny = BatchProcessor::auto_tune_batch_size(100);
        assert_eq!(tiny, 1); // Minimum bound

        let huge = BatchProcessor::auto_tune_batch_size(usize::MAX);
        assert_eq!(huge, 65536); // Maximum bound
    }

    #[test]
    fn test_memory_tracking() {
        let processor = BatchProcessor::new(256);
        assert_eq!(processor.get_memory_usage(), 0);

        processor.update_memory_usage(1024);
        assert_eq!(processor.get_memory_usage(), 1024);

        processor.update_memory_usage(512);
        assert_eq!(processor.get_memory_usage(), 1536);

        processor.reset_memory_tracking();
        assert_eq!(processor.get_memory_usage(), 0);
    }

    #[test]
    fn test_calculate_batch_memory() {
        let processor = BatchProcessor::new(256);
        let memory = processor.calculate_batch_memory(768);

        let expected = 256 * 768 * std::mem::size_of::<f32>();
        assert_eq!(memory, expected);
    }

    #[tokio::test]
    async fn test_process_empty_vectors() {
        let processor = BatchProcessor::new(256);
        let result = processor.process_vectors(&[]).await;

        assert!(result.is_err());
        match result {
            Err(BatchError::InvalidInput(msg)) => assert!(msg.contains("empty")),
            _ => panic!("Expected InvalidInput error"),
        }
    }

    #[tokio::test]
    async fn test_process_zero_dimension_vectors() {
        let processor = BatchProcessor::new(256);
        let vectors = vec![vec![]];

        let result = processor.process_vectors(&vectors).await;
        assert!(result.is_err());
        match result {
            Err(BatchError::InvalidInput(msg)) => assert!(msg.contains("zero dimension")),
            _ => panic!("Expected InvalidInput error"),
        }
    }

    #[tokio::test]
    async fn test_process_dimension_mismatch() {
        let processor = BatchProcessor::new(256);
        let vectors = vec![vec![1.0, 2.0, 3.0], vec![1.0, 2.0]];

        let result = processor.process_vectors(&vectors).await;
        assert!(result.is_err());
        match result {
            Err(BatchError::DimensionMismatch { expected, actual }) => {
                assert_eq!(expected, 3);
                assert_eq!(actual, 2);
            }
            _ => panic!("Expected DimensionMismatch error"),
        }
    }

    #[tokio::test]
    async fn test_process_single_batch() {
        let processor = BatchProcessor::new(256);
        let vectors = vec![
            vec![1.0, 2.0, 3.0],
            vec![4.0, 5.0, 6.0],
            vec![7.0, 8.0, 9.0],
        ];

        let result = processor.process_vectors(&vectors).await;
        assert!(result.is_ok());

        let processed = result.unwrap();
        assert_eq!(processed.len(), 9);
        assert_eq!(processed, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    }

    #[tokio::test]
    async fn test_process_multiple_batches() {
        let processor = BatchProcessor::new(2);
        let vectors = vec![
            vec![1.0, 2.0],
            vec![3.0, 4.0],
            vec![5.0, 6.0],
            vec![7.0, 8.0],
            vec![9.0, 10.0],
        ];

        let result = processor.process_vectors(&vectors).await;
        assert!(result.is_ok());

        let processed = result.unwrap();
        assert_eq!(processed.len(), 10);
        assert_eq!(
            processed,
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]
        );
    }

    #[tokio::test]
    async fn test_vector_dimension_persistence() {
        let processor = BatchProcessor::new(256);
        assert_eq!(processor.get_vector_dimension(), None);

        let vectors = vec![vec![1.0, 2.0, 3.0]];
        let _ = processor.process_vectors(&vectors).await;
        assert_eq!(processor.get_vector_dimension(), Some(3));

        // Second call with same dimension should succeed
        let vectors2 = vec![vec![4.0, 5.0, 6.0]];
        let result = processor.process_vectors(&vectors2).await;
        assert!(result.is_ok());

        // Call with different dimension should fail
        let vectors3 = vec![vec![1.0, 2.0]];
        let result = processor.process_vectors(&vectors3).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_memory_tracking_during_processing() {
        let processor = BatchProcessor::new(2);
        assert_eq!(processor.get_memory_usage(), 0);

        let vectors = vec![
            vec![1.0, 2.0, 3.0],
            vec![4.0, 5.0, 6.0],
            vec![7.0, 8.0, 9.0],
            vec![10.0, 11.0, 12.0],
        ];

        let _ = processor.process_vectors(&vectors).await;
        assert!(processor.get_memory_usage() > 0);
    }
}
