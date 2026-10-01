# Graphics pipeline

World coordinates are right-handed with +Y up. Matrices are row-major and act on
column vectors. The clip volume is `-w <= x,y <= w` and `0 <= z <= w`. Six-plane
Sutherland–Hodgman clipping linearly interpolates positions and varyings in
homogeneous space, then a triangle fan assembles the resulting polygon. The
viewport maps NDC +Y to screen top; depth remains 0..1. Backface culling respects
CW/CCW before the internal coverage winding normalization.

Screen X/Y are rounded to 8 fractional bits. Three signed i64 edge functions
advance across pixels. Coverage is evaluated at pixel centers `(x+.5,y+.5)` with
the top-left tie rule. Two triangles sharing an edge cover it exactly once;
alpha blending in the shared-edge test detects double coverage and cracks.
Clipped triangles are binned into screen-aligned 16×16 tiles in bounded batches.
Each tile retains triangle submission order, and batches flush in order so depth,
stencil and blending remain deterministic. Very large tile grids use streaming
triangle traversal instead of allocating a bin for every tile. This provides
tile-local raster work but not hierarchical Z or a persistent worker pool.

For barycentrics `b_i`, each varying is reconstructed as
`sum(b_i * attribute_i / w_i) / sum(b_i / w_i)`. Clip/NDC depth is interpolated
*affinely* in screen space. Texture derivatives use the perspective reconstruction
at the neighboring X/Y pixel center as an approximation; LOD is the logarithm
of the larger texel-space derivative length. Unlike hardware derivative quads,
these neighbors do not depend on other shader invocations.

Depth and stencil comparisons precede fragment shading. This is valid because
shader interfaces cannot write depth or produce side effects. A discarded Rust or SIR
fragment does not write depth or stencil-pass results. Failed stencil/depth
comparisons execute their respective stencil operations. All eight depth compare
modes, stencil masks, saturation/invert operations and replace/alpha/add/multiply
color blending are implemented. Alpha blending uses unpremultiplied source RGB.
Transparency requires caller-provided draw ordering and disabled depth writes.
The `stencil` scene uses a circular mask to limit both a textured cube and an
alpha-blended triangle to a portal. The overlay keeps depth writes disabled and
also tests the mask; its fragments outside the portal leave the clear color
untouched. `tests/stencil.rs` checks that boundary and exact framebuffer, depth,
and stencil results between scalar and four-band SIMD rendering.

SPIR-V fragment outputs at locations 0–3 write to matching RGBA8 color targets.
`CommandBuffer::begin_render_pass_with_colors` takes one clear color per target;
depth, stencil, sample count, blend mode and color-write enable are shared across
the pass. `Framebuffer::color_attachment_pixel` and
`Framebuffer::color_attachment_bytes` read each result; the existing `pixel`,
`bytes` and PNG methods still address target 0. The renderer caps a pass at four
targets and 512 MiB for color, depth and stencil storage including MSAA samples.
Multi-target command captures use version 3; version 1 and 2 captures retain
their existing format.

Textures own RGBA-expanded texels from RGBA8, RGB8 or R8 input. Nearest and
bilinear filters support clamp, repeat and mirror addressing. Bilinear samples
texel centers and wraps each neighbor, including at seams. Mips average source
regions and include all texels for odd dimensions; dimensions use floor halving.
Nearest-mip and trilinear filtering are available. `Texture::sample_anisotropic`
uses both UV derivatives, samples along the larger texel-space direction, and
chooses its mip level from the smaller footprint. Its explicit tap cap is 1×–16×;
`anisotropy_showcase` compares 16× filtering against ordinary trilinear sampling
on a steeply viewed stripe plane. Filtering is in stored numeric color space;
sRGB decoding remains future work. Anisotropy is
currently exposed to native Rust fragment shaders; the SIR/SPIR-V sampling path
continues to use isotropic LOD.

`TextureArray` layers must have matching format, dimensions and mip chains; its
layer coordinate is a finite non-negative integer. `Texture3D` stores bounded
RGBA-expanded color voxels and generates mips by averaging 3D source regions.
Both support nearest/bilinear spatial filtering, no/nearest/trilinear mip
selection, and clamp/repeat/mirror addressing. Volume bilinear filtering
interpolates the eight neighboring voxels. SIR offers explicit-LOD and
derivative-LOD sampling for both types. Arrays are capped at 2048 layers and
16M total mip texels; a volume is capped at 16M base texels and 32M total mip
texels. Depth textures remain single-level 2D resources.

`CubeMap` owns six square color textures in +X, -X, +Y, -Y, +Z, -Z order. A
direction selects the face with the largest absolute component; the other two
components map to face UVs. Nearest samples stay on that face; bilinear taps that
cross an edge are projected onto the adjacent face. Faces must have matching mip
dimensions. `cubemap_showcase` uses this
sampler for an environment skybox and reflected directions on the native Rust
shader scene. Material roughness selects a box-filtered mip level, which blurs
reflections without implementing split-sum image-based lighting. Ordinary GLSL
SPIR-V can also bind `samplerCube` and sample it with explicit `textureLod`; the
PBR scene uses this route. `texture()` also supports an unmodified vec3 direction
at fragment input location 1. Neighbor-center derivatives project through the
center direction's selected face for isotropic mip selection; this remains a
software approximation, and transformed directions remain unsupported.

`Renderer::set_sample_count` selects 1×, 2× or 4× rendering. The 2×/4× modes use
fixed deterministic subpixel positions and track color, depth and stencil for
each sample. Fragment varyings are shaded once per primitive/pixel using the
first covered sample; depth tests and attachment writes remain per sample.
Resolve averages the stored color bytes, reports the
nearest sample depth, and exposes sample zero's stencil value through the
single-sample framebuffer. The combined color/depth/stencil storage is capped at
512 MiB.
The CLI accepts `--samples 2` or `--samples 4`. Frame captures preserve the
sample count and replay the same per-sample color, depth and stencil state.

The scalar reference and optional NEON/AVX2 coverage paths both process four
adjacent pixel masks with identical i64 arithmetic. With the SIMD backend,
recorded SIR fragment shaders execute surviving lanes as a masked group of four;
vertex shaders and native closures retain scalar execution. See [SIMD masks](simd.md).
Parallel rendering assigns disjoint horizontal bands to Rust scoped threads,
each using local tile bins. Draw order is preserved within every band, including
depth, stencil and blending. `render_bands_shared_vertices` is an opt-in for
identical per-band draw streams: it validates geometry and pipeline state and
runs each draw's vertex shader once, then shares the immutable outputs. Primitive
setup and tile bins remain local to each band; persistent workers remain future
optimization work. The CLI and offline animation use the shared-vertex path.

See [Khronos rasterization conventions](https://docs.vulkan.org/spec/latest/chapters/primsrast.html)
for background on pixel coverage and interpolation. These conventions do not
constitute Vulkan compatibility.
