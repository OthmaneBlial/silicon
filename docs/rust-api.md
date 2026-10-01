# Rust API

`silicon::api` is the supported integration surface for Rust applications.
Its `API_VERSION` is `1`; the crate's root re-exports and lower-level modules
remain available for existing users but may evolve with the implementation.
Incompatible changes to the `silicon::api` surface require an API version
change and release notes.

The API is synchronous and CPU-only. `Device` creates owned buffers, SPIR-V
shader modules and immutable pipelines, records command buffers, and submits
them to a caller-owned `Renderer`. The renderer fixes framebuffer dimensions
and owns the output. Submission structurally validates the command stream
before execution.

The same `Device` also exposes an experimental SIR compute map kernel through
`StorageBuffer`, `StorageLayout`, `ComputePipeline`, `dispatch_compute` and
`dispatch_compute_simd`. The `*_with_layouts` methods apply checked offset/stride
layouts to each input and the output. Dispatch runs one SIR program per 3D
invocation; SIMD uses four-lane SIR execution and a scalar tail. Shader-selected
SIR compute programs also support bounded shader-selected storage loads and
single-writer output stores. `create_compute_pipeline_with_shared_memory` adds
per-workgroup vec4 memory and `WorkgroupBarrier`; `dispatch_compute_with_atomics`
and its layout/SIMD variants accept separate mutable bindings for scalar f32
storage atomics. Shared and atomic programs use the scalar workgroup scheduler.
SIR atomics are scalar f32; translated SPIR-V also supports bounded uint32
add, exchange and compare-exchange storage operations. Command-buffer integration,
cross-workgroup barriers, other integer/vector atomics and compute-stage SPIR-V
beyond the documented subset are unsupported. `create_compute_pipeline_from_spirv`
translates the documented narrow SPIR-V 1.0 storage-buffer subset into
storage-only dispatch. Its compute subset includes fixed `vec4[N]` workgroup
arrays up to 4,096 values total and GLSL `barrier()`; see the
[compute contract](compute.md),
[vector-add example](../examples/compute_vector_add.rs),
[SPIR-V vector-add example](../examples/compute_spirv_vector_add.rs),
[SPIR-V inversion example](../examples/compute_spirv_invert.rs),
[SPIR-V shared-memory example](../examples/compute_spirv_shared.rs),
[SPIR-V uint atomic example](../examples/compute_spirv_atomic_uint.rs),
[benchmark example](../examples/compute_bench.rs), and
[atomic example](../examples/compute_atomics.rs).

Shader creation accepts at most 1 MiB per SPIR-V 1.0 module and translates only
the documented graphics subset. Pipeline creation requires a compatible
vertex/fragment pair. Vertex buffers are limited to 1M finite vertices, index
buffers to 3M indices, and uniform buffers to 64 finite `Vec4` values. Errors
are returned rather than silently dropping invalid resources or commands.

The runnable [Rust API example](../examples/rust_api.rs) creates shader modules,
links a pipeline, uploads triangle and uniform data, records a draw, submits it,
and saves the resulting framebuffer without opening a window:

```sh
cargo run --release --example rust_api
```

The generated image is `output/rust_api.png`.
