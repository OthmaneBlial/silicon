# SIR — Silicon Intermediate Representation

SIR is a bounded vec4 register machine. Programs contain 1..4096
instructions, 64 vec4 registers, 16 input slots, 64 uniform slots, 16 texture
slots and 8 outputs. Validation rejects undefined/out-of-range register reads,
invalid swizzles, nonfinite constants and missing output 0 on a returning path. Deserialization calls
the same validation. Execution checks resource bindings and rejects nonfinite
arithmetic results with the instruction number. The independent scalar executor
is the reference. `Program::execute4` executes masked four-fragment packets with
component registers spanning fragments; programs with loops use the scalar
executor once per active lane. See [SIMD execution](simd.md).

Operations: input/uniform/constant loads; component add/subtract/multiply/divide
and power; elementwise round/round-even/trunc/floor/ceil/fract, trigonometric,
exponential/logarithmic and square-root math; min/max/mix; dot3/dot4;
length/normalization of 1..4 components;
legacy normalize3; saturation; swizzle and lane composition; row-major matrix-vector
multiply; filtered texture sample; output store; comparisons, logical operations,
component selection, structured `If`/`Else`/`EndIf`, `Merge`, `LoopStart` and
`LoopEnd`, `Return` and `Discard`. Compute also has shared-memory loads/stores
and a workgroup barrier. Values are f32 vec4s, including scalar splats and
boolean 0/1 components. Integer types, shader depth writes and JIT compilation
remain unsupported. Compute
supports scalar f32 storage atomics on vec4 x components. Bounded scalar and
SIMD4 [SIR compute dispatch](compute.md)
are separate from the graphics shader path. A narrow SPIR-V 1.0 compute
storage-buffer subset also translates externally compiled GLSL, including
bounded uint atomics, into these
instructions; general SPIR-V compatibility remains unsupported. See the
[accepted SPIR-V subset](spirv.md).

Selections nest at most 64 levels and require an `Else` (possibly empty). `If` tests
condition.x for nonzero. The scalar VM skips the untaken branch; packet execution
carries separate active masks, preserves the other branch's register values and
reconverges surviving lanes at `EndIf`. `Return` and `Discard` never reactivate.
Inactive branches perform no resource reads, samples, output writes or traces.
Validation tracks defined registers/outputs separately on each live path.

`LoopStart` tests condition.x before each iteration and skips to its paired
`LoopEnd` when zero; otherwise execution repeats the enclosed range. The loop
body must update its condition for termination. Validation accounts for the
zero-iteration path, rejects loops crossing selection boundaries, and pairs at
most 64 nested loops. Each invocation is limited to 65,536 dynamically visited
instructions; exceeding the limit returns an error. SPIR-V `OpLoopMerge` and
loop-carried `OpPhi` are not yet lowered into these SIR instructions.

`Merge { dst, a, b }` selects the immediately preceding selection's true/false
value. Consecutive merges implement SPIR-V Phi and local snapshots; another
instruction closes that merge window. Unlike `Select`, an unchosen merge value
need not have executed on that lane. `Select` reads both alternatives and chooses
per component. Executed instruction counters and traces omit skipped instructions;
structural Else/EndIf count for lanes still live in their parent region.

Vertex inputs: 0 = position with w=1; 1 = color; 2 = UV; 3 = normal with w=0.
Vertex outputs: 0 = homogeneous position; 1 = color; 2 = UV; 3 = normal;
4 = world position. Fragment inputs 0..3 are these four interpolated varyings.
Fragment output 0 is RGBA. `Sample` uses its source register's xy as UV and z as
explicit LOD; `SampleImplicit` gets LOD from a separate per-texture invocation
array, so derivative metadata does not contaminate SPIR-V vector arithmetic.
`SampleCube` uses xyz direction and w explicit LOD; `SampleCubeImplicit` gets its
LOD from the same per-texture array.
`SampleArray` uses xy UV, z integer layer and w explicit LOD;
`SampleArrayImplicit` replaces w with derivative-derived LOD. `Sample3D` uses
xyz volume coordinates and w explicit LOD; `Sample3DImplicit` derives that LOD
from the interpolated coordinate derivatives. The SPIR-V translator currently
emits only 2D and cube samples.
`StorageLoad`, `StorageStore`, `AtomicAdd`, `AtomicExchange`,
`AtomicCompareExchange`, `SharedLoad`, `SharedStore` and `WorkgroupBarrier` are
compute-only. Indexed accesses use checked vec4 addresses; the barrier
synchronizes every invocation in one workgroup. See the
[compute contract](compute.md).
`Discard` ends the fragment invocation without color, depth or stencil-pass writes.
Vertex pipelines reject it before submission changes the framebuffer. Native Rust
shaders may also return `None` to discard.

`demo::shader_cube` supplies MVP/model rows and the lighting binding contract, and
binds a checker texture. It records vertex/index/uniform buffers, pipeline state,
shader bytecode and textures into an owned command stream. `FrameCapture` embeds
all of those resources, the sample count, and commands in versioned JSON.
Version 2 records multisample state; legacy version-1 captures without a
sample-count field remain single-sample. A replay does not reconstruct a scene
from its name; it submits the captured bytes and programs. The roundtrip test
compares exact framebuffer bytes.
Golden tests additionally compare an approved PNG with one quantization step of
cross-platform tolerance.

```sh
cargo run --release -p silicon-cli -- render shader_cube --samples 4 --capture output/cube.silicon
cargo run --release -p silicon-cli -- inspect output/cube.silicon
cargo run --release -p silicon-cli -- replay output/cube.silicon
cargo run --release -p silicon-cli -- debug-pixel shader_cube --pixel 480,320
```

Pixel traces show every covering primitive, local triangle barycentrics, UV,
normal/world varyings, old/new depth, comparison outcomes and shader color.
The SIR debugger also prints executed instructions and sampled values for
accepted fragments. Rejected fragments show their rejection without running a
shader for display purposes. Barycentrics refer to the emitted clipped triangle,
not necessarily the original unclipped triangle.
