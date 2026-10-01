# SILICON

**A GPU built entirely in software.**

[![CI](https://github.com/OthmaneBlial/silicon/actions/workflows/ci.yml/badge.svg)](https://github.com/OthmaneBlial/silicon/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/OthmaneBlial/silicon)](https://github.com/OthmaneBlial/silicon/releases/latest)

[Project site](https://othmaneblial.github.io/silicon/) ·
[Online docs](https://othmaneblial.github.io/silicon/docs.html)

SILICON is an experimental programmable graphics processor implemented in Rust.
Vertex processing, clipping, rasterization, shader execution, textures, depth,
stencil and blending generate scene pixels entirely on the CPU.

![SILICON's CPU-rendered sculpture scene](assets/screenshots/spirv_showcase.png)

*Rendered entirely on the CPU by a GPU I wrote from scratch.*

![Metallic materials with tangent-space normal mapping rendered by SILICON's GLSL SPIR-V pipeline](assets/screenshots/pbr_showcase.png)

*Cook-Torrance GGX shading. The shader, rasterizer and framebuffer all run on the CPU.*

![A CPU-rendered sky environment reflected by the SILICON sculpture and metal rings](assets/screenshots/cubemap_showcase.png)

*Six-face cube-map skybox and roughness-selected environment reflections in the native Rust reference scene.*

![The cube-map reflection scene rendered with four samples per pixel](assets/screenshots/cubemap_showcase_msaa4.png)

*The same CPU scene with 4× multisample coverage and a resolved color buffer.*

![Side-by-side trilinear and 16x anisotropic filtering on a steeply viewed stripe plane](assets/screenshots/anisotropy_showcase.png)

[Watch the CPU-rendered animation](assets/demos/spirv_showcase.mp4) ·
[Architecture](docs/architecture.md) · [Pipeline](docs/graphics-pipeline.md) ·
[Shader VM](docs/sir.md) · [SIMD](docs/simd.md) · [SPIR-V subset](docs/spirv.md) · [Vulkan-like Rust subset](docs/vulkan-like.md) · [Third-party demo](docs/third-party-demo.md) · [Roadmap](docs/roadmap.md)

## Run it

[Download the macOS ARM64 CLI](https://github.com/OthmaneBlial/silicon/releases/latest)
with checksums, or build from source:

```sh
git clone https://github.com/OthmaneBlial/silicon
cd silicon
cargo run --release -p silicon-cli -- run spirv_showcase --threads 4
```

Escape exits, Space pauses, arrows adjust rotation. The window presents an
already rendered CPU framebuffer. On macOS `minifb` uses Metal for that final
pixel presentation; no scene geometry or SILICON shading runs on the host GPU.
Headless rendering requires no graphics device or display.

```sh
cargo run --release -p silicon-cli -- render assets/scenes/showcase.json --output output/scene.png
cargo run --release -p silicon-cli -- render assets/scenes/pbr_showcase.json --backend simd --threads 4 --output output/pbr.png
cargo run --release -p silicon-cli -- render assets/scenes/cubemap_showcase.json --backend simd --threads 4 --output output/cubemap.png
cargo run --release -p silicon-cli -- render cubemap_showcase --samples 4 --backend simd --threads 4 --output output/cubemap-msaa4.png
cargo run --release -p silicon-cli -- render anisotropy_showcase --output output/anisotropy.png
cargo run --release -p silicon-cli -- render showcase --width 640 --height 400 --threads 4
cargo run --release -p silicon-cli -- render stencil --backend simd --threads 4 --output output/stencil.png
cargo run --release --example triangle
cargo run --release --example textured_cube
cargo run --release --example stencil
cargo run --release --example rust_api
cargo run --release --example vulkan_like
cargo run --release --example khronos_hello_triangle
```

Requires Rust 1.95.0 (pinned). macOS ARM64 was run locally, including the window.
Linux x86-64 and ARM64 are CI targets. Window mode on Linux uses X11; the
headless renderer works without X11 at runtime. Other platforms are unverified.

## Use the Rust API

The supported Rust integration surface is [`silicon::api`](docs/rust-api.md).
`Device` creates bounded SPIR-V shader modules, links them into an immutable
pipeline, creates owned typed buffers and command buffers, and submits commands
into an explicit `Renderer`. The renderer owns the output size and framebuffer.
`examples/rust_api.rs` is a runnable end-to-end sample:

```sh
cargo run --release --example rust_api
```

It writes `output/rust_api.png`. Shader modules currently accept SILICON's
documented SPIR-V 1.0 graphics subset; this is not a general-purpose shader
compiler or Vulkan API.

The versioned C ABI is documented in [docs/c-api.md](docs/c-api.md), with its
public header in `crates/silicon-c-api/include/silicon.h`. Its standalone C
smoke test renders and reads back a triangle:

```sh
sh crates/silicon-c-api/scripts/test.sh
```

Phase 68 adds a separate Rust-only [Vulkan-like subset](docs/vulkan-like.md)
with instance/device setup, typed resources, descriptor-like bindings, an
offscreen render pass and synchronous indexed drawing. `examples/vulkan_like.rs`
writes `output/vulkan_like_triangle.png`. It is not a Vulkan loader, ABI, or
conformant implementation.

Phase 69 adapts KhronosGroup's Apache-2.0 Vulkan `hello_triangle` through that
subset. The original positions and RGB colors, adapted vertex shader, and
unchanged upstream fragment shader run through SILICON. See the
[source, license, and adaptation notes](docs/third-party-demo.md).

Phase 70 now reads Freedoom E1M1, palette-decodes its real floor and ceiling
flats, composes its opaque wall textures from WAD patches, and submits 4,812
triangles across 141 SILICON draws through the textured SPIR-V pipeline. This
static view is not playable Doom. The
[WAD source, run command, screenshot, license, and remaining limits](docs/freedoom.md)
are documented. The Freedoom WAD stays external.

![Freedoom E1M1 geometry rendered from its player start by SILICON](assets/screenshots/freedoom_e1m1.png)

*Static E1M1 geometry rendered by SILICON; WAD flats and opaque walls use their palette and sector lighting.*

## Programmable, observable, reproducible

The `spirv_showcase` executes ordinary GLSL vertex/fragment shaders through
SPIR-V → SIR → CPU commands: a real original OBJ sculpture,
filtered textures, smooth normals, a normal matrix, Lambert/Blinn-Phong lighting,
a directional light, a point light and fog. Its 30 draws submit 12,588 triangles.
The native `showcase` remains an independent Rust shader reference. The `shader_cube` executes **both stages**
through SILICON's validated SIR bytecode interpreter and owned GPU-like commands.
The `spirv_cube` loads ordinary externally compiled GLSL vertex/fragment shaders,
validates a strict SPIR-V 1.0 subset and translates it into the same CPU VM.
The `spirv_cutout` executes nested GLSL branches, conditional texture sampling,
Phi/local reconvergence, early return and fragment discard.
The `shadow_showcase` renders a CPU depth pass for the same OBJ scene and samples
its depth texture from an ordinary GLSL fragment shader through explicit-LOD SPIR-V.
The `pbr_showcase` runs a GLSL metallic/roughness Cook-Torrance GGX shader through
SPIR-V and the same CPU SIR pipeline. It uses direct lighting and a procedural
tangent-space normal map; image-based lighting remains future work.
The `cubemap_showcase` samples a six-face `CubeMap` for a skybox and reflected
environment color in a native Rust shader. Material roughness selects a
box-filtered mip level as a compact reflection-blur approximation. This is not
split-sum IBL and is not yet a `samplerCube` binding in the SPIR-V command path.
The `stencil` scene uses a circular portal mask for both a textured cube and a
translucent overlay.
The renderer also supports deterministic 2× and 4× multisampling with
per-sample color, depth and stencil attachments; `--samples 1` keeps the
single-sample default.
Native Rust fragment shaders can opt into anisotropic texture sampling with
actual UV derivatives and a bounded 1×–16× tap count. `anisotropy_showcase`
compares trilinear filtering on the left with 16× anisotropy on the right.

```sh
cargo run --release -p silicon-cli -- run spirv_showcase --threads 4
cargo run --release -p silicon-cli -- run spirv_cutout --backend simd
cargo run --release -p silicon-cli -- render shadow_showcase --backend simd --threads 4 --output output/shadows.png
cargo run --release -p silicon-cli -- run spirv_cube
cargo run --release -p silicon-cli -- inspect-shader assets/shaders/textured.frag.spv
cargo run --release -p silicon-cli -- render-shaders assets/shaders/textured.vert.spv assets/shaders/textured.frag.spv --output output/glsl.png
```

See the [exact SPIR-V subset and binding contract](docs/spirv.md).
External compiler tools are needed only to regenerate shader fixtures.

```sh
cargo run --release -p silicon-cli -- render spirv_showcase --capture output/frame.silicon
cargo run --release -p silicon-cli -- inspect output/frame.silicon
cargo run --release -p silicon-cli -- replay output/frame.silicon --output output/replay.png
cargo run --release -p silicon-cli -- debug-pixel spirv_showcase --pixel 480,320
```

Captures embed actual buffers, textures, uniforms, pipeline state, commands and
shader bytecode. Replays compare byte-for-byte to the original framebuffer.
Pixel traces expose primitives, barycentrics, varying values, depth rejection,
shader instructions, sampled values and final color. Native closures currently
cannot be captured; recorded SIR commands, including translated SPIR-V, can.

```mermaid
flowchart LR
  Commands --> VertexShader[Vertex shader]
  VertexShader --> Clipping
  Clipping --> Tiles[Fixed-point tiled rasterizer]
  Tiles --> Depth[Depth / stencil]
  Depth --> FragmentShader[Fragment shader]
  FragmentShader --> Blend[Blend / attachments]
  Blend --> Framebuffer[CPU framebuffer]
  Framebuffer --> Output[PNG / window pixels]
```

## What works

| Stage | Implemented |
| --- | --- |
| API | Supported Rust API v1, versioned C ABI v1, and Vulkan-like Rust subset; synchronous command submission and CPU framebuffer readback |
| Memory | Owned typed vertex/index/uniform buffers; reference-counted command lifetimes |
| Geometry | Indexed/non-indexed triangles, homogeneous six-plane clipping, CW/CCW culling |
| Raster | Pixel-center top-left coverage, 8-bit subpixel precision, 16×16 tiles |
| Interpolation | Colors/UV/normals/custom vec4 varyings, perspective reconstruction, affine NDC depth |
| Shading | Native Lambert/Blinn reference; GLSL Blinn and Cook-Torrance GGX metallic/roughness through SPIR-V |
| Attachments | RGBA8/BGRA8, depth with 8 compare modes, stencil masks/operations |
| Texturing | RGBA8/RGB8/R8, nearest/bilinear/trilinear, clamp/repeat/mirror, mip generation/LOD |
| Shaders | Rust closures; bounded SIR VM; strict SPIR-V 1.0 → SIR; nested selections, Phi, early return/discard |
| Output merger | Replace, source alpha, additive and multiplicative blending; color/depth write enables |
| Execution | Scalar reference, optional SIMD coverage4 and NEON/SSE four-fragment SIR, disjoint worker bands |
| Tools | Headless rendering, native window, frame capture/replay/inspection, pixel trace, profiling, pipeline-cache probe, cargo-fuzz targets |

The opt-in SIMD path processes four coverage lanes and runs recorded SIR fragment
shaders in masked groups of four. Arithmetic spans fragments; vertex shaders,
native Rust closures, power and texture callbacks remain scalar. Tiled coverage
is implemented; full primitive binning and persistent worker pools are future work.
Parallel bands repeat geometry setup. Tests compare exact scalar/SIMD framebuffer
bytes and bitwise VM outputs/traces for all sixteen lane masks. See [SIMD details](docs/simd.md).

## Measure it

```sh
cargo run --release -p silicon-cli -- benchmark showcase --frames 30 --backend scalar --threads 1
cargo run --release -p silicon-cli -- benchmark spirv_showcase --frames 30 --backend simd --threads 4 --report output/frames.json
cargo run --release -p silicon-cli -- profile spirv_showcase --threads 4
cargo run --release -p silicon-cli -- pipeline-cache assets/shaders/textured.vert.spv assets/shaders/textured.frag.spv
python3 benchmarks/run.py --frames 30
```

Benchmarks include frame rendering, exclude image encoding/window presentation,
and report measured median/p95 and throughput. `--report` retains chronological
frame times, backend/worker configuration, actual shader packet occupancy and executed
instruction/sample/discard counts. Profile additionally instruments
fragment shader time; per-worker accumulated stage times can exceed wall time.
See [performance notes](docs/performance.md) for actual host measurements and
limits. The animation uses offline frames encoded at 24 fps; it is not an FPS
claim. No physical-GPU performance comparisons are made.

## Check it

```sh
cargo fmt --all --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Tests cover math, framebuffer bytes, shared-edge ownership, clipping, depth and
discard, perspective interpolation, texture addressing/mips, OBJ bounds, stencil,
shader validation/tracing, GLSL/SPIR-V translation and malformed modules,
command validation, exact capture replay, an approved
PNG and scalar/SIMD/parallel equivalence. Golden changes require an explicit
`cargo run --release --example shader_cube -- --bless` and image review.

## Boundaries

This is a research software GPU, not a conformant driver. SPIR-V support is
a narrow graphics subset with acyclic structured selections with a fixed binding contract. **General
SPIR-V/GLSL compatibility, WGSL, conformant Vulkan/OpenGL drivers, compute, JIT,
and games are not implemented.** The small Rust Vulkan-like subset is documented
separately; it is not binary compatible with Vulkan. Do not infer support from the long-term roadmap. MSAA and
GLSL shadow-map samples are implemented within the documented renderer subset. Multiple color
attachments and asynchronous queues are also future work.

No Mesa, LLVMpipe, SwiftShader, ANGLE, wgpu backend or existing rasterizer
produces these pixels. Image encoding and native window presentation are the
only graphics-adjacent external components. See [input safety](docs/security.md)
for bounds and limitations; this is not a hardened hostile-shader sandbox.

Contributions should bring a small reproducer, a correctness check and an actual
render or measurement. Improve the pipeline rather than special-case an asset.

Apache-2.0.
