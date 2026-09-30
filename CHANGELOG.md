# Changelog

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
