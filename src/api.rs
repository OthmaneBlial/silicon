//! Supported Rust API for applications that embed SILICON.
pub use silicon_core::{
    Address, Blend, Buffer, BufferUsage, Color, Command, CommandBuffer, Compare, ComputePipeline,
    ComputeStats, Cull, Device, Filter, FrameCapture, Framebuffer, FrontFace, MipFilter, Pipeline,
    PipelineCache, PipelineCacheStats, Renderer, Result, SampleCount, Sampler, ShaderModule,
    ShaderPipeline, StencilOp, StencilState, StorageBuffer, StorageLayout, Submission, Texture,
    Texture3D, TextureArray, TextureFormat, Vec2, Vec3, Vec4, Vertex,
};

/// Version of the supported `silicon::api` surface, independent of the crate release version.
pub const API_VERSION: u32 = 1;
