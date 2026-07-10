#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! GPU-accelerated compute for Nanna using wgpu
//!
//! Provides GPU acceleration for embedding operations, matrix multiplication,
//! and other compute-heavy tasks.

use bytemuck::Pod;
use std::sync::Arc;
use thiserror::Error;
use tracing::info;

pub mod memory_manager;

pub use memory_manager::{BatchedSearch, GpuMemoryStats, GpuVectorStore};

#[derive(Error, Debug)]
pub enum GpuError {
    #[error("No suitable GPU adapter found")]
    NoAdapter,
    #[error("Failed to request device: {0}")]
    DeviceRequest(#[from] wgpu::RequestDeviceError),
    #[error("Shader compilation error: {0}")]
    ShaderCompilation(String),
    #[error("Buffer mapping failed")]
    BufferMapping,
    #[error("GPU memory insufficient: {0}")]
    InsufficientMemory(String),
}

/// GPU compute context
pub struct GpuContext {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub adapter_info: wgpu::AdapterInfo,
}

impl GpuContext {
    /// Initialize GPU context, preferring high-performance discrete GPU.
    ///
    /// # Errors
    ///
    /// Returns `GpuError::NoAdapter` if no GPU adapter is found.
    /// Returns `GpuError::DeviceCreation` if the GPU device cannot be created.
    pub async fn new() -> Result<Self, GpuError> {
        // wgpu 30: Instance::default() == all backends (what the old explicit
        // InstanceDescriptor { backends: all(), .. } spelled out).
        let instance = wgpu::Instance::default();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
            .map_err(|_| GpuError::NoAdapter)?;

        let adapter_info = adapter.get_info();
        info!(
            "GPU: {} ({:?})",
            adapter_info.name, adapter_info.backend
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("nanna-gpu"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            })
            .await?;

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            adapter_info,
        })
    }

    /// Check if GPU supports compute shaders
    #[must_use] 
    pub const fn supports_compute(&self) -> bool {
        true // wgpu always supports compute on valid adapters
    }
}

/// GPU buffer for compute operations
pub struct GpuBuffer {
    _buffer: wgpu::Buffer,
    _size: u64,
}

impl GpuBuffer {
    /// Create a new GPU buffer from CPU data
    pub fn from_slice<T: Pod>(ctx: &GpuContext, data: &[T], usage: wgpu::BufferUsages) -> Self {
        let buffer = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("compute_buffer"),
            contents: bytemuck::cast_slice(data),
            usage,
        });

        Self {
            _buffer: buffer,
            _size: std::mem::size_of_val(data) as u64,
        }
    }

    /// Create an empty buffer for output
    #[must_use] 
    pub fn empty(ctx: &GpuContext, size: u64, usage: wgpu::BufferUsages) -> Self {
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("output_buffer"),
            size,
            usage,
            mapped_at_creation: false,
        });

        Self { _buffer: buffer, _size: size }
    }
}

/// Dot product compute shader (WGSL)
const _DOT_PRODUCT_SHADER: &str = r"
@group(0) @binding(0) var<storage, read> a: array<f32>;
@group(0) @binding(1) var<storage, read> b: array<f32>;
@group(0) @binding(2) var<storage, read_write> result: array<f32>;

var<workgroup> partial_sums: array<f32, 256>;

@compute @workgroup_size(256)
fn main(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>,
    @builtin(workgroup_id) workgroup_id: vec3<u32>,
) {
    let idx = global_id.x;
    let local_idx = local_id.x;
    
    // Each thread computes one multiplication
    if (idx < arrayLength(&a)) {
        partial_sums[local_idx] = a[idx] * b[idx];
    } else {
        partial_sums[local_idx] = 0.0;
    }
    
    workgroupBarrier();
    
    // Parallel reduction
    for (var stride = 128u; stride > 0u; stride = stride >> 1u) {
        if (local_idx < stride) {
            partial_sums[local_idx] += partial_sums[local_idx + stride];
        }
        workgroupBarrier();
    }
    
    // First thread writes workgroup result
    if (local_idx == 0u) {
        result[workgroup_id.x] = partial_sums[0];
    }
}
";

/// Cosine similarity batch compute shader
const COSINE_SIMILARITY_BATCH_SHADER: &str = r"
struct Params {
    query_len: u32,
    num_vectors: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> query: array<f32>;
@group(0) @binding(2) var<storage, read> vectors: array<f32>;
@group(0) @binding(3) var<storage, read_write> similarities: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let vec_idx = global_id.x;
    if (vec_idx >= params.num_vectors) {
        return;
    }
    
    let offset = vec_idx * params.query_len;
    var dot: f32 = 0.0;
    var norm_q: f32 = 0.0;
    var norm_v: f32 = 0.0;
    
    for (var i = 0u; i < params.query_len; i++) {
        let q = query[i];
        let v = vectors[offset + i];
        dot += q * v;
        norm_q += q * q;
        norm_v += v * v;
    }
    
    similarities[vec_idx] = dot / (sqrt(norm_q) * sqrt(norm_v));
}
";

