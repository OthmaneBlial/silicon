# Roadmap and evidence

The original brief describes seventy progressive phases, many explicitly
long-term. This repository ships working stages and labels the remaining work.

| Milestone | Current evidence |
| --- | --- |
| Math / framebuffer / first pixel | Unit tests and `pixel` PNG |
| Lines / triangle / barycentrics | Exact shared-edge, line and varying tests; `triangle` PNG |
| Depth / clipping / culling | Six-plane, depth-discard and culling tests |
| Vertex / fragment programmability | Native Rust closures and SIR programs |
| Perspective / textures / mipmaps | Numeric perspective/sampling tests; textured cube |
| Lit OBJ scene / normals | Original sculpture OBJ, normal matrix, Lambert/Blinn-Phong and point light |
| Window / loop / measured statistics | CLI `run`, finite frame mode, render time in title |
| Commands / buffers / shader VM | Owned typed buffers, validated commands, SIR shader cube |
| Frame capture and replay | Versioned capture owns buffers, textures, pipeline state, sample count, SIR modules and commands; byte-exact single- and multisample replay |
| Frame inspector | `silicon inspect` reports render passes, draw/triangle totals, resources, pipeline state and SIR module instruction counts; no GUI |
| Headless mode | `render` and `replay` write PNGs without opening a window |
| SIMD / tiled parallel rendering | Scalar reference, NEON/AVX2 coverage4, NEON/SSE masked four-fragment SIR, disjoint bands, bitwise equivalence tests |
| SPIR-V / ordinary GLSL | Strict binary parser + typed SIR lowering, textured cube, lit OBJ showcase, local/uniform, arithmetic, and float-negation fixtures |
| Divergent shader control flow | Nested GLSL selections, local/Phi merges, early return/discard; all-mask VM and attachment tests |
| Shadow maps / explicit-LOD sampling | Two SILICON CPU raster passes; 512×512 `Depth32Float` texture sampled by ordinary GLSL/SPIR-V; scalar and SIMD replay match |
| PBR material shading | Cook-Torrance GGX direct lighting, tangent-space normal mapping, and explicit-LOD `samplerCube` reflections in ordinary GLSL/SPIR-V, with capture replay |
| Cube-map sampling and visual reflections | Six-face `CubeMap` sampler, mip-selected roughness approximation, explicit and direct-input implicit GLSL sampling, scalar/SIMD pixel equivalence; transformed direction derivatives remain unsupported |
| Advanced textures (phase 50) | Bounded 2D texture arrays and color 3D volumes with checked construction, nearest/bilinear spatial filtering, mip selection/generation and explicit/implicit SIR samples; command binding is exercised through scalar and SIMD-requested renders. Cube maps and single-level depth textures are covered above. SPIR-V remains limited to its documented 2D/cube subset |
| MSAA | Deterministic 2×/4× coverage with separate color/depth/stencil samples, resolved framebuffer, CLI control, scalar/SIMD band equivalence |
| Anisotropic texture filtering | Derivative-aware 1×–16× sampling, minor-axis mip selection, focused unit test and side-by-side steep-angle scene |
| GPU profiler | Command, vertex, primitive setup, coverage/depth, shader, and blend/write timing with render counters; headless presentation is marked unmeasured |
| Stencil / transparency integration | Circular stencil portal constrains a textured cube and translucent overlay; scalar and SIMD four-band color/depth/stencil match |
| Compute (phases 45–49, first slices) | Scalar and four-lane SIMD SIR dispatch expose 3D global/local/workgroup IDs, read up to 12 vec4 buffers by map or shader-selected index, stage indexed output writes, provide zeroed per-workgroup shared memory with synchronized barriers, and support scalar f32 add/exchange/compare-exchange atomics; a strict SPIR-V 1.0 GLCompute subset runs GLSL vec4 addition, guarded partial-workgroup inversion and bounded shared-array broadcasts on the same CPU SIR path; shared-memory races and divergent barriers reject the whole dispatch; shared/atomic kernels use the scalar scheduler |
| Image regression tests | Approved SIR cube PNG, exact backend comparisons and <=1 channel-step tolerance; failures save `output/shader_cube.diff.png` |
| Fuzzing | cargo-fuzz targets cover SPIR-V parsing/lowering, capture/resource validation and bounded replay, plus triangle setup and texture sampling; see `docs/security.md` |
| Safety review | Explicit input/resource bounds and targeted malformed-input tests; this is not a hostile-workload sandbox or process-wide memory budget |
| JIT shaders | Not implemented; execution stays in the validated SIR interpreter |
| CPU backends | Scalar and four-lane SIMD paths; runtime selects NEON on ARM64 or AVX2 coverage on x86-64, with SSE2 shader arithmetic; no SIMD8, AVX-512 or JIT |
| Pipeline cache (phase 65) | Caller-owned 16-entry cache keyed by exact SPIR-V pairs and pipeline state; built-in cube scenes also retain linked pipeline `Arc`s across frames; reports hit/miss/eviction plus compile/lookup time and has a 100-hit CLI probe |
| Simple Rust graphics API (phase 66) | Versioned `silicon::api` facade, bounded SPIR-V shader/pipeline creation, owned typed buffers, command submission to an explicit renderer, and a runnable direct-triangle example |
| C API (phase 67) | Version-1 shared library and header expose opaque device/resource/command handles, synchronous draw submission and RGBA8 readback; standalone C client renders a SPIR-V triangle |
| Vulkan-like compatibility subset (phase 68) | Rust-only instance/device, typed buffers, RGBA8 images, SPIR-V pipelines, descriptor-like bindings, one offscreen render pass and synchronous queue; indexed textured triangle example. This is not Vulkan ABI, loader, or conformance support |
| Third-party demo (phase 69) | Khronos Vulkan-Samples `hello_triangle` at a pinned upstream commit; adapted vertex layout, upstream SPIR-V fragment shader, CPU framebuffer output and a color-interpolation integration test |
| DOOM (phase 70, in progress) | Loads a selected Freedoom Phase 1 map (E1M1 by default; map-specific sky textures render on F_SKY1 surfaces) and submits textured map geometry and two-sided masked middle textures through SILICON. Its checked-in E1M1 start view shows 29 cutout enemy and 59 pickup billboards, plus a pistol. BSP cells are split at sector boundaries before validated pieces fill flat geometry; leaf mesh bounds restore view-visible geometry when WAD child bounds are too tight, while the player leaf is always retained. BSP child bounds cull the horizontal view cone and per-mesh bounds test all six frustum planes; material batches draw front-to-back in coarse depth bands. The checked-in E1M1 start view submits 5,711 triangles in 259 draws, including 64 sky triangles, across 570 of 682 horizontal BSP leaves. The interactive prototype adds movement, basic collision, health, ammo, health-bonus and soul-sphere pickups, radiation suits that prevent sector-7 nukage damage for 60 seconds, green/blue armor and armor-bonus pickups, color-key card/skull pickups, a once-only counter for four sector-special-9 secrets, sector-special-7 nukage damage, special-1 random light flashes, synchronized special-12 slow strobes, player hitscan, armor-aware melee, hitscan, fireball and nukage damage, chance-based enemy pain reactions, Doom-timed idle states and 100-tic target memory, sector-portal enemy pursuit around blocked corridors, missed-shot pistol noise alerts through open sectors crossing at most one sound-blocking line, animated front-side special-1 doors, matching-key special-26/27/28 blue/yellow/red doors and a special-117 blazing use door with matching sector geometry and collision, walk-crossed special-2 doors that open matching sector tags, repeatable special-88 walk-triggered and special-62 use-triggered platforms that descend, wait and return to their starting height, and the one-shot special-23 use action that lowers tagged floors to their lowest neighbors. Special-11 exits advance to the next map in the same episode when its WAD marker exists; special-51 exits route to M9, whose ordinary exit returns to E1M4/E2M6/E3M7/E4M3 when present, carrying health, ammo, keys, and armor; episode-ending exits stop without a finale. The interactive prototype also includes pursuing melee attacks, line-of-sight hitscan attacks for former humans and shotgunners, 3D straight-line imp fireballs with height-aware hits and three-frame impacts, four-frame enemy walk cycles, three-frame attack poses, death sequences and persistent corpses, and eight camera-relative sprite views. BSP wall occlusion, per-column portal clipping, Doom's detailed actor navigation, enemy gib states, other sound events, other power-ups, other locked-door action variants, enemy-triggered platforms and other walk/use-triggered specials, and an episode-end screen remain. See [Freedoom checkpoint](freedoom.md) |

