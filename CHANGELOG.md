# Changelog

## Unreleased

- Add one-shot, use-triggered Doom special-29 doors that raise and close tagged
  sectors through the existing door animation and collision path.
- Add use-triggered Doom special-103 doors that open tagged sectors and remain
  open.
- Route Doom special-51 exits to the episode's M9 map and ordinary M9 exits back
  to the original episode path when the target map exists in the WAD.
- Render and collect all six Doom keycard/skull map things; red, yellow, and blue
  keys now open their matching special-28, special-27, and special-26 doors.
- Add Doom green/blue armor and armor bonuses, with damage absorption for
  melee, hitscan, fireball, and nukage attacks.
- Render and collect Doom health bonuses and soul spheres, following their
  separate 100/200 health caps.
- Render and collect radiation suits; active suits prevent nukage-sector damage
  for 60 seconds and refresh when another suit is collected.

## 0.7.0 — 2026-10-01

- Make the stencil portal a shared CLI/library scene and constrain its
  translucent overlay with the same stencil mask.
- Verify the portal boundary and exact color/depth/stencil output between scalar
  rendering and SIMD four-band rendering.

## 0.6.0 — 2026-10-01

- Add validated single-level `Depth32Float` textures and explicit scalar-LOD
  sampling for transformed fragment coordinates.
- Render the OBJ showcase in a CPU shadow-depth pass and sample the captured map
  from ordinary GLSL/SPIR-V; include the scene in the CLI and gallery.
- Verify depth texture serialization and exact color/depth/stencil replay across
  scalar and SIMD four-band execution.

## 0.5.0 — 2026-10-01

- Execute nested structured SIR selections with per-fragment divergence masks,
  reconvergence, comparisons/logical math, select/merge, early return and discard.
- Lower ordinary GLSL/SPIR-V acyclic branches, scalar bool, local snapshots and
  Phi values with predecessor, type and per-path definition validation.
- Add the captured `spirv_cutout` scene and real glslang/SPIRV-Tools fixtures;
  scalar, packet and band rendering preserve exact color/depth/stencil.
- Count actual executed shader instructions, texture samples and discarded
  fragments, including physical vertex work repeated by worker bands.
- Reject vertex discard before any command mutates the framebuffer; bound
  selection nesting and reject loops, switches and overlapping graph regions.

## 0.4.0 — 2026-09-30

Recorded SIR fragment shaders now execute in masked groups of four with NEON
on ARM64 and baseline SSE/SSE2 on x86-64. Coverage, early depth/stencil rejection,
per-pixel attachment operations and rejected pixel traces retain their behavior.
Vertex shaders and native Rust closures remain scalar. SIMD remains opt-in.

The VM retains resource/mask validation and finite-result checks. Tests compare
every lane mask, output/intermediate value bits, samples, errors and exact
color/depth/stencil replay with one/four workers. Profiling exposes actual packet
count/occupancy; `benchmark --report` preserves chronological frame times and
configuration. A sequential alternating benchmark script compares an existing
release with the current scalar/SIMD executable, recording binary hashes.

## 0.3.0 — 2026-09-30

The full lit OBJ showcase now executes ordinary GLSL through SPIR-V and SIR:
30 recorded draws, 12,588 triangles, normal transforms, Lambert/Blinn-Phong,
directional/point lights, textures, fog and display transfer. The compiler adds
float/vector locals, snapshot/component stores, one-member float/vector uniform
blocks and checked GLSL.std.450 math. SSA temporaries recycle into the existing
64 runtime registers. The scene shares geometry/materials with the native reference.
Capture exports now create missing parent directories, matching PNG export.

Four additional tests verify locals and padding, malformed extended instructions,
register lifetimes/capacity, native lighting/depth equivalence and exact captured
scalar/SIMD/four-band replay. Branches, loops, function calls, general shader/API
compatibility and compute remain unsupported.

## 0.2.0 — 2026-09-30

Strict SPIR-V 1.0 binary parsing and typed SIR lowering; ordinary externally
compiled GLSL vertex/fragment shaders rendered through SILICON's CPU VM;
shader inspection and external shader rendering; stage/varying validation;
lane composition and separate per-texture implicit LOD; captured translated
program replay. Four additional tests cover exact reference/parallel pixels,
GLSL arithmetic, malformed modules and 1500 bounded binary mutations.

This is a limited straight-line graphics subset, not general SPIR-V/GLSL or
Vulkan conformance. See the documented binding and sampling restrictions.

## 0.1.0 — 2026-09-30

Initial experimental software-GPU release: CPU-owned framebuffer; fixed-point
tiled rasterization; clipping; depth/stencil/blending; perspective-correct
varyings; filtered mipmapped textures; programmable native shaders; a bounded
SIR interpreter; owned command resources and captured frame replay; optional
NEON/AVX2 coverage and parallel bands; lit OBJ showcase; headless/native-window
CLI; pixel traces, profiling and reproducible benchmark scripts.

No SPIR-V, Vulkan/OpenGL compatibility, compute or games in this release.
