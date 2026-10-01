# Experimental SIR compute

SILICON can dispatch a validated SIR `Program` once per global invocation ID
on the CPU through either a scalar reference path or a four-lane SIMD path. The
first contract is a bounded map kernel: each invocation reads
the same linear element from zero or more `StorageBuffer`s and writes one vec4
to the matching output element. Input slots 0–3 are global ID, local ID,
workgroup ID and total workgroup count; read buffers occupy slots 4 onward.
Output slot 0 is the per-invocation result.

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

Dispatch dimensions are three-dimensional, with x varying fastest. A workgroup
can contain at most 1,024 local invocations, and one dispatch can contain at
most 1,048,576 total invocations. Storage buffers contain at most 1,048,576
finite vec4 values. Dispatch results are staged and copied to the output only
after every invocation succeeds, so a failed shader leaves output unchanged.

This is an initial data-parallel SIR path, not general compute compatibility.
Programs cannot bind uniforms or textures, perform arbitrary storage addressing,
write multiple outputs, synchronize workgroups, use shared memory or atomics, or
load compute-stage SPIR-V. Dispatch is synchronous; command-buffer capture/replay
and C API support are not included yet.
