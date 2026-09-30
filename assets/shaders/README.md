# Original GLSL fixtures

These shaders are project-authored Apache-2.0 assets. Their committed SPIR-V was
compiled by Khronos glslang 16.6.0 for Vulkan 1.0/SPIR-V 1.0 and validated with
SPIRV-Tools 1.4.357.0. The external tools only produce/check bytecode; SILICON
parses, translates and executes it entirely on the CPU.

`textured.vert` + `textured.frag` power `spirv_cube`. `arithmetic.frag` is the
numeric vec2/shuffle/division/dot/sampling regression fixture. `lit.vert` +
`lit.frag` power the full `spirv_showcase` lighting pipeline. `locals.frag` checks
local SSA snapshots, vector component stores and float/vector uniform padding. See
[the subset and reproduction commands](../../docs/spirv.md).
