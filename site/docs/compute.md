# Experimental SIR compute

SILICON can dispatch a validated SIR `Program` once per global invocation ID
on the CPU through either a scalar reference path or a four-lane SIMD path. The
first contract is a bounded map kernel: each invocation reads
the same linear element from zero or more `StorageBuffer`s and writes one vec4
to the matching output element. Input slots 0–3 are global ID, local ID,
workgroup ID and total workgroup count; read buffers occupy slots 4 onward.
Output slot 0 is the per-invocation result.

`dispatch_compute` uses packed buffers. `dispatch_compute_with_layouts` and its
SIMD4 counterpart accept one `StorageLayout` per input plus an output layout.
For invocation `i`, each layout addresses vec4 element `offset + i * stride`;
this supports interleaved records while retaining one same-position element per
invocation. Dispatch checks stride arithmetic and every addressed range before
running.

SIR `StorageLoad { dst, buffer, index }` reads an absolute vec4 index from one
bound input buffer, and `StorageStore { index, src }` stages an absolute vec4
write to the output buffer. The index comes from `index.x` and must be a finite,
non-negative integer. Shader-selected loads check the chosen buffer and address
at execution time; layout-based `Input` reads still use their checked
offset/stride. Stores are checked against the output length, capped at 1,048,576
per dispatch, and committed only after all invocations succeed. Duplicate
shader-selected destinations fail the dispatch before commit; explicit stores
run after map output writes and can overwrite them. This provides deterministic
single-writer scatter.

`create_compute_pipeline_with_shared_memory` reserves up to 4,096 vec4s (64 KiB)
per workgroup. The memory starts at zero for each group. SIR
`SharedLoad { dst, index }` and `SharedStore { index, src }` use the same checked
`index.x` format. `WorkgroupBarrier` pauses each invocation until all local
invocations reach that same barrier, then resumes them with shared writes
visible. If invocations take different barrier paths, dispatch returns a
divergence error instead of waiting indefinitely.

Within one barrier interval, shared accesses by different invocations may read
the same element, but conflicting cross-invocation read/write or write/write
accesses return a race error. Same-invocation accesses are ordered. A barrier
starts a new interval. The scalar workgroup scheduler runs shared-memory and
atomic programs for both dispatch entry points; the SIMD dispatch request uses
this scalar scheduler for correctness. Other programs retain the SIMD4 path.

`AtomicAdd`, `AtomicExchange` and `AtomicCompareExchange` operate on a separate
mutable atomic-buffer binding selected by `buffer`. These are scalar f32
operations on the addressed vec4's x component; yzw are preserved. Operands use
their x component, and each instruction returns the previous scalar splatted to
vec4. Compare-exchange compares f32 bit patterns. Values and addition results
must remain finite. The bindings are available through
`dispatch_compute_with_atomics` and `dispatch_compute_with_layouts_and_atomics`;
the SIMD-requested variants use the same scalar workgroup scheduler.

Atomic buffers are checked and copied into dispatch-local `AtomicU32` cells,
using sequentially consistent host atomics. Invocations execute in deterministic
workgroup/local order today; atomics linearize across workgroups, but there is no
cross-workgroup barrier. Atomic-buffer changes commit only after the full
dispatch succeeds, alongside the output. Failed shaders leave all bound buffers
unchanged.

The runnable atomic proof sums values across four workgroups and verifies every
returned prefix:

```sh
cargo run --release --example compute_atomics
```

Run the vector-add proof:

```sh
cargo run --release --example compute_vector_add
```

The example adds 4,096 vec4 pairs, verifies every output against Rust's scalar
reference, and reports the actual invocation, workgroup and SIR instruction
counts plus measured dispatch time. It does not compare against a physical GPU.

Compare a release Rust loop with scalar and SIMD4 SIR for vector addition and
4×4 matrix-vector transforms:

```sh
cargo run --release --example compute_bench
```

The benchmark verifies every result before timing, uses one warm-up and eleven
samples, and reports the median and full sample range. Dispatch timing includes
ID setup, interpreter execution, storage reads and output commit; buffer setup
and result checking are outside the timed region. It has no physical-GPU
comparison. SIMD4 reuses SIR's existing NEON/SSE packet executor, and a final
partial packet falls back to scalar execution.
See the [recorded Apple M2 sample](../benchmarks/compute-apple-m2-2026-10-01.json)
and [measurement notes](performance.md#sir-compute-workloads).

Dispatch dimensions are three-dimensional, with x varying fastest. A workgroup
can contain at most 1,024 local invocations, and one dispatch can contain at
most 1,048,576 total invocations. Storage buffers contain at most 1,048,576
finite vec4 values. Dispatch results are staged and copied to the output only
after every invocation succeeds, so a failed shader leaves output unchanged.

## GLSL compute through SPIR-V

`Device::create_compute_pipeline_from_spirv` accepts one narrow SPIR-V 1.0
compute path and lowers it into the same SIR dispatcher. It supports a declared
`LocalSize` up to 1,024 invocations, the `uvec3` global/local/workgroup ID and
workgroup-count built-ins, and set-0 read-only `vec4[]` storage buffers at
bindings `0..N-1` followed by one write-only `vec4[]` output at binding `N`.
The runtime-array stride must be 16 bytes and its block member offset must be
zero. At most 12 input buffers are accepted. Loads and writes use shader
indices, and every address is checked during dispatch; explicit writes are
staged and only modify elements the shader writes.

The checked-in GLSL vector-add and one-minus inversion kernels are compiled
offline with glslang. Run them without a GPU or display:

```sh
glslangValidator -V --target-env vulkan1.0 \
  assets/shaders/compute_vector_add.comp \
  -o assets/shaders/compute_vector_add.comp.spv
cargo run --release --example compute_spirv_vector_add
cargo run --release --example compute_spirv_invert
```

These examples prove GLSL → SPIR-V → SIR → CPU storage-buffer execution with
vector addition and one-minus inversion. The host chooses the workgroup count,
so each sample dispatches exactly 4,096 invocations for 4,096 elements; shaders
with a partial final workgroup must guard their own indices. Compute SPIR-V does
not yet support shared memory, barriers, atomics, textures, uniforms, general
integer arithmetic, loops, or storage images.

This remains an initial data-parallel path, not general compute compatibility.
Dispatch is synchronous; command-buffer capture/replay and C API support are not
included yet.
