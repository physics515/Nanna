#![warn(clippy::all, clippy::restriction)]
#![deny(clippy::pedantic, clippy::nursery)]

//! GPU-accelerated compute for Nanna using wgpu
//!
//! Provides GPU acceleration for embedding operations, matrix multiplication,
//! and other compute-heavy tasks.
#![allow(dead_code)]

use bytemuck::Pod;
use std::sync::Arc;
use thiserror::Error;
use tracing::info;

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
}

/// GPU compute context
pub struct GpuContext {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub adapter_info: wgpu::AdapterInfo,
}

impl GpuContext {
    /// Initialize GPU context, preferring high-performance discrete GPU
    pub async fn new() -> Result<Self, GpuError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or(GpuError::NoAdapter)?;

        let adapter_info = adapter.get_info();
        info!(
            "GPU: {} ({:?})",
            adapter_info.name, adapter_info.backend
        );

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("nanna-gpu"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await?;

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            adapter_info,
        })
    }

    /// Check if GPU supports compute shaders
    pub fn supports_compute(&self) -> bool {
        true // wgpu always supports compute on valid adapters
    }
}

/// GPU buffer for compute operations
pub struct GpuBuffer {
    buffer: wgpu::Buffer,
    size: u64,
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
            buffer,
            size: (data.len() * std::mem::size_of::<T>()) as u64,
        }
    }

    /// Create an empty buffer for output
    pub fn empty(ctx: &GpuContext, size: u64, usage: wgpu::BufferUsages) -> Self {
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("output_buffer"),
            size,
            usage,
            mapped_at_creation: false,
        });

        Self { buffer, size }
    }
}

/// Dot product compute shader (WGSL)
const DOT_PRODUCT_SHADER: &str = r#"
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
"#;

/// Cosine similarity batch compute shader
const COSINE_SIMILARITY_BATCH_SHADER: &str = r#"
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
"#;

/// GPU-accelerated cosine similarity search
pub struct CosineSimilaritySearch {
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

impl CosineSimilaritySearch {
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
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = ctx.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("cosine_similarity_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Ok(Self {
            pipeline,
            bind_group_layout,
        })
    }
}

// wgpu buffer init descriptor helper
mod wgpu {
    pub use ::wgpu::*;
    
    pub mod util {
        use super::*;
        
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
        
        buffer.slice(..).get_mapped_range_mut()[..desc.contents.len()]
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
