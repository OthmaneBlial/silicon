//! A small Vulkan-inspired Rust interface backed by SILICON's software renderer.
//! This is not the Vulkan ABI, loader, or a conformant Vulkan implementation.
use crate::api as gpu;
use std::{collections::BTreeMap, sync::Arc};

pub const API_VERSION: u32 = 1;

pub struct Instance;
impl Instance {
    pub const fn new() -> Self {
        Self
    }
    pub fn enumerate_physical_devices(&self) -> Vec<PhysicalDevice> {
        vec![PhysicalDevice]
    }
}
impl Default for Instance {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PhysicalDevice;
impl PhysicalDevice {
    pub const fn name(self) -> &'static str {
        "SILICON CPU device"
    }
    pub fn create_device(self, width: u32, height: u32) -> gpu::Result<LogicalDevice> {
        Ok(LogicalDevice {
            core: gpu::Device::new(),
            renderer: gpu::Renderer::new(width, height)?,
        })
    }
}

pub struct LogicalDevice {
    core: gpu::Device,
    renderer: gpu::Renderer,
}
impl LogicalDevice {
    pub fn create_shader_module(&self, spirv: &[u8]) -> Result<ShaderModule, String> {
        self.core.create_shader(spirv).map(ShaderModule)
    }
    pub fn create_graphics_pipeline(
        &self,
        vertex: &ShaderModule,
        fragment: &ShaderModule,
        state: gpu::Pipeline,
    ) -> Result<GraphicsPipeline, String> {
        self.core
            .create_pipeline(&vertex.0, &fragment.0, state)
            .map(GraphicsPipeline)
    }
    pub fn create_vertex_buffer(&self, data: Vec<gpu::Vertex>) -> gpu::Result<Buffer> {
        self.core
            .create_vertex_buffer(data)
            .map(BufferResource::Vertex)
            .map(|resource| Buffer { resource })
    }
    pub fn create_index_buffer(&self, data: Vec<u32>) -> gpu::Result<Buffer> {
        self.core
            .create_index_buffer(data)
            .map(BufferResource::Index)
            .map(|resource| Buffer { resource })
    }
    pub fn create_uniform_buffer(&self, data: Vec<gpu::Vec4>) -> gpu::Result<Buffer> {
        self.core
            .create_uniform_buffer(data)
            .map(BufferResource::Uniform)
            .map(|resource| Buffer { resource })
    }
    pub fn create_image_rgba8(&self, width: u32, height: u32, pixels: &[u8]) -> gpu::Result<Image> {
        gpu::Texture::new(width, height, gpu::TextureFormat::Rgba8, pixels)
            .map(Arc::new)
            .map(Image)
    }
    pub fn create_descriptor_set(&self) -> DescriptorSet {
        DescriptorSet::default()
    }
    pub fn create_render_pass(&self, clear_color: gpu::Color) -> RenderPass {
        let mut clear_colors = [gpu::Color::BLACK; gpu::MAX_COLOR_ATTACHMENTS];
        clear_colors[0] = clear_color;
        RenderPass {
            clear_colors,
            color_count: 1,
        }
    }
    pub fn create_render_pass_with_colors(
        &self,
        clear_colors: &[gpu::Color],
    ) -> gpu::Result<RenderPass> {
        if !(1..=gpu::MAX_COLOR_ATTACHMENTS).contains(&clear_colors.len())
            || clear_colors.iter().any(|color| !color.0.is_finite())
        {
            return Err(format!(
                "render pass requires 1..={} finite clear colors",
                gpu::MAX_COLOR_ATTACHMENTS
            )
            .into());
        }
        let mut colors = [gpu::Color::BLACK; gpu::MAX_COLOR_ATTACHMENTS];
        colors[..clear_colors.len()].copy_from_slice(clear_colors);
        Ok(RenderPass {
            clear_colors: colors,
            color_count: clear_colors.len(),
        })
    }
    pub fn create_command_buffer(&self) -> CommandBuffer {
        CommandBuffer(self.core.commands())
    }
    pub fn graphics_queue(&mut self) -> Queue<'_> {
        Queue {
            core: &self.core,
            renderer: &mut self.renderer,
        }
    }
    pub fn framebuffer(&self) -> &gpu::Framebuffer {
        &self.renderer.framebuffer
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferUsage {
    Vertex,
    Index,
    Uniform,
}

#[derive(Clone)]
enum BufferResource {
    Vertex(gpu::Buffer<gpu::Vertex>),
    Index(gpu::Buffer<u32>),
    Uniform(gpu::Buffer<gpu::Vec4>),
}

#[derive(Clone)]
pub struct Buffer {
    resource: BufferResource,
}
impl Buffer {
    pub fn usage(&self) -> BufferUsage {
        match &self.resource {
            BufferResource::Vertex(_) => BufferUsage::Vertex,
            BufferResource::Index(_) => BufferUsage::Index,
            BufferResource::Uniform(_) => BufferUsage::Uniform,
        }
    }
}

#[derive(Clone)]
pub struct Image(Arc<gpu::Texture>);

#[derive(Clone, Debug)]
pub struct ShaderModule(gpu::ShaderModule);

#[derive(Clone)]
pub struct GraphicsPipeline(Arc<gpu::ShaderPipeline>);

#[derive(Clone, Default)]
pub struct DescriptorSet {
    uniform: Option<Buffer>,
    images: BTreeMap<u8, Image>,
}
impl DescriptorSet {
    pub fn bind_uniform_buffer(&mut self, buffer: &Buffer) -> Result<(), String> {
        if buffer.usage() != BufferUsage::Uniform {
            return Err("descriptor requires a uniform buffer".into());
        }
        self.uniform = Some(buffer.clone());
        Ok(())
    }
    pub fn bind_image(&mut self, slot: u8, image: &Image) {
        self.images.insert(slot, image.clone());
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RenderPass {
    clear_colors: [gpu::Color; gpu::MAX_COLOR_ATTACHMENTS],
    color_count: usize,
}

#[derive(Clone, Default)]
pub struct CommandBuffer(gpu::CommandBuffer);
impl CommandBuffer {
    pub fn begin_render_pass(&mut self, render_pass: RenderPass) {
        if render_pass.color_count == 1 {
            self.0.begin_render_pass(render_pass.clear_colors[0]);
        } else {
            self.0.begin_render_pass_with_colors(
                render_pass.clear_colors[..render_pass.color_count].to_vec(),
            );
        }
    }
    pub fn bind_pipeline(&mut self, pipeline: &GraphicsPipeline) {
        self.0.bind_pipeline(Arc::clone(&pipeline.0));
    }
    pub fn bind_vertex_buffer(&mut self, buffer: &Buffer) -> Result<(), String> {
        let BufferResource::Vertex(buffer) = &buffer.resource else {
            return Err("vertex input requires a vertex buffer".into());
        };
        self.0.bind_vertex_buffer(buffer.clone());
        Ok(())
    }
    pub fn bind_index_buffer(&mut self, buffer: &Buffer) -> Result<(), String> {
        let BufferResource::Index(buffer) = &buffer.resource else {
            return Err("indexed draw requires an index buffer".into());
        };
        self.0.bind_index_buffer(buffer.clone());
        Ok(())
    }
    pub fn bind_descriptor_set(&mut self, descriptors: &DescriptorSet) -> Result<(), String> {
        if let Some(buffer) = &descriptors.uniform {
            let BufferResource::Uniform(buffer) = &buffer.resource else {
                return Err("descriptor uniform binding has the wrong buffer usage".into());
            };
            self.0.bind_uniform_buffer(buffer.clone());
        }
        for (slot, image) in &descriptors.images {
            self.0
                .bind_texture(*slot, Arc::clone(&image.0), gpu::Sampler::default());
        }
        Ok(())
    }
    pub fn draw(&mut self, first_vertex: u32, vertex_count: u32) {
        self.0.draw(first_vertex, vertex_count);
    }
    pub fn draw_indexed(&mut self, first_index: u32, index_count: u32) {
        self.0.draw_indexed(first_index, index_count);
    }
    pub fn end_render_pass(&mut self) {
        self.0.end_render_pass();
    }
    pub fn validate(&self) -> gpu::Result<()> {
        self.0.validate()
    }
}

pub struct Queue<'a> {
    core: &'a gpu::Device,
    renderer: &'a mut gpu::Renderer,
}
impl Queue<'_> {
    pub fn submit(&mut self, commands: &CommandBuffer) -> gpu::Result<gpu::Submission> {
        self.core.submit(&commands.0, self.renderer)
    }
}