The compute path has bounded map dispatch, checked structured layouts,
shader-selected storage access, shared memory, barriers, transactional scalar
f32 storage atomics, and one strict SPIR-V storage-buffer path. Cross-workgroup
barriers, integer/vector atomics and broader compute-SPIR-V compatibility remain
future steps; a JIT stays an advanced experiment until more interpreter evidence
exists.
Cube maps use cross-face bilinear filtering; implicit SPIR-V sampling is limited
to an unmodified fragment input direction, and anisotropic SPIR-V sampling
remains future work.

Future research: multiple targets, full tile binning, loops and broader control flow,
cross-workgroup barriers, integer/vector atomics and broader compute-stage SPIR-V, transformed implicit sampling,
anisotropic SPIR-V sampling, JIT, DOOM BSP wall occlusion and per-column portal
clipping, advanced enemy states and full gameplay rules, and
possibly a software ray-tracing unit.

None of those future items are advertised as implemented. Conformant Vulkan/OpenGL
drivers, general SPIR-V compatibility, WGSL and full-game compatibility are
**unsupported**. Phase 70 is an experimental selected-map gameplay slice. The
small Rust-only subset does not provide Vulkan loader or binary compatibility.
No existing rasterizer, Mesa, LLVMpipe, SwiftShader, ANGLE or wgpu backend is used.