/// GPU-accelerated cosine similarity search
pub struct CosineSimilaritySearch {
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

/// Parameters struct for the compute shader (must match WGSL layout)
#[repr(C)]
#[derive(Clone, Copy, Pod, bytemuck::Zeroable)]
struct SimilarityParams {
    query_len: u32,
    num_vectors: u32,
}

impl CosineSimilaritySearch {
    /// Create a new cosine similarity search pipeline.
    ///
    /// # Errors
    ///
    /// Returns `GpuError` if the compute pipeline cannot be created.
    pub fn new(ctx: &GpuContext) -> Result<Self, GpuError> {
        let shader = ctx.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cosine_similarity_shader"),
            source: wgpu::ShaderSource::Wgsl(COSINE_SIMILARITY_BATCH_SHADER.into()),
        });

        let bind_group_layout = ctx.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cosine_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = ctx.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cosine_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            ..Default::default()
        });

        let pipeline = ctx.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("cosine_similarity_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Ok(Self {
            pipeline,
            bind_group_layout,
        })
    }

    /// Compute cosine similarity between a query vector and a batch of vectors.
    ///
    /// Returns a vector of similarity scores, one per input vector.
    ///
    /// # Arguments
    ///
    /// * `ctx` - GPU context
    /// * `query` - Query vector (normalized)
    /// * `vectors` - Batch of vectors to compare against (flattened, each same length as query)
    ///
    /// # Errors
    ///
    /// Returns `GpuError::BufferMapping` if the result buffer cannot be read.
    pub async fn search(
        &self,
        ctx: &GpuContext,
        query: &[f32],
        vectors: &[f32],
    ) -> Result<Vec<f32>, GpuError> {
        let query_len = query.len() as u32;
        let num_vectors = (vectors.len() / query.len()) as u32;

        if num_vectors == 0 {
            return Ok(vec![]);
        }

        // Create uniform buffer for parameters
        let params = SimilarityParams {
            query_len,
            num_vectors,
        };
        let params_buffer = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("params_buffer"),
            contents: bytemuck::cast_slice(&[params]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // Create storage buffers
        let query_buffer = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("query_buffer"),
            contents: bytemuck::cast_slice(query),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let vectors_buffer = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vectors_buffer"),
            contents: bytemuck::cast_slice(vectors),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let output_size = (num_vectors as usize) * std::mem::size_of::<f32>();
        let output_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("output_buffer"),
            size: output_size as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let staging_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging_buffer"),
            size: output_size as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create bind group
        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cosine_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: query_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: vectors_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: output_buffer.as_entire_binding(),
                },
            ],
        });

        // Dispatch compute
        let mut encoder = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cosine_encoder"),
        });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cosine_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            // Dispatch with ceiling division for workgroups
            let workgroups = (num_vectors + 63) / 64;
            pass.dispatch_workgroups(workgroups, 1, 1);
        }

        // Copy output to staging buffer
        encoder.copy_buffer_to_buffer(&output_buffer, 0, &staging_buffer, 0, output_size as u64);

        ctx.queue.submit(std::iter::once(encoder.finish()));

        // Read results
        let buffer_slice = staging_buffer.slice(..);
        let (tx, rx) = futures::channel::oneshot::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });

        // wgpu 30: Maintain → PollType; poll now returns a Result.
        ctx.device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
            .map_err(|_| GpuError::BufferMapping)?;

        rx.await
            .map_err(|_| GpuError::BufferMapping)?
            .map_err(|_| GpuError::BufferMapping)?;

        // wgpu 30: get_mapped_range returns a Result.
        let data = buffer_slice
            .get_mapped_range()
            .map_err(|_| GpuError::BufferMapping)?;
        let results: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging_buffer.unmap();

        Ok(results)
    }
}

// wgpu buffer init descriptor helper
mod wgpu {
    pub use ::wgpu::*;
    
    pub mod util {
        use super::BufferUsages;
        
        pub struct BufferInitDescriptor<'a> {
            pub label: Option<&'a str>,
            pub contents: &'a [u8],
            pub usage: BufferUsages,
        }
    }
}

trait DeviceExt {
    fn create_buffer_init(&self, desc: &wgpu::util::BufferInitDescriptor) -> wgpu::Buffer;
}

impl DeviceExt for wgpu::Device {
    fn create_buffer_init(&self, desc: &wgpu::util::BufferInitDescriptor) -> wgpu::Buffer {
        let unpadded_size = desc.contents.len() as u64;
        let padding = (4 - (unpadded_size % 4)) % 4;
        let padded_size = unpadded_size + padding;
        
        let buffer = self.create_buffer(&wgpu::BufferDescriptor {
            label: desc.label,
            size: padded_size,
            usage: desc.usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        
        // wgpu 30: get_mapped_range_mut returns a Result, and BufferViewMut is
        // written via slice().copy_from_slice (no direct indexing). A buffer
        // created with `mapped_at_creation: true` is mapped by construction —
        // failure here is a programmer error, so the expect is an invariant assert.
        buffer
            .slice(..)
            .get_mapped_range_mut()
            .expect("buffer created with mapped_at_creation must be mappable")
            .slice(..desc.contents.len())
            .copy_from_slice(desc.contents);
        buffer.unmap();
        
        buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_gpu_context_creation() {
        // Skip if no GPU available (CI environments)
        match GpuContext::new().await {
            Ok(ctx) => {
                println!("GPU: {}", ctx.adapter_info.name);
                assert!(ctx.supports_compute());
            }
            Err(GpuError::NoAdapter) => {
                println!("No GPU adapter found, skipping test");
            }
            Err(e) => panic!("Unexpected error: {}", e),
        }
    }
}
