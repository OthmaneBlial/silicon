# Experimental SIR compute

SILICON can dispatch a validated SIR `Program` once per global invocation ID
on the CPU. The first contract is a bounded map kernel: each invocation reads
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

Dispatch dimensions are three-dimensional, with x varying fastest. A workgroup
can contain at most 1,024 local invocations, and one dispatch can contain at
most 1,048,576 total invocations. Storage buffers contain at most 1,048,576
finite vec4 values. Dispatch results are staged and copied to the output only
after every invocation succeeds, so a failed shader leaves output unchanged.

This is an initial data-parallel SIR path, not general compute compatibility.
Programs cannot bind uniforms or textures, perform arbitrary storage addressing,
write multiple outputs, synchronize workgroups, use shared memory or atomics, or
load compute-stage SPIR-V. Dispatch is synchronous and scalar; command-buffer
capture/replay and C API support are not included yet.
