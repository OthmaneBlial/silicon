# Original GLSL fixtures

Project-authored shaders in this directory are Apache-2.0 assets. Their
committed SPIR-V was compiled by Khronos glslang 16.6.0 for Vulkan 1.0/SPIR-V
1.0 and validated with SPIRV-Tools 1.4.357.0. The explicitly attributed
Khronos Vulkan-Samples fixtures are documented below. The external tools only
produce/check bytecode; SILICON parses, translates and executes it entirely on
the CPU.

`textured.vert` + `textured.frag` power `spirv_cube`. `arithmetic.frag` covers
vec2/shuffle/division/dot/sampling; `negate.frag` checks float sign-bit
negation, including signed zero. `lit.vert` +
`lit.frag` power the full `spirv_showcase` lighting pipeline, and `shadow.frag`
powers the depth-textured `shadow_showcase`. `locals.frag` checks
local SSA snapshots, vector component stores and float/vector uniform padding.
`pbr.vert` and `pbr.frag` power `pbr_showcase`, including a procedural
tangent-space normal map on the sculpture. Image-based lighting is not yet
implemented.

`khronos_hello_triangle.vert` is a documented Apache-2.0 adaptation of the
Khronos Vulkan-Samples `hello_triangle` vertex shader; its fragment shader is
upstream-authored. The sample example uses the original three vertex positions
and RGB colors with alpha padded to 1. See [the provenance and adaptation
notes](../../docs/third-party-demo.md).
See [the subset and reproduction commands](../../docs/spirv.md).
