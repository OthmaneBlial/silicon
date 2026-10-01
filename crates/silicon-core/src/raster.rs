use crate::*;
use std::time::{Duration, Instant};
const SUBPIXEL: i64 = 256;
const TILE: u32 = 16;
// ponytail: cap bins at 256 triangles, 1M references and 262k tiles; raise only if profiling warrants it.
const MAX_BINNED_TILES: usize = 262_144;
const MAX_BINNED_TRIANGLES: usize = 256;
const MAX_TILE_REFERENCES: usize = 1_048_576;
const SAMPLE_2X: [(i64, i64); 2] = [(64, 64), (192, 192)];
const SAMPLE_4X: [(i64, i64); 4] = [(96, 32), (224, 96), (32, 160), (160, 224)];
type ColorOutputs = [Option<Color>; MAX_COLOR_ATTACHMENTS];
#[derive(Clone, Debug, Default)]
pub struct Statistics {
    pub command_processing_time: Duration,
    pub primitive_setup_time: Duration,
    pub rasterization_time: Duration,
    pub blend_write_time: Duration,
    pub shader_time: Duration,
    pub shader_packets: u64,
    pub shader_packet_lanes: u64,
    pub shader_instructions: u64,
    pub texture_samples: u64,
    pub discarded: u64,
    pub vertices: u64,
    pub triangles: u64,
    pub clipped: u64,
    pub culled: u64,
    pub tiles: u64,
    pub fragments: u64,
    pub early_z_rejected: u64,
    pub stencil_rejected: u64,
    pub shaded: u64,
    pub vertex_time: Duration,
    pub raster_time: Duration,
}
impl std::ops::AddAssign<&Self> for Statistics {
    fn add_assign(&mut self, other: &Self) {
        self.command_processing_time += other.command_processing_time;
        self.primitive_setup_time += other.primitive_setup_time;
        self.rasterization_time += other.rasterization_time;
        self.blend_write_time += other.blend_write_time;
        self.shader_time += other.shader_time;
        self.shader_packets += other.shader_packets;
        self.shader_packet_lanes += other.shader_packet_lanes;
        self.shader_instructions += other.shader_instructions;
        self.texture_samples += other.texture_samples;
        self.discarded += other.discarded;
        self.vertices += other.vertices;
        self.triangles += other.triangles;
        self.clipped += other.clipped;
        self.culled += other.culled;
        self.tiles += other.tiles;
        self.fragments += other.fragments;
        self.early_z_rejected += other.early_z_rejected;
        self.stencil_rejected += other.stencil_rejected;
        self.shaded += other.shaded;
        self.vertex_time += other.vertex_time;
        self.raster_time += other.raster_time;
    }
}
#[derive(Clone, Debug)]
pub struct PixelTrace {
    pub fragment: Fragment,
    pub output: Option<Color>,
    pub previous_depth: f32,
    pub depth_pass: bool,
    pub stencil_pass: bool,
}
pub struct Renderer {
    pub profile_shaders: bool,
    row_offset: u32,
    viewport_height: u32,
    pub backend: Backend,
    pub framebuffer: Framebuffer,
    pub stats: Statistics,
    pub debug_pixel: Option<(u32, u32)>,
    pub traces: Vec<PixelTrace>,
}
#[derive(Clone, Copy)]
struct ScreenVertex {
    source: usize,
    x: i64,
    y: i64,
    z: f32,
    inv_w: f32,
    varyings: [Vec4; 4],
}
#[derive(Clone, Copy)]
struct TriangleSetup {
    primitive: u32,
    vertices: [ScreenVertex; 3],
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
    inclusive: [bool; 3],
    offsets: [[i64; 3]; 4],
    inv_area: f32,
    dx: [f32; 3],
    dy: [f32; 3],
}
#[derive(Clone, Copy)]
struct PreparedFragment {
    input: Fragment,
    index: usize,
    previous_depth: f32,
    stencil_pass: bool,
    depth_pass: bool,
    passing_samples: u8,
    sample_depths: [f32; 4],
    debug: bool,
}
fn edge(a: ScreenVertex, b: ScreenVertex, x: i64, y: i64) -> i64 {
    (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x)
}
fn top_left(a: ScreenVertex, b: ScreenVertex) -> bool {
    b.y < a.y || (b.y == a.y && b.x > a.x)
}
impl Renderer {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        Ok(Self {
            profile_shaders: false,
            backend: Backend::Scalar,
            row_offset: 0,
            viewport_height: height,
            framebuffer: Framebuffer::new(width, height)?,
            stats: Statistics::default(),
            debug_pixel: None,
            traces: Vec::new(),
        })
    }
    /// Select single-sample, 2× or 4× attachments before drawing.
    pub fn set_sample_count(&mut self, count: SampleCount) -> Result<()> {
        self.framebuffer.set_sample_count(count)
    }
    pub fn sample_count(&self) -> SampleCount {
        self.framebuffer.sample_count()
    }
    pub fn clear(&mut self, color: Color) {
        self.framebuffer.clear(color);
        self.framebuffer.clear_depth(1.);
        self.framebuffer.clear_stencil(0);
        self.stats = Statistics::default();
        self.traces.clear();
    }
    pub(crate) fn clear_color_attachments(&mut self, colors: &[Color]) -> Result<()> {
        self.framebuffer.clear_attachments(colors)?;
        self.framebuffer.clear_depth(1.);
        self.framebuffer.clear_stencil(0);
        self.stats = Statistics::default();
        self.traces.clear();
        Ok(())
    }
    fn record_primitive_setup(&mut self, start: Option<Instant>) {
        if let Some(start) = start {
            self.stats.primitive_setup_time += start.elapsed();
        }
    }
    pub fn draw<V, F>(
        &mut self,
        vertices: &[Vertex],
        indices: Option<&[u32]>,
        pipeline: Pipeline,
        vertex: V,
        fragment: F,
    ) -> Result<()>
    where
        V: Fn(&Vertex) -> VertexOutput,
        F: Fn(&Fragment) -> Option<Color>,
    {
        self.try_draw(
            vertices,
            indices,
            pipeline,
            |v| Ok(vertex(v)),
            |f| Ok(fragment(f)),
        )
    }
    pub fn try_draw<V, F>(
        &mut self,
        vertices: &[Vertex],
        indices: Option<&[u32]>,
        pipeline: Pipeline,
        vertex: V,
        fragment: F,
    ) -> Result<()>
    where
        V: Fn(&Vertex) -> Result<VertexOutput>,
        F: Fn(&Fragment) -> Result<Option<Color>>,
    {
        self.try_draw_packets(vertices, indices, pipeline, vertex, |inputs, mask| {
            let mut colors = [None; 4];
            for i in 0..4 {
                if mask & (1 << i) != 0 {
                    colors[i] = fragment(&inputs[i])?;
                }
            }
            Ok(colors)
        })
    }
    /// Four adjacent fragments, masked after coverage/stencil/depth testing.
    /// Each active lane owns a distinct pixel; the shader must respect `mask`.
    pub fn try_draw_packets<V, F>(
        &mut self,
        vertices: &[Vertex],
        indices: Option<&[u32]>,
        pipeline: Pipeline,
        vertex: V,
        fragment: F,
    ) -> Result<()>
    where
        V: Fn(&Vertex) -> Result<VertexOutput>,
        F: Fn(&[Fragment; 4], u8) -> Result<[Option<Color>; 4]>,
    {
        self.try_draw_packets_with_outputs(vertices, indices, pipeline, vertex, |inputs, mask| {
            Ok(fragment(inputs, mask)?.map(|color| {
                color.map(|color| {
                    let mut outputs = [None; MAX_COLOR_ATTACHMENTS];
                    outputs[0] = Some(color);
                    outputs
                })
            }))
        })
    }
    pub(crate) fn try_draw_packets_with_outputs<V, F>(
        &mut self,
        vertices: &[Vertex],
        indices: Option<&[u32]>,
        pipeline: Pipeline,
        vertex: V,
        fragment: F,
    ) -> Result<()>
    where
        V: Fn(&Vertex) -> Result<VertexOutput>,
        F: Fn(&[Fragment; 4], u8) -> Result<[Option<ColorOutputs>; 4]>,
    {
        let count = indices.map_or(vertices.len(), |i| i.len());
        if !count.is_multiple_of(3) {
            return Err("triangle list draw requires a multiple of 3 vertices/indices".into());
        }
        if indices.is_some_and(|i| i.iter().any(|&n| n as usize >= vertices.len())) {
            return Err("draw index outside vertex buffer".into());
        }
        let start = Instant::now();
        let transformed: Vec<_> = vertices.iter().map(vertex).collect::<Result<Vec<_>>>()?;
        if !transformed.iter().all(VertexOutput::is_finite) {
            return Err("vertex shader produced a non-finite output".into());
        }
        self.stats.vertices += vertices.len() as u64;
        self.stats.vertex_time += start.elapsed();
        let start = Instant::now();
        let render_result = (|| {
            let tiles_x = self.framebuffer.width.div_ceil(TILE);
            let tile_count = (tiles_x as usize)
                .checked_mul(self.framebuffer.height.div_ceil(TILE) as usize)
                .ok_or("tile grid size overflow")?;
            let mut bins = if tile_count <= MAX_BINNED_TILES {
                let mut bins = Vec::new();
                bins.try_reserve_exact(tile_count)
                    .map_err(|_| "could not allocate tile bins")?;
                bins.resize_with(tile_count, Vec::new);
                Some(bins)
            } else {
                None
            };
            let mut setups = Vec::new();
            setups
                .try_reserve_exact(MAX_BINNED_TRIANGLES)
                .map_err(|_| "could not allocate triangle bin")?;
            let mut touched = Vec::new();
            let mut references = 0usize;
            for i in (0..count).step_by(3) {
                let setup_start = self.profile_shaders.then(Instant::now);
                let ix = |n: usize| indices.map_or(n, |ind| ind[n] as usize);
                let original = [
                    transformed[ix(i)],
                    transformed[ix(i + 1)],
                    transformed[ix(i + 2)],
                ];
                let poly = clip_triangle(original);
                self.record_primitive_setup(setup_start);
                self.stats.triangles += 1;
                if poly.as_slice() != original.as_slice() {
                    self.stats.clipped += 1;
                }
                let primitive = (self.stats.triangles - 1) as u32;
                for k in 1..poly.len().saturating_sub(1) {
                    let Some(setup) =
                        self.setup_triangle([poly[0], poly[k], poly[k + 1]], primitive, pipeline)
                    else {
                        continue;
                    };
                    let Some(bins) = bins.as_mut() else {
                        self.raster_triangle_setup(setup, pipeline, &fragment)?;
                        continue;
                    };
                    let min_tile_x = setup.min_x / TILE;
                    let min_tile_y = setup.min_y / TILE;
                    let end_tile_x = (setup.max_x - 1) / TILE + 1;
                    let end_tile_y = (setup.max_y - 1) / TILE + 1;
                    let tile_references = ((end_tile_x - min_tile_x) as usize)
                        .checked_mul((end_tile_y - min_tile_y) as usize)
                        .ok_or("triangle tile count overflow")?;
                    if tile_references > MAX_TILE_REFERENCES
                        || setups.len() == MAX_BINNED_TRIANGLES
                        || references
                            .checked_add(tile_references)
                            .is_none_or(|count| count > MAX_TILE_REFERENCES)
                    {
                        self.flush_tile_bins(
                            tiles_x,
                            &mut setups,
                            bins,
                            &mut touched,
                            pipeline,
                            &fragment,
                        )?;
                        references = 0;
                    }
                    if tile_references > MAX_TILE_REFERENCES {
                        self.raster_triangle_setup(setup, pipeline, &fragment)?;
                        continue;
                    }
                    let setup_index = setups.len();
                    setups.push(setup);
                    let bin_start = self.profile_shaders.then(Instant::now);
                    for tile_y in min_tile_y..end_tile_y {
                        for tile_x in min_tile_x..end_tile_x {
                            let tile = (tile_y * tiles_x + tile_x) as usize;
                            if bins[tile].is_empty() {
                                touched.push(tile);
                            }
                            bins[tile].push(setup_index);
                        }
                    }
                    if let Some(bin_start) = bin_start {
                        self.stats.primitive_setup_time += bin_start.elapsed();
                    }
                    references += tile_references;
                }
            }
            if let Some(bins) = bins.as_mut() {
                self.flush_tile_bins(
                    tiles_x,
                    &mut setups,
                    bins,
                    &mut touched,
                    pipeline,
                    &fragment,
                )?;
            }
            Ok(())
        })();
        self.framebuffer.resolve_samples();
        self.stats.raster_time += start.elapsed();
        render_result
    }
    fn setup_triangle(
        &mut self,
        v: [VertexOutput; 3],
        primitive: u32,
        state: Pipeline,
    ) -> Option<TriangleSetup> {
        let setup_start = self.profile_shaders.then(Instant::now);
        if v.iter().any(|v| v.position.w <= 0.) {
            self.record_primitive_setup(setup_start);
            return None;
        }
        let w = self.framebuffer.width;
        let h = self.framebuffer.height;
        let full_height = self.viewport_height;
        let min_w = v.iter().map(|v| v.position.w).fold(f32::INFINITY, f32::min);
        let mut s = std::array::from_fn::<_, 3, _>(|source| {
            let v = v[source];
            // A common scale cancels from perspective reconstruction and avoids
            // reciprocal overflow for tiny positive homogeneous coordinates.
            let iw = min_w / v.position.w;
            ScreenVertex {
                source,
                x: ((v.position.x / v.position.w * 0.5 + 0.5) * w as f32 * SUBPIXEL as f32).round()
                    as i64,
                y: ((0.5 - v.position.y / v.position.w * 0.5)
                    * full_height as f32
                    * SUBPIXEL as f32)
                    .round() as i64
                    - self.row_offset as i64 * SUBPIXEL,
                z: v.position.z / v.position.w,
                inv_w: iw,
                varyings: v.varyings,
            }
        });
        let mut area = edge(s[0], s[1], s[2].x, s[2].y);
        if area == 0 {
            self.record_primitive_setup(setup_start);
            return None;
        }
        let front = match state.front_face {
            FrontFace::Ccw => area < 0,
            FrontFace::Cw => area > 0,
        };
        if (state.cull == Cull::Back && !front) || (state.cull == Cull::Front && front) {
            self.stats.culled += 1;
            self.record_primitive_setup(setup_start);
            return None;
        }
        if area < 0 {
            s.swap(1, 2);
            area = -area;
        }
        let min_x = (s.iter().map(|p| p.x).min().unwrap() / SUBPIXEL).clamp(0, w as i64) as u32;
        let max_x = ((s.iter().map(|p| p.x).max().unwrap() + SUBPIXEL - 1) / SUBPIXEL)
            .clamp(0, w as i64) as u32;
        let min_y = (s.iter().map(|p| p.y).min().unwrap() / SUBPIXEL).clamp(0, h as i64) as u32;
        let max_y = ((s.iter().map(|p| p.y).max().unwrap() + SUBPIXEL - 1) / SUBPIXEL)
            .clamp(0, h as i64) as u32;
        if min_x >= max_x || min_y >= max_y {
            self.record_primitive_setup(setup_start);
            return None;
        }
        let edges = [(s[1], s[2]), (s[2], s[0]), (s[0], s[1])];
        let inclusive = edges.map(|(a, b)| top_left(a, b));
        let positions: &[(i64, i64)] = match self.framebuffer.sample_count() {
            SampleCount::One => &[(SUBPIXEL / 2, SUBPIXEL / 2)],
            SampleCount::Two => &SAMPLE_2X,
            SampleCount::Four => &SAMPLE_4X,
        };
        let offsets: [[i64; 3]; 4] = std::array::from_fn(|sample| {
            positions.get(sample).map_or([0; 3], |&(px, py)| {
                edges.map(|(a, b)| {
                    (b.x - a.x) * (py - SUBPIXEL / 2) - (b.y - a.y) * (px - SUBPIXEL / 2)
                })
            })
        });
        let inv_area = 1. / area as f32;
        let dx = edges.map(|(a, b)| -(b.y - a.y) as f32 * SUBPIXEL as f32 * inv_area);
        let dy = edges.map(|(a, b)| (b.x - a.x) as f32 * SUBPIXEL as f32 * inv_area);
        self.record_primitive_setup(setup_start);
        Some(TriangleSetup {
            primitive,
            vertices: s,
            min_x,
            min_y,
            max_x,
            max_y,
            inclusive,
            offsets,
            inv_area,
            dx,
            dy,
        })
    }
    fn raster_triangle_setup<F: Fn(&[Fragment; 4], u8) -> Result<[Option<ColorOutputs>; 4]>>(
        &mut self,
        setup: TriangleSetup,
        state: Pipeline,
        shader: &F,
    ) -> Result<()> {
        for tile_y in setup.min_y / TILE..=(setup.max_y - 1) / TILE {
            for tile_x in setup.min_x / TILE..=(setup.max_x - 1) / TILE {
                self.stats.tiles += 1;
                self.raster_tile(setup, tile_x, tile_y, state, shader)?;
            }
        }
        Ok(())
    }
    fn raster_tile<F: Fn(&[Fragment; 4], u8) -> Result<[Option<ColorOutputs>; 4]>>(
        &mut self,
        setup: TriangleSetup,
        tile_x: u32,
        tile_y: u32,
        state: Pipeline,
        shader: &F,
    ) -> Result<()> {
        let s = setup.vertices;
        let edges = [(s[1], s[2]), (s[2], s[0]), (s[0], s[1])];
        let min_x = (tile_x * TILE).max(setup.min_x);
        let min_y = (tile_y * TILE).max(setup.min_y);
        let end_x = ((tile_x + 1) * TILE).min(setup.max_x);
        let end_y = ((tile_y + 1) * TILE).min(setup.max_y);
        let step = edges.map(|(a, b)| -(b.y - a.y) * SUBPIXEL);
        let sample_count = self.framebuffer.sample_count().get();
        for y in min_y..end_y {
            let mut e = edges.map(|(a, b)| {
                edge(
                    a,
                    b,
                    min_x as i64 * SUBPIXEL + SUBPIXEL / 2,
                    y as i64 * SUBPIXEL + SUBPIXEL / 2,
                )
            });
            for x in (min_x..end_x).step_by(4) {
                let raster_start = self.profile_shaders.then(Instant::now);
                let mut coverage = [0u8; 4];
                for (sample, offset) in setup.offsets.iter().enumerate().take(sample_count) {
                    let sample_edges = std::array::from_fn(|i| e[i] + offset[i]);
                    let mask = simd::coverage4(sample_edges, step, setup.inclusive, self.backend);
                    for lane in 0..(end_x - x).min(4) {
                        if mask & (1 << lane) != 0 {
                            coverage[lane as usize] |= 1 << sample;
                        }
                    }
                }
                let mut prepared = [None; 4];
                let mut inputs = [Fragment::default(); 4];
                let mut active = 0u8;
                for lane in 0..(end_x - x).min(4) {
                    let coverage_samples = coverage[lane as usize];
                    if coverage_samples != 0 {
                        let sample = coverage_samples.trailing_zeros() as usize;
                        let sample_barycentrics: [[f32; 3]; 4] = std::array::from_fn(|sample| {
                            std::array::from_fn(|i| {
                                (e[i] + step[i] * lane as i64 + setup.offsets[sample][i]) as f32
                                    * setup.inv_area
                            })
                        });
                        let bary = sample_barycentrics[sample];
                        let sample_depths = sample_barycentrics.map(|bary| {
                            (0..3).map(|i| bary[i] * s[i].z).sum::<f32>().clamp(0., 1.)
                        });
                        let p = self.prepare_fragment(
                            x + lane,
                            y,
                            setup.primitive,
                            bary,
                            setup.dx,
                            setup.dy,
                            s,
                            state,
                            coverage_samples,
                            sample_depths,
                        )?;
                        if let Some(p) = p {
                            inputs[lane as usize] = p.input;
                            if p.passing_samples != 0 || p.debug {
                                active |= 1 << lane;
                            }
                            prepared[lane as usize] = Some(p);
                        }
                    }
                }
                if let Some(start) = raster_start {
                    self.stats.rasterization_time += start.elapsed();
                }
                self.stats.shaded += active.count_ones() as u64;
                let start = (self.profile_shaders && active != 0).then(Instant::now);
                let outputs = if active == 0 {
                    [None; 4]
                } else {
                    shader(&inputs, active)?
                };
                if let Some(start) = start {
                    self.stats.shader_time += start.elapsed();
                }
                for lane in 0..4 {
                    if let Some(p) = prepared[lane] {
                        let output = if active & (1 << lane) != 0 {
                            outputs[lane]
                        } else {
                            None
                        };
                        self.finish_fragment(p, output, state)?;
                    }
                }
                for i in 0..3 {
                    e[i] += step[i] * 4;
                }
            }
        }
        Ok(())
    }
    fn flush_tile_bins<F: Fn(&[Fragment; 4], u8) -> Result<[Option<ColorOutputs>; 4]>>(
        &mut self,
        tiles_x: u32,
        setups: &mut Vec<TriangleSetup>,
        bins: &mut [Vec<usize>],
        touched: &mut Vec<usize>,
        state: Pipeline,
        shader: &F,
    ) -> Result<()> {
        for &tile in touched.iter() {
            let tile_x = tile as u32 % tiles_x;
            let tile_y = tile as u32 / tiles_x;
            self.stats.tiles += bins[tile].len() as u64;
            for &setup in &bins[tile] {
                self.raster_tile(setups[setup], tile_x, tile_y, state, shader)?;
            }
            bins[tile].clear();
        }
        touched.clear();
        setups.clear();
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn prepare_fragment(
        &mut self,
        x: u32,
        y: u32,
        primitive: u32,
        bary: [f32; 3],
        dx: [f32; 3],
        dy: [f32; 3],
        s: [ScreenVertex; 3],
        state: Pipeline,
        covered_samples: u8,
        sample_depths: [f32; 4],
    ) -> Result<Option<PreparedFragment>> {
        self.stats.fragments += 1;
        let index = (y * self.framebuffer.width + x) as usize;
        let z = sample_depths[covered_samples.trailing_zeros() as usize];
        let mut previous_depth = None;
        let mut stencil_samples = 0u8;
        let mut depth_samples = 0u8;
        let mut passing_samples = 0u8;
        for (sample, &sample_depth) in sample_depths
            .iter()
            .enumerate()
            .take(self.framebuffer.sample_count().get())
        {
            let bit = 1 << sample;
            if covered_samples & bit == 0 {
                continue;
            }
            let old_depth = self.framebuffer.sample_depth(index, sample);
            previous_depth.get_or_insert(old_depth);
            let stencil_pass = state.stencil.is_none_or(|st| {
                st.compare.test(
                    st.reference & st.read_mask,
                    self.framebuffer.sample_stencil(index, sample) & st.read_mask,
                )
            });
            let depth_pass = state.depth_compare.test(sample_depth, old_depth);
            if stencil_pass {
                stencil_samples |= bit;
            }
            if depth_pass {
                depth_samples |= bit;
            }
            if !stencil_pass {
                self.stats.stencil_rejected += 1;
                self.stencil_op(index, sample, state.stencil.map(|s| (s, s.fail)));
            } else if !depth_pass {
                self.stats.early_z_rejected += 1;
                self.stencil_op(index, sample, state.stencil.map(|s| (s, s.depth_fail)));
            } else {
                passing_samples |= bit;
            }
        }
        let old_depth = previous_depth.unwrap_or(1.);
        let stencil_pass = stencil_samples == covered_samples;
        let depth_pass = depth_samples == covered_samples;
        let debug = self.debug_pixel == Some((x, y + self.row_offset));
        if !debug && passing_samples == 0 {
            return Ok(None);
        }
        let interpolate = |b: [f32; 3]| {
            let weights = std::array::from_fn::<_, 3, _>(|i| b[i] * s[i].inv_w);
            let denom = weights.iter().sum::<f32>();
            std::array::from_fn::<_, 4, _>(|slot| {
                (s[0].varyings[slot] * weights[0]
                    + s[1].varyings[slot] * weights[1]
                    + s[2].varyings[slot] * weights[2])
                    / denom
            })
        };
        let interpolate_uv = |b: [f32; 3]| {
            let weights = std::array::from_fn::<_, 3, _>(|i| b[i] * s[i].inv_w);
            let denom = weights.iter().sum::<f32>();
            (s[0].varyings[1] * weights[0]
                + s[1].varyings[1] * weights[1]
                + s[2].varyings[1] * weights[2])
                / denom
        };
        let varyings = interpolate(bary);
        let uv_x = interpolate_uv(std::array::from_fn(|i| bary[i] + dx[i]));
        let uv_y = interpolate_uv(std::array::from_fn(|i| bary[i] + dy[i]));
        if !varyings.iter().all(|v| v.is_finite()) || !uv_x.is_finite() || !uv_y.is_finite() {
            return Err("perspective interpolation exceeded finite f32 range".into());
        }
        let input = Fragment {
            x,
            y: y + self.row_offset,
            primitive,
            depth: z,
            barycentric: {
                let mut original = [0.; 3];
                for i in 0..3 {
                    original[s[i].source] = bary[i];
                }
                Vec3::new(original[0], original[1], original[2])
            },
            varyings,
            uv_dx: Vec2::new(uv_x.x - varyings[1].x, uv_x.y - varyings[1].y),
            uv_dy: Vec2::new(uv_y.x - varyings[1].x, uv_y.y - varyings[1].y),
            direction_dx: uv_x.xyz() - varyings[1].xyz(),
            direction_dy: uv_y.xyz() - varyings[1].xyz(),
        };
        Ok(Some(PreparedFragment {
            input,
            index,
            previous_depth: old_depth,
            stencil_pass,
            depth_pass,
            passing_samples,
            sample_depths,
            debug,
        }))
    }
    fn finish_fragment(
        &mut self,
        prepared: PreparedFragment,
        output: Option<ColorOutputs>,
        state: Pipeline,
    ) -> Result<()> {
        let PreparedFragment {
            input,
            index,
            previous_depth: old_depth,
            stencil_pass,
            depth_pass,
            passing_samples,
            sample_depths,
            debug,
        } = prepared;
        if passing_samples != 0 && output.is_none() {
            self.stats.discarded += 1;
        }
        if debug {
            self.traces.push(PixelTrace {
                fragment: input,
                output: output.and_then(|outputs| outputs[0]),
                previous_depth: old_depth,
                depth_pass,
                stencil_pass,
            });
        }
        let blend_start = self.profile_shaders.then(Instant::now);
        if let Some(colors) = output {
            if colors.iter().flatten().any(|color| !color.0.is_finite()) {
                return Err("fragment shader produced a non-finite color".into());
            }
            for (sample, &sample_depth) in sample_depths
                .iter()
                .enumerate()
                .take(self.framebuffer.sample_count().get())
            {
                if passing_samples & (1 << sample) == 0 {
                    continue;
                }
                self.stencil_op(index, sample, state.stencil.map(|s| (s, s.pass)));
                if state.depth_write {
                    self.framebuffer
                        .write_sample_depth(index, sample, sample_depth);
                }
                for (attachment, source) in colors
                    .iter()
                    .take(self.framebuffer.color_attachment_count())
                    .enumerate()
                {
                    let Some(source) = source else { continue };
                    let color = if state.blend == Blend::Replace {
                        *source
                    } else {
                        state.blend.apply(
                            *source,
                            self.framebuffer
                                .read_sample_color(attachment, index, sample),
                        )
                    };
                    if state.color_write {
                        self.framebuffer
                            .write_sample_color(attachment, index, sample, color);
                    }
                }
            }
        }
        if let Some(start) = blend_start {
            self.stats.blend_write_time += start.elapsed();
        }
        Ok(())
    }
    fn stencil_op(
        &mut self,
        index: usize,
        sample: usize,
        state: Option<(StencilState, StencilOp)>,
    ) {
        if let Some((s, op)) = state {
            let old = self.framebuffer.sample_stencil(index, sample);
            self.framebuffer.write_sample_stencil(
                index,
                sample,
                (old & !s.write_mask) | (op.apply(old, s.reference) & s.write_mask),
            );
        }
    }
}
/// Bresenham lines after Liang–Barsky viewport clipping. Endpoints may be offscreen.
pub fn draw_line(fb: &mut Framebuffer, a: Vec2, b: Vec2, color: Color) -> Result<()> {
    if !a.is_finite() || !b.is_finite() {
        return Err("line endpoints must be finite".into());
    }
    let d = b - a;
    let mut lo: f32 = 0.;
    let mut hi: f32 = 1.;
    for (p, q) in [
        (-d.x, a.x),
        (d.x, fb.width as f32 - 1. - a.x),
        (-d.y, a.y),
        (d.y, fb.height as f32 - 1. - a.y),
    ] {
        if p == 0. {
            if q < 0. {
                return Ok(());
            }
        } else {
            let t = q / p;
            if p < 0. {
                lo = lo.max(t);
            } else {
                hi = hi.min(t);
            }
        }
    }
    if lo > hi {
        return Ok(());
    }
    let start = a + d * lo;
    let end = a + d * hi;
    let (mut x, mut y) = (start.x.round() as i32, start.y.round() as i32);
    let (ex, ey) = (end.x.round() as i32, end.y.round() as i32);
    let dx = (ex - x).abs();
    let dy = -(ey - y).abs();
    let sx = if x < ex { 1 } else { -1 };
    let sy = if y < ey { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        fb.set_pixel(x as u32, y as u32, color)?;
        if x == ex && y == ey {
            break;
        }
        let e = err * 2;
        if e >= dy {
            err += dy;
            x += sx;
        }
        if e <= dx {
            err += dx;
            y += sy;
        }
    }
    Ok(())
}
impl Renderer {
    /// Raster workers own disjoint horizontal bands and retain submission order.
    /// ponytail: geometry setup is repeated per band; bin prepared triangles when
    /// profiling shows setup dominates. No framebuffer lock or unsafe sharing.
    pub fn render_bands<F>(&mut self, threads: usize, render: F) -> Result<()>
    where
        F: Fn(&mut Renderer) -> Result<()> + Sync,
    {
        if !(1..=64).contains(&threads) {
            return Err("raster workers must be 1..64".into());
        }
        if threads == 1 {
            return render(self);
        }
        let workers = threads.min(self.framebuffer.height as usize);
        let rows = self.framebuffer.height.div_ceil(workers as u32);
        let mut bands = Vec::new();
        for y in (0..self.framebuffer.height).step_by(rows as usize) {
            let height = rows.min(self.framebuffer.height - y);
            let fb = self.framebuffer.band(y, height)?;
            bands.push(Renderer {
                framebuffer: fb,
                backend: self.backend,
                profile_shaders: self.profile_shaders,
                stats: Statistics::default(),
                debug_pixel: self.debug_pixel,
                traces: Vec::new(),
                row_offset: y,
                viewport_height: self.framebuffer.height,
            });
        }
        let outputs = std::thread::scope(|scope| {
            let render = &render;
            let tasks: Vec<_> = bands
                .into_iter()
                .map(|mut band| {
                    scope.spawn(move || {
                        render(&mut band)?;
                        Ok(band)
                    })
                })
                .collect();
            tasks
                .into_iter()
                .map(|t| {
                    t.join()
                        .map_err(|_| "raster worker panicked".into())
                        .and_then(|r: Result<Renderer>| r)
                })
                .collect::<Result<Vec<_>>>()
        })?;
        if let Some(first) = outputs.first() {
            self.framebuffer
                .set_color_attachment_count(first.framebuffer.color_attachment_count())?;
        }
        self.stats = Statistics::default();
        self.traces.clear();
        for (i, band) in outputs.into_iter().enumerate() {
            self.framebuffer
                .copy_band_from(band.row_offset, &band.framebuffer)?;
            if i == 0 {
                self.stats.vertices = band.stats.vertices;
                self.stats.triangles = band.stats.triangles;
                self.stats.clipped = band.stats.clipped;
                self.stats.culled = band.stats.culled;
            }
            self.stats.tiles += band.stats.tiles;
            self.stats.command_processing_time += band.stats.command_processing_time;
            self.stats.primitive_setup_time += band.stats.primitive_setup_time;
            self.stats.rasterization_time += band.stats.rasterization_time;
            self.stats.blend_write_time += band.stats.blend_write_time;
            self.stats.fragments += band.stats.fragments;
            self.stats.shaded += band.stats.shaded;
            self.stats.early_z_rejected += band.stats.early_z_rejected;
            self.stats.stencil_rejected += band.stats.stencil_rejected;
            self.stats.shader_time += band.stats.shader_time;
            self.stats.shader_packets += band.stats.shader_packets;
            self.stats.shader_packet_lanes += band.stats.shader_packet_lanes;
            self.stats.shader_instructions += band.stats.shader_instructions;
            self.stats.texture_samples += band.stats.texture_samples;
            self.stats.discarded += band.stats.discarded;
            self.stats.vertex_time += band.stats.vertex_time;
            self.stats.raster_time += band.stats.raster_time;
            self.traces.extend(band.traces);
        }
        Ok(())
    }
    pub fn surface_size(&self) -> (u32, u32) {
        (self.framebuffer.width, self.viewport_height)
    }
}
