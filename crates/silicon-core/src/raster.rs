use crate::*;
use std::time::{Duration, Instant};
const SUBPIXEL: i64 = 256;
const TILE: u32 = 16;
#[derive(Clone, Debug, Default)]
pub struct Statistics {
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
#[derive(Clone, Debug)]
pub struct PixelTrace {
    pub fragment: Fragment,
    pub output: Option<Color>,
    pub previous_depth: f32,
    pub depth_pass: bool,
    pub stencil_pass: bool,
}
pub struct Renderer {
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
fn edge(a: ScreenVertex, b: ScreenVertex, x: i64, y: i64) -> i64 {
    (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x)
}
fn top_left(a: ScreenVertex, b: ScreenVertex) -> bool {
    b.y < a.y || (b.y == a.y && b.x > a.x)
}
impl Renderer {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        Ok(Self {
            backend: Backend::Scalar,
            row_offset: 0,
            viewport_height: height,
            framebuffer: Framebuffer::new(width, height)?,
            stats: Statistics::default(),
            debug_pixel: None,
            traces: Vec::new(),
        })
    }
    pub fn clear(&mut self, color: Color) {
        self.framebuffer.clear(color);
        self.framebuffer.clear_depth(1.);
        self.framebuffer.clear_stencil(0);
        self.stats = Statistics::default();
        self.traces.clear();
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
        for i in (0..count).step_by(3) {
            let ix = |n: usize| indices.map_or(n, |ind| ind[n] as usize);
            let original = [
                transformed[ix(i)],
                transformed[ix(i + 1)],
                transformed[ix(i + 2)],
            ];
            let poly = clip_triangle(original);
            self.stats.triangles += 1;
            if poly.as_slice() != original.as_slice() {
                self.stats.clipped += 1;
            }
            let primitive = (self.stats.triangles - 1) as u32;
            for k in 1..poly.len().saturating_sub(1) {
                self.triangle(
                    [poly[0], poly[k], poly[k + 1]],
                    primitive,
                    pipeline,
                    &fragment,
                )?;
            }
        }
        self.stats.raster_time += start.elapsed();
        Ok(())
    }
    fn triangle<F: Fn(&Fragment) -> Result<Option<Color>>>(
        &mut self,
        v: [VertexOutput; 3],
        primitive: u32,
        state: Pipeline,
        shader: &F,
    ) -> Result<()> {
        if v.iter().any(|v| v.position.w <= 1e-8) {
            return Ok(());
        }
        let w = self.framebuffer.width;
        let h = self.framebuffer.height;
        let full_height = self.viewport_height;
        let mut s = std::array::from_fn::<_, 3, _>(|source| {
            let v = v[source];
            let iw = 1. / v.position.w;
            ScreenVertex {
                source,
                x: ((v.position.x * iw * 0.5 + 0.5) * w as f32 * SUBPIXEL as f32).round() as i64,
                y: ((0.5 - v.position.y * iw * 0.5) * full_height as f32 * SUBPIXEL as f32).round()
                    as i64
                    - self.row_offset as i64 * SUBPIXEL,
                z: v.position.z * iw,
                inv_w: iw,
                varyings: v.varyings,
            }
        });
        let mut area = edge(s[0], s[1], s[2].x, s[2].y);
        if area == 0 {
            return Ok(());
        }
        let front = match state.front_face {
            FrontFace::Ccw => area < 0,
            FrontFace::Cw => area > 0,
        };
        if (state.cull == Cull::Back && !front) || (state.cull == Cull::Front && front) {
            self.stats.culled += 1;
            return Ok(());
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
        let edges = [(s[1], s[2]), (s[2], s[0]), (s[0], s[1])];
        let inclusive = edges.map(|(a, b)| top_left(a, b));
        let inv_area = 1. / area as f32;
        let dx = edges.map(|(a, b)| -(b.y - a.y) as f32 * SUBPIXEL as f32 * inv_area);
        let dy = edges.map(|(a, b)| (b.x - a.x) as f32 * SUBPIXEL as f32 * inv_area);
        for ty in (min_y..max_y).step_by(TILE as usize) {
            for tx in (min_x..max_x).step_by(TILE as usize) {
                self.stats.tiles += 1;
                let end_x = (tx + TILE).min(max_x);
                let end_y = (ty + TILE).min(max_y);
                for y in ty..end_y {
                    let mut e = edges.map(|(a, b)| {
                        edge(
                            a,
                            b,
                            tx as i64 * SUBPIXEL + SUBPIXEL / 2,
                            y as i64 * SUBPIXEL + SUBPIXEL / 2,
                        )
                    });
                    let step = edges.map(|(a, b)| -(b.y - a.y) * SUBPIXEL);
                    for x in (tx..end_x).step_by(4) {
                        let mask = simd::coverage4(e, step, inclusive, self.backend);
                        for lane in 0..(end_x - x).min(4) {
                            if mask & (1 << lane) != 0 {
                                let bary = std::array::from_fn(|i| {
                                    (e[i] + step[i] * lane as i64) as f32 * inv_area
                                });
                                self.fragment(
                                    x + lane,
                                    y,
                                    primitive,
                                    bary,
                                    dx,
                                    dy,
                                    s,
                                    state,
                                    shader,
                                )?;
                            }
                        }
                        for i in 0..3 {
                            e[i] += step[i] * 4;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn fragment<F: Fn(&Fragment) -> Result<Option<Color>>>(
        &mut self,
        x: u32,
        y: u32,
        primitive: u32,
        bary: [f32; 3],
        dx: [f32; 3],
        dy: [f32; 3],
        s: [ScreenVertex; 3],
        state: Pipeline,
        shader: &F,
    ) -> Result<()> {
        self.stats.fragments += 1;
        let index = (y * self.framebuffer.width + x) as usize;
        let z = (0..3).map(|i| bary[i] * s[i].z).sum::<f32>().clamp(0., 1.);
        let old_depth = self.framebuffer.depth[index];
        let stencil_pass = state.stencil.is_none_or(|st| {
            st.compare.test(
                st.reference & st.read_mask,
                self.framebuffer.stencil[index] & st.read_mask,
            )
        });
        let depth_pass = state.depth_compare.test(z, old_depth);
        let debug = self.debug_pixel == Some((x, y + self.row_offset));
        if !stencil_pass {
            self.stats.stencil_rejected += 1;
            self.stencil_op(index, state.stencil.map(|s| (s, s.fail)));
        } else if !depth_pass {
            self.stats.early_z_rejected += 1;
            self.stencil_op(index, state.stencil.map(|s| (s, s.depth_fail)));
        }
        if !debug && (!stencil_pass || !depth_pass) {
            return Ok(());
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
        let varyings = interpolate(bary);
        let vx = interpolate(std::array::from_fn(|i| bary[i] + dx[i]));
        let vy = interpolate(std::array::from_fn(|i| bary[i] + dy[i]));
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
            uv_dx: Vec2::new(vx[1].x - varyings[1].x, vx[1].y - varyings[1].y),
            uv_dy: Vec2::new(vy[1].x - varyings[1].x, vy[1].y - varyings[1].y),
        };
        let output = if stencil_pass && depth_pass {
            self.stats.shaded += 1;
            shader(&input)?
        } else {
            None
        };
        if debug {
            self.traces.push(PixelTrace {
                fragment: input,
                output,
                previous_depth: old_depth,
                depth_pass,
                stencil_pass,
            });
        }
        if let Some(color) = output {
            if !color.0.is_finite() {
                return Err("fragment shader produced a non-finite color".into());
            }
            self.stencil_op(index, state.stencil.map(|s| (s, s.pass)));
            if state.depth_write {
                self.framebuffer.depth[index] = z;
            }
            let color = if state.blend == Blend::Replace {
                color
            } else {
                state.blend.apply(color, self.framebuffer.read(index))
            };
            if state.color_write {
                self.framebuffer.write(index, color);
            }
        }
        Ok(())
    }
    fn stencil_op(&mut self, index: usize, state: Option<(StencilState, StencilOp)>) {
        if let Some((s, op)) = state {
            let old = self.framebuffer.stencil[index];
            self.framebuffer.stencil[index] =
                (old & !s.write_mask) | (op.apply(old, s.reference) & s.write_mask);
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
        let width = self.framebuffer.width;
        let mut bands = Vec::new();
        for y in (0..self.framebuffer.height).step_by(rows as usize) {
            let height = rows.min(self.framebuffer.height - y);
            let start = (y * width) as usize;
            let end = ((y + height) * width) as usize;
            let fb = Framebuffer {
                width,
                height,
                stride: self.framebuffer.stride,
                format: self.framebuffer.format,
                pixels: self.framebuffer.pixels[start * 4..end * 4].to_vec(),
                depth: self.framebuffer.depth[start..end].to_vec(),
                stencil: self.framebuffer.stencil[start..end].to_vec(),
            };
            bands.push(Renderer {
                framebuffer: fb,
                backend: self.backend,
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
        self.stats = Statistics::default();
        self.traces.clear();
        for (i, band) in outputs.into_iter().enumerate() {
            let start = (band.row_offset * width) as usize;
            let end = start + (band.framebuffer.height * width) as usize;
            self.framebuffer.pixels[start * 4..end * 4].copy_from_slice(&band.framebuffer.pixels);
            self.framebuffer.depth[start..end].copy_from_slice(&band.framebuffer.depth);
            self.framebuffer.stencil[start..end].copy_from_slice(&band.framebuffer.stencil);
            if i == 0 {
                self.stats.vertices = band.stats.vertices;
                self.stats.triangles = band.stats.triangles;
                self.stats.clipped = band.stats.clipped;
                self.stats.culled = band.stats.culled;
            }
            self.stats.tiles += band.stats.tiles;
            self.stats.fragments += band.stats.fragments;
            self.stats.shaded += band.stats.shaded;
            self.stats.early_z_rejected += band.stats.early_z_rejected;
            self.stats.stencil_rejected += band.stats.stencil_rejected;
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
