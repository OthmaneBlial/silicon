//! Owned command resources use Arc lifetimes. No borrowed guest pointers or handles.
use crate::*;
use serde::{Deserialize, Serialize};
use silicon_shader::{Instruction, Program};
use std::{path::Path, sync::Arc};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum BufferUsage {
    Vertex,
    Index,
    Uniform,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Buffer<T> {
    data: Arc<[T]>,
    usage: BufferUsage,
}
impl<T> Buffer<T> {
    pub fn mapped(&self) -> &[T] {
        &self.data
    }
    pub fn size(&self) -> usize {
        std::mem::size_of_val(&*self.data)
    }
    pub fn alignment(&self) -> usize {
        std::mem::align_of::<T>()
    }
    pub fn usage(&self) -> BufferUsage {
        self.usage
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShaderPipeline {
    pub state: Pipeline,
    pub vertex: Program,
    pub fragment: Program,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Command {
    BeginRenderPass {
        clear: Color,
    },
    EndRenderPass,
    BindPipeline(Arc<ShaderPipeline>),
    BindVertices(Buffer<Vertex>),
    BindIndices(Buffer<u32>),
    BindUniforms(Buffer<Vec4>),
    BindTexture {
        slot: u8,
        texture: Arc<Texture>,
        sampler: Sampler,
    },
    Draw {
        first: u32,
        count: u32,
        indexed: bool,
    },
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CommandBuffer {
    commands: Vec<Command>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Device;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameCapture {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub commands: CommandBuffer,
}
#[derive(Clone, Debug, Default)]
pub struct Submission {
    pub shader_traces: Vec<(u32, Vec<shader::Trace>)>,
    pub draws: u64,
    pub shader_instructions: u64,
    pub texture_samples: u64,
}
impl Device {
    pub fn commands(self) -> CommandBuffer {
        CommandBuffer::default()
    }
    pub fn create_vertex_buffer(self, data: Vec<Vertex>) -> Result<Buffer<Vertex>> {
        if data.len() > 1_000_000
            || data.iter().any(|v| {
                !v.position.is_finite()
                    || !v.normal.is_finite()
                    || !v.uv.is_finite()
                    || !v.color.is_finite()
            })
        {
            return Err("invalid vertex buffer: requires at most 1M finite vertices".into());
        }
        Ok(Buffer {
            data: data.into(),
            usage: BufferUsage::Vertex,
        })
    }
    pub fn create_index_buffer(self, data: Vec<u32>) -> Result<Buffer<u32>> {
        if data.len() > 3_000_000 {
            return Err("index buffer exceeds 3M indices".into());
        }
        Ok(Buffer {
            data: data.into(),
            usage: BufferUsage::Index,
        })
    }
    pub fn create_uniform_buffer(self, data: Vec<Vec4>) -> Result<Buffer<Vec4>> {
        if data.len() > 64 || data.iter().any(|v| !v.is_finite()) {
            return Err("uniform buffer requires at most 64 finite vec4s".into());
        }
        Ok(Buffer {
            data: data.into(),
            usage: BufferUsage::Uniform,
        })
    }
    pub fn submit(self, cmd: &CommandBuffer, r: &mut Renderer) -> Result<Submission> {
        cmd.validate()?;
        let mut pipeline = None;
        let mut vertices = None;
        let mut indices = None;
        let mut uniforms = None;
        let mut textures: [Option<(&Texture, Sampler)>; 16] = [None; 16];
        let mut stats = Submission::default();
        for (number, command) in cmd.commands.iter().enumerate() {
            match command {
                Command::BeginRenderPass { clear } => r.clear(*clear),
                Command::EndRenderPass => {}
                Command::BindPipeline(p) => pipeline = Some(p.as_ref()),
                Command::BindVertices(b) => vertices = Some(b.mapped()),
                Command::BindIndices(b) => indices = Some(b.mapped()),
                Command::BindUniforms(b) => uniforms = Some(b.mapped()),
                Command::BindTexture {
                    slot,
                    texture,
                    sampler,
                } => textures[*slot as usize] = Some((texture.as_ref(), *sampler)),
                Command::Draw {
                    first,
                    count,
                    indexed,
                } => {
                    let p = pipeline.unwrap();
                    let v = vertices.unwrap();
                    let u = uniforms.unwrap_or(&[]);
                    let range = *first as usize..(*first as usize + *count as usize);
                    let ind = if *indexed {
                        Some(&indices.unwrap()[range.clone()])
                    } else {
                        None
                    };
                    let v = if *indexed { v } else { &v[range] };
                    let sample = |slot: usize, uv: Vec4| -> silicon_shader::Result<Vec4> {
                        let (t, s) = textures
                            .get(slot)
                            .copied()
                            .flatten()
                            .ok_or_else(|| format!("texture slot {slot} is not bound"))?;
                        t.sample(Vec2::new(uv.x, uv.y), uv.z, s)
                            .map(|c| c.0)
                            .map_err(|e| e.to_string())
                    };
                    let previous = r.stats.shaded;
                    let debug = r.debug_pixel;
                    let shader_traces = std::cell::RefCell::new(Vec::new());
                    r.try_draw(
                        v,
                        ind,
                        p.state,
                        |v| {
                            let e = p
                                .vertex
                                .execute(
                                    &[
                                        v.position.extend(1.),
                                        v.color,
                                        Vec4::new(v.uv.x, v.uv.y, 0., 0.),
                                        v.normal.extend(0.),
                                    ],
                                    u,
                                    sample,
                                    false,
                                )
                                .map_err(|e| format!("command {number}, vertex shader: {e}"))?;
                            Ok(VertexOutput {
                                position: e.outputs[0],
                                varyings: std::array::from_fn(|i| e.outputs[i + 1]),
                            })
                        },
                        |f| {
                            let mut input = f.varyings;
                            let lods =
                                textures.map(|t| t.map_or(0., |(t, _)| t.lod(f.uv_dx, f.uv_dy)));
                            input[1].z = lods[0];
                            let e = p
                                .fragment
                                .execute_with_lod(
                                    &input,
                                    u,
                                    &lods,
                                    sample,
                                    debug == Some((f.x, f.y)),
                                )
                                .map_err(|e| {
                                    format!(
                                        "command {number}, pixel {},{}, fragment shader: {e}",
                                        f.x, f.y
                                    )
                                })?;
                            if !e.trace.is_empty() {
                                shader_traces.borrow_mut().push((f.primitive, e.trace));
                            }
                            Ok(Some(Color(e.outputs[0])))
                        },
                    )?;
                    stats.shader_traces.extend(shader_traces.into_inner());
                    let shaded = r.stats.shaded - previous;
                    let count_samples = |p: &Program| {
                        p.instructions()
                            .iter()
                            .filter(|op| {
                                matches!(
                                    op,
                                    Instruction::Sample { .. } | Instruction::SampleImplicit { .. }
                                )
                            })
                            .count() as u64
                    };
                    stats.draws += 1;
                    stats.shader_instructions += v.len() as u64
                        * p.vertex.instructions().len() as u64
                        + shaded * p.fragment.instructions().len() as u64;
                    stats.texture_samples += v.len() as u64 * count_samples(&p.vertex)
                        + shaded * count_samples(&p.fragment);
                }
            }
        }
        Ok(stats)
    }
}
impl CommandBuffer {
    pub fn begin_render_pass(&mut self, clear: Color) {
        self.commands.push(Command::BeginRenderPass { clear });
    }
    pub fn end_render_pass(&mut self) {
        self.commands.push(Command::EndRenderPass);
    }
    pub fn bind_pipeline(&mut self, pipeline: Arc<ShaderPipeline>) {
        self.commands.push(Command::BindPipeline(pipeline));
    }
    pub fn bind_vertex_buffer(&mut self, buffer: Buffer<Vertex>) {
        self.commands.push(Command::BindVertices(buffer));
    }
    pub fn bind_index_buffer(&mut self, buffer: Buffer<u32>) {
        self.commands.push(Command::BindIndices(buffer));
    }
    pub fn bind_uniform_buffer(&mut self, buffer: Buffer<Vec4>) {
        self.commands.push(Command::BindUniforms(buffer));
    }
    pub fn bind_texture(&mut self, slot: u8, texture: Arc<Texture>, sampler: Sampler) {
        self.commands.push(Command::BindTexture {
            slot,
            texture,
            sampler,
        });
    }
    pub fn draw(&mut self, first: u32, count: u32) {
        self.commands.push(Command::Draw {
            first,
            count,
            indexed: false,
        });
    }
    pub fn draw_indexed(&mut self, first: u32, count: u32) {
        self.commands.push(Command::Draw {
            first,
            count,
            indexed: true,
        });
    }
    pub fn stream(&self) -> &[Command] {
        &self.commands
    }
    pub fn validate(&self) -> Result<()> {
        if self.commands.len() > 65536 {
            return Err("command buffer exceeds 65536 commands".into());
        }
        let mut pass = false;
        let mut begins = 0;
        let mut p = false;
        let mut vertex: Option<&[Vertex]> = None;
        let mut index: Option<&[u32]> = None;
        for (number, cmd) in self.commands.iter().enumerate() {
            let error = |s: &str| format!("command {number}: {s}");
            match cmd {
                Command::BeginRenderPass { clear } => {
                    if pass || begins > 0 {
                        return Err(error(
                            "only one non-nested render pass is currently supported",
                        )
                        .into());
                    }
                    if !clear.0.is_finite() {
                        return Err(error("clear color must be finite").into());
                    }
                    pass = true;
                    begins += 1;
                }
                Command::EndRenderPass => {
                    if !pass {
                        return Err(error("end without begin render pass").into());
                    }
                    pass = false;
                }
                _ if !pass => return Err(error("command must be inside a render pass").into()),
                Command::BindPipeline(_) => p = true,
                Command::BindVertices(b) => {
                    Device.create_vertex_buffer(b.data.to_vec())?;
                    if b.usage != BufferUsage::Vertex {
                        return Err(error("vertex buffer usage mismatch").into());
                    }
                    vertex = Some(b.mapped());
                }
                Command::BindIndices(b) => {
                    if b.data.len() > 3_000_000 || b.usage != BufferUsage::Index {
                        return Err(error("invalid index buffer").into());
                    }
                    index = Some(b.mapped());
                }
                Command::BindUniforms(b) => {
                    Device.create_uniform_buffer(b.data.to_vec())?;
                    if b.usage != BufferUsage::Uniform {
                        return Err(error("uniform buffer usage mismatch").into());
                    }
                }
                Command::BindTexture { slot, texture, .. } => {
                    if *slot >= 16 {
                        return Err(error("texture slot exceeds 15").into());
                    }
                    texture.validate()?;
                }
                Command::Draw {
                    first,
                    count,
                    indexed,
                } => {
                    let v = vertex.ok_or_else(|| error("draw has no vertex buffer"))?;
                    if !p {
                        return Err(error("draw has no pipeline").into());
                    }
                    if !count.is_multiple_of(3) {
                        return Err(error("triangle draw count must be a multiple of 3").into());
                    }
                    let end = (*first as usize)
                        .checked_add(*count as usize)
                        .ok_or_else(|| error("draw range overflow"))?;
                    if *indexed {
                        let ind = index.ok_or_else(|| error("indexed draw has no index buffer"))?;
                        if end > ind.len()
                            || ind[*first as usize..end]
                                .iter()
                                .any(|&i| i as usize >= v.len())
                        {
                            return Err(error("out-of-bounds index/vertex access").into());
                        }
                    } else if end > v.len() {
                        return Err(error("draw range exceeds vertex buffer").into());
                    }
                }
            }
        }
        if pass || begins == 0 {
            return Err("command buffer requires a complete render pass".into());
        }
        Ok(())
    }
}
impl FrameCapture {
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.commands.validate()?;
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec(self)?)?;
        Ok(())
    }
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        use std::io::Read;
        let file = std::fs::File::open(path)?;
        if file.metadata()?.len() > 64 * 1024 * 1024 {
            return Err("capture exceeds 64 MiB".into());
        }
        let mut bytes = Vec::new();
        file.take(64 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("capture exceeds 64 MiB".into());
        }
        let capture: Self = serde_json::from_slice(&bytes)?;
        if capture.version != 1 {
            return Err("unsupported capture version".into());
        }
        Framebuffer::new(capture.width, capture.height)?;
        capture.commands.validate()?;
        Ok(capture)
    }
    pub fn replay(&self) -> Result<Renderer> {
        if self.version != 1 {
            return Err("unsupported capture version".into());
        }
        let mut r = Renderer::new(self.width, self.height)?;
        Device.submit(&self.commands, &mut r)?;
        Ok(r)
    }
}
