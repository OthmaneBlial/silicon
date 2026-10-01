//! Owned command resources use Arc lifetimes. No borrowed guest pointers or handles.
use crate::*;
use serde::{Deserialize, Serialize};
use silicon_shader::{Instruction, Program};
use std::{
    collections::HashMap,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
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
#[derive(Clone, Copy, PartialEq, Eq)]
enum ImageKind {
    Texture2D,
    CubeMap,
}
#[derive(Clone, Copy)]
enum BoundImage<'a> {
    Texture2D(&'a Texture, Sampler),
    CubeMap(&'a CubeMap, Sampler),
}
#[derive(Clone, Debug)]
pub struct ShaderModule {
    compiled: shader::spirv::Compiled,
}
impl ShaderModule {
    pub fn stage(&self) -> shader::spirv::Stage {
        self.compiled.stage
    }
}
impl ShaderPipeline {
    pub(crate) fn from_spirv(
        vertex: &[u8],
        fragment: &[u8],
        state: Pipeline,
    ) -> shader::Result<Arc<Self>> {
        let device = Device::new();
        let vertex = device.create_shader(vertex).map_err(|e| e.to_string())?;
        let fragment = device.create_shader(fragment).map_err(|e| e.to_string())?;
        device
            .create_pipeline(&vertex, &fragment, state)
            .map_err(|e| e.to_string())
    }
}
#[derive(Clone, Hash, PartialEq, Eq)]
struct PipelineCacheKey {
    vertex: Vec<u8>,
    fragment: Vec<u8>,
    state: Pipeline,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct PipelineCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: usize,
    pub lookup_time: Duration,
    pub compile_time: Duration,
}
/// A caller-owned cache for linked, translated SPIR-V pipelines.
/// ponytail: fixed 16-entry arbitrary eviction; add LRU only if hit rates demand it.
#[derive(Default)]
pub struct PipelineCache {
    entries: HashMap<PipelineCacheKey, Arc<ShaderPipeline>>,
    stats: PipelineCacheStats,
}
pub const PIPELINE_CACHE_CAPACITY: usize = 16;
impl PipelineCache {
    pub fn get_or_compile(
        &mut self,
        vertex: &[u8],
        fragment: &[u8],
        state: Pipeline,
    ) -> Result<Arc<ShaderPipeline>> {
        if vertex.len() > 1024 * 1024 || fragment.len() > 1024 * 1024 {
            return Err("pipeline cache accepts SPIR-V modules up to 1 MiB each".into());
        }
        let start = Instant::now();
        let key = PipelineCacheKey {
            vertex: vertex.to_vec(),
            fragment: fragment.to_vec(),
            state,
        };
        if let Some(pipeline) = self.entries.get(&key) {
            self.stats.hits = self.stats.hits.saturating_add(1);
            self.stats.lookup_time = self.stats.lookup_time.saturating_add(start.elapsed());
            return Ok(Arc::clone(pipeline));
        }
        self.stats.misses = self.stats.misses.saturating_add(1);
        self.stats.lookup_time = self.stats.lookup_time.saturating_add(start.elapsed());
        let compile_start = Instant::now();
        let compiled = ShaderPipeline::from_spirv(vertex, fragment, state)
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() });
        self.stats.compile_time = self
            .stats
            .compile_time
            .saturating_add(compile_start.elapsed());
        let pipeline = compiled?;
        if self.entries.len() == PIPELINE_CACHE_CAPACITY
            && let Some(candidate) = self.entries.keys().next().cloned()
        {
            self.entries.remove(&candidate);
            self.stats.evictions = self.stats.evictions.saturating_add(1);
        }
        self.entries.insert(key, Arc::clone(&pipeline));
        self.stats.entries = self.entries.len();
        Ok(pipeline)
    }
    pub fn stats(&self) -> PipelineCacheStats {
        self.stats
    }
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
    BindCubeMap {
        slot: u8,
        cube_map: Arc<CubeMap>,
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
    pub const fn new() -> Self {
        Self
    }
    /// Parses and translates a SPIR-V 1.0 module in SILICON's supported graphics subset.
    pub fn create_shader(&self, spirv: &[u8]) -> shader::Result<ShaderModule> {
        let compiled = shader::spirv::Module::parse(spirv)?.translate()?;
        Ok(ShaderModule { compiled })
    }
    /// Links one vertex and one fragment module into an immutable pipeline.
    pub fn create_pipeline(
        &self,
        vertex: &ShaderModule,
        fragment: &ShaderModule,
        state: Pipeline,
    ) -> shader::Result<Arc<ShaderPipeline>> {
        shader::spirv::link(&vertex.compiled, &fragment.compiled)?;
        Ok(Arc::new(ShaderPipeline {
            state,
            vertex: vertex.compiled.program.clone(),
            fragment: fragment.compiled.program.clone(),
        }))
    }
    pub fn commands(&self) -> CommandBuffer {
        CommandBuffer::default()
    }
    pub fn create_vertex_buffer(&self, data: Vec<Vertex>) -> Result<Buffer<Vertex>> {
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
    pub fn create_index_buffer(&self, data: Vec<u32>) -> Result<Buffer<u32>> {
        if data.len() > 3_000_000 {
            return Err("index buffer exceeds 3M indices".into());
        }
        Ok(Buffer {
            data: data.into(),
            usage: BufferUsage::Index,
        })
    }
    pub fn create_uniform_buffer(&self, data: Vec<Vec4>) -> Result<Buffer<Vec4>> {
        if data.len() > 64 || data.iter().any(|v| !v.is_finite()) {
            return Err("uniform buffer requires at most 64 finite vec4s".into());
        }
        Ok(Buffer {
            data: data.into(),
            usage: BufferUsage::Uniform,
        })
    }
    pub fn submit(&self, cmd: &CommandBuffer, r: &mut Renderer) -> Result<Submission> {
        let validation_start = r.profile_shaders.then(Instant::now);
        cmd.validate()?;
        let mut command_processing_time =
            validation_start.map_or(Duration::ZERO, |start| start.elapsed());
        let mut pipeline = None;
        let mut vertices = None;
        let mut indices = None;
        let mut uniforms = None;
        let mut textures: [Option<BoundImage<'_>>; 16] = [None; 16];
        let mut stats = Submission::default();
        for (number, command) in cmd.commands.iter().enumerate() {
            let mut command_start = r.profile_shaders.then(Instant::now);
            let is_draw = matches!(command, Command::Draw { .. });
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
                } => {
                    textures[*slot as usize] =
                        Some(BoundImage::Texture2D(texture.as_ref(), *sampler));
                }
                Command::BindCubeMap {
                    slot,
                    cube_map,
                    sampler,
                } => {
                    textures[*slot as usize] =
                        Some(BoundImage::CubeMap(cube_map.as_ref(), *sampler));
                }
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
                        let image = textures
                            .get(slot)
                            .copied()
                            .flatten()
                            .ok_or_else(|| format!("texture slot {slot} is not bound"))?;
                        match image {
                            BoundImage::Texture2D(texture, sampler) => texture
                                .sample(Vec2::new(uv.x, uv.y), uv.z, sampler)
                                .map(|c| c.0)
                                .map_err(|e| e.to_string()),
                            BoundImage::CubeMap(cube_map, sampler) => cube_map
                                .sample(Vec3::new(uv.x, uv.y, uv.z), uv.w, sampler)
                                .map(|c| c.0)
                                .map_err(|e| e.to_string()),
                        }
                    };
                    let previous = r.stats.shaded;
                    let debug = r.debug_pixel;
                    let shader_traces = std::cell::RefCell::new(Vec::new());
                    let simd = r.backend == Backend::Simd;
                    let mut implicit_cube_slots = [false; 16];
                    for instruction in p.fragment.instructions() {
                        if let Instruction::SampleCubeImplicit { texture, .. } = instruction {
                            implicit_cube_slots[*texture as usize] = true;
                        }
                    }
                    let packets = std::cell::Cell::new(0u64);
                    let instructions = std::cell::Cell::new(0u64);
                    let samples = std::cell::Cell::new(0u64);
                    if let Some(start) = command_start.take() {
                        command_processing_time += start.elapsed();
                    }
                    r.try_draw_packets(
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
                            instructions.set(instructions.get() + e.instructions as u64);
                            samples.set(samples.get() + e.samples as u64);
                            Ok(VertexOutput {
                                position: e.outputs[0],
                                varyings: std::array::from_fn(|i| e.outputs[i + 1]),
                            })
                        },
                        |fragments, mask| {
                            let mut inputs = [[Vec4::ZERO; 4]; 4];
                            let mut lods = [[0.; 16]; 4];
                            let tracing = std::array::from_fn(|i| {
                                debug == Some((fragments[i].x, fragments[i].y))
                            });
                            for i in 0..4 {
                                if mask & (1 << i) == 0 {
                                    continue;
                                }
                                inputs[i] = fragments[i].varyings;
                                for (slot, image) in textures.iter().copied().enumerate() {
                                    lods[i][slot] = match image {
                                        Some(BoundImage::CubeMap(cube_map, sampler))
                                            if implicit_cube_slots[slot]
                                                && !matches!(sampler.mip, MipFilter::None) =>
                                        {
                                            cube_map.lod(
                                                fragments[i].varyings[1].xyz(),
                                                fragments[i].direction_dx,
                                                fragments[i].direction_dy,
                                            )
                                        }
                                        Some(BoundImage::Texture2D(texture, sampler))
                                            if !matches!(sampler.mip, MipFilter::None) =>
                                        {
                                            texture.lod(
                                                fragments[i].uv_dx,
                                                fragments[i].uv_dy,
                                            )
                                        }
                                        _ => 0.,
                                    };
                                }
                            }
                            let mut colors = [None; 4];
                            if simd {
                                let executed = p
                                    .fragment
                                    .execute4(
                                        std::array::from_fn(|i| inputs[i].as_slice()),
                                        u,
                                        std::array::from_fn(|i| lods[i].as_slice()),
                                        mask,
                                        |_, slot, uv| sample(slot, uv),
                                        tracing,
                                    )
                                    .map_err(|e| {
                                        let pixels = fragments.map(|f| (f.x, f.y));
                                        format!(
                                            "command {number}, fragment packet mask {mask:04b}, pixels {pixels:?}: {e}"
                                        )
                                    })?;
                                packets.set(packets.get() + 1);
                                for (i, e) in executed.into_iter().enumerate() {
                                    if mask & (1 << i) == 0 {
                                        continue;
                                    }
                                    instructions.set(instructions.get() + e.instructions as u64);
                                    samples.set(samples.get() + e.samples as u64);
                                    if !e.trace.is_empty() {
                                        shader_traces
                                            .borrow_mut()
                                            .push((fragments[i].primitive, e.trace));
                                    }
                                    colors[i] = (!e.discarded).then_some(Color(e.outputs[0]));
                                }
                            } else {
                                for i in 0..4 {
                                    if mask & (1 << i) == 0 {
                                        continue;
                                    }
                                    let e = p
                                        .fragment
                                        .execute_with_lod(
                                            &inputs[i], u, &lods[i], sample, tracing[i],
                                        )
                                        .map_err(|e| {
                                            format!(
                                                "command {number}, pixel {},{}, fragment shader: {e}",
                                                fragments[i].x, fragments[i].y
                                            )
                                        })?;
                                    instructions.set(instructions.get() + e.instructions as u64);
                                    samples.set(samples.get() + e.samples as u64);
                                    if !e.trace.is_empty() {
                                        shader_traces
                                            .borrow_mut()
                                            .push((fragments[i].primitive, e.trace));
                                    }
                                    colors[i] = (!e.discarded).then_some(Color(e.outputs[0]));
                                }
                            }
                            Ok(colors)
                        },
                    )?;
                    r.stats.shader_packets += packets.get();
                    if simd {
                        r.stats.shader_packet_lanes += r.stats.shaded - previous;
                    }
                    stats.shader_traces.extend(shader_traces.into_inner());
                    stats.draws += 1;
                    stats.shader_instructions += instructions.get();
                    stats.texture_samples += samples.get();
                    r.stats.shader_instructions += instructions.get();
                    r.stats.texture_samples += samples.get();
                }
            }
            if !is_draw && let Some(start) = command_start {
                command_processing_time += start.elapsed();
            }
        }
        r.stats.command_processing_time += command_processing_time;
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
    pub fn bind_cube_map(&mut self, slot: u8, cube_map: Arc<CubeMap>, sampler: Sampler) {
        self.commands.push(Command::BindCubeMap {
            slot,
            cube_map,
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
        let mut bound_pipeline: Option<&ShaderPipeline> = None;
        let mut image_kinds = [None; 16];
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
                Command::BindPipeline(pipeline) => {
                    if pipeline
                        .vertex
                        .instructions()
                        .iter()
                        .chain(pipeline.fragment.instructions())
                        .any(|op| {
                            matches!(
                                op,
                                Instruction::StorageLoad { .. } | Instruction::StorageStore { .. }
                            )
                        })
                    {
                        return Err(error(
                            "compute storage instructions are not supported by graphics pipelines",
                        )
                        .into());
                    }
                    if pipeline
                        .vertex
                        .instructions()
                        .iter()
                        .any(|op| matches!(op, Instruction::Discard))
                    {
                        return Err(error("discard is only allowed in fragment shaders").into());
                    }
                    bound_pipeline = Some(pipeline.as_ref());
                }
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
                    image_kinds[*slot as usize] = Some(ImageKind::Texture2D);
                }
                Command::BindCubeMap { slot, cube_map, .. } => {
                    if *slot >= 16 {
                        return Err(error("texture slot exceeds 15").into());
                    }
                    cube_map.validate()?;
                    image_kinds[*slot as usize] = Some(ImageKind::CubeMap);
                }
                Command::Draw {
                    first,
                    count,
                    indexed,
                } => {
                    let v = vertex.ok_or_else(|| error("draw has no vertex buffer"))?;
                    let bound = bound_pipeline.ok_or_else(|| error("draw has no pipeline"))?;
                    for instruction in bound
                        .vertex
                        .instructions()
                        .iter()
                        .chain(bound.fragment.instructions())
                    {
                        let (slot, expected) = match instruction {
                            Instruction::Sample { texture, .. }
                            | Instruction::SampleImplicit { texture, .. } => {
                                (*texture, ImageKind::Texture2D)
                            }
                            Instruction::SampleCube { texture, .. }
                            | Instruction::SampleCubeImplicit { texture, .. } => {
                                (*texture, ImageKind::CubeMap)
                            }
                            _ => continue,
                        };
                        if image_kinds[slot as usize] != Some(expected) {
                            return Err(
                                error("shader sampler and bound image types do not match").into()
                            );
                        }
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
