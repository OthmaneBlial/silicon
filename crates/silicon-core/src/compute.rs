use crate::{Device, Result, Vec4};
use silicon_shader::{AtomicOperation, Instruction, Program};
use std::sync::atomic::{AtomicU32, Ordering};

const MAX_WORKGROUP_SIZE: usize = 1024;
const MAX_DISPATCH_INVOCATIONS: usize = 1_048_576;
const MAX_STORAGE_VECTORS: usize = 1_048_576;
const MAX_STORAGE_WRITES: usize = MAX_DISPATCH_INVOCATIONS;
const MAX_SHARED_VECTORS: usize = 4096;
const MAX_INPUT_BUFFERS: usize = 12;
const MAX_ATOMIC_BUFFERS: usize = 12;
const SIMT_WIDTH: usize = 4;

/// Address a vec4 element as `offset + invocation * stride`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageLayout {
    offset: usize,
    stride: usize,
}

impl StorageLayout {
    pub const PACKED: Self = Self {
        offset: 0,
        stride: 1,
    };

    /// Create a vec4-element layout. Dispatch checks the full addressed range.
    pub fn new(offset: usize, stride: usize) -> Result<Self> {
        if stride == 0 {
            return Err("storage layout stride must be nonzero".into());
        }
        Ok(Self { offset, stride })
    }

    pub const fn offset(self) -> usize {
        self.offset
    }

    pub const fn stride(self) -> usize {
        self.stride
    }

    fn index(self, invocation: usize) -> usize {
        self.offset + invocation * self.stride
    }
}

/// An owned vec4 storage resource for the experimental SIR compute path.
pub struct StorageBuffer {
    values: Vec<Vec4>,
}

impl StorageBuffer {
    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn as_slice(&self) -> &[Vec4] {
        &self.values
    }

    pub fn as_mut_slice(&mut self) -> &mut [Vec4] {
        &mut self.values
    }
}

/// One SIR program invocation per global ID; output slot 0 writes one vec4.
#[derive(Clone)]
pub struct ComputePipeline {
    program: Program,
    local_size: [u32; 3],
    shared_memory_vec4s: usize,
    storage_input_count: Option<usize>,
    storage_only: bool,
}

impl ComputePipeline {
    pub fn local_size(&self) -> [u32; 3] {
        self.local_size
    }

    pub fn shared_memory_vec4s(&self) -> usize {
        self.shared_memory_vec4s
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ComputeStats {
    pub workgroups: u64,
    pub invocations: u64,
    pub instructions: u64,
}

impl Device {
    pub fn create_storage_buffer(&self, values: Vec<Vec4>) -> Result<StorageBuffer> {
        if values.len() > MAX_STORAGE_VECTORS || values.iter().any(|value| !value.is_finite()) {
            return Err("storage buffer requires at most 1048576 finite vec4 values".into());
        }
        Ok(StorageBuffer { values })
    }

    pub fn create_compute_pipeline(
        &self,
        program: Program,
        local_size: [u32; 3],
    ) -> Result<ComputePipeline> {
        self.create_compute_pipeline_with_shared_memory(program, local_size, 0)
    }

    /// Translate the supported compute SPIR-V subset into the existing SIR dispatcher.
    pub fn create_compute_pipeline_from_spirv(&self, bytes: &[u8]) -> Result<ComputePipeline> {
        let compiled = silicon_shader::spirv::Module::parse(bytes)?.translate()?;
        if compiled.stage != silicon_shader::spirv::Stage::Compute {
            return Err("compute pipeline requires a Compute SPIR-V entry point".into());
        }
        let input_count = usize::from(compiled.storage_input_count);
        if input_count > MAX_INPUT_BUFFERS {
            return Err("compute SPIR-V supports at most 12 input storage buffers".into());
        }
        let mut pipeline = self.create_compute_pipeline(compiled.program, compiled.local_size)?;
        pipeline.storage_input_count = Some(input_count);
        pipeline.storage_only = true;
        Ok(pipeline)
    }

    /// Create a pipeline with zero-initialized per-workgroup vec4 memory.
    pub fn create_compute_pipeline_with_shared_memory(
        &self,
        program: Program,
        local_size: [u32; 3],
        shared_memory_vec4s: usize,
    ) -> Result<ComputePipeline> {
        if shared_memory_vec4s > MAX_SHARED_VECTORS {
            return Err("compute workgroup shared memory exceeds 4096 vec4 values".into());
        }
        let invocations = local_size
            .into_iter()
            .try_fold(1usize, |n, axis| n.checked_mul(axis as usize))
            .ok_or("compute workgroup size overflows")?;
        if local_size.contains(&0) || invocations > MAX_WORKGROUP_SIZE {
            return Err("compute workgroup requires 1..1024 local invocations".into());
        }
        if shared_memory_vec4s == 0
            && program.instructions().iter().any(|op| {
                matches!(
                    op,
                    Instruction::SharedLoad { .. } | Instruction::SharedStore { .. }
                )
            })
        {
            return Err(
                "compute shared-memory instructions require a nonzero shared-memory binding".into(),
            );
        }
        for op in program.instructions() {
            match op {
                Instruction::Discard => {
                    return Err("compute programs do not support fragment discard".into());
                }
                Instruction::Uniform { .. } => {
                    return Err("compute programs do not bind uniform buffers".into());
                }
                Instruction::Mat4 { .. } => {
                    return Err("compute programs do not use matrix uniforms".into());
                }
                Instruction::Sample { .. }
                | Instruction::SampleImplicit { .. }
                | Instruction::SampleArray { .. }
                | Instruction::SampleArrayImplicit { .. }
                | Instruction::Sample3D { .. }
                | Instruction::Sample3DImplicit { .. }
                | Instruction::SampleCube { .. }
                | Instruction::SampleCubeImplicit { .. } => {
                    return Err("compute programs do not sample textures".into());
                }
                Instruction::Output { slot, .. } if *slot != 0 => {
                    return Err("compute programs write only output slot 0".into());
                }
                _ => {}
            }
        }
        Ok(ComputePipeline {
            program,
            local_size,
            shared_memory_vec4s,
            storage_input_count: None,
            storage_only: false,
        })
    }

    /// Dispatches a bounded map kernel. Input slots 0–3 are global ID, local ID,
    /// workgroup ID and workgroup count; slots 4 onward read the matching element
    /// from each input buffer. Output slot 0 writes the matching output element.
    pub fn dispatch_compute(
        &self,
        pipeline: &ComputePipeline,
        workgroups: [u32; 3],
        inputs: &[&StorageBuffer],
        output: &mut StorageBuffer,
    ) -> Result<ComputeStats> {
        self.dispatch_compute_with_atomics(pipeline, workgroups, inputs, &mut [], output)
    }

    /// Dispatches with mutable storage buffers available to SIR atomic operations.
    pub fn dispatch_compute_with_atomics(
        &self,
        pipeline: &ComputePipeline,
        workgroups: [u32; 3],
        inputs: &[&StorageBuffer],
        atomic_buffers: &mut [&mut StorageBuffer],
        output: &mut StorageBuffer,
    ) -> Result<ComputeStats> {
        let layouts = packed_layouts(inputs.len())?;
        self.dispatch_compute_with_layouts_and_atomics(
            pipeline,
            workgroups,
            inputs,
            &layouts[..inputs.len()],
            atomic_buffers,
            output,
            StorageLayout::PACKED,
        )
    }

    /// Dispatches with fixed per-buffer offset and stride in vec4 elements.
    /// Input layout `i` applies to input buffer `i`; the output layout controls
    /// where each invocation result is written.
    pub fn dispatch_compute_with_layouts(
        &self,
        pipeline: &ComputePipeline,
        workgroups: [u32; 3],
        inputs: &[&StorageBuffer],
        input_layouts: &[StorageLayout],
        output: &mut StorageBuffer,
        output_layout: StorageLayout,
    ) -> Result<ComputeStats> {
        self.dispatch_compute_with_layouts_and_atomics(
            pipeline,
            workgroups,
            inputs,
            input_layouts,
            &mut [],
            output,
            output_layout,
        )
    }

    /// Layout-based dispatch with separate mutable storage bindings for atomics.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_compute_with_layouts_and_atomics(
        &self,
        pipeline: &ComputePipeline,
        workgroups: [u32; 3],
        inputs: &[&StorageBuffer],
        input_layouts: &[StorageLayout],
        atomic_buffers: &mut [&mut StorageBuffer],
        output: &mut StorageBuffer,
        output_layout: StorageLayout,
    ) -> Result<ComputeStats> {
        validate_atomic_bindings(&pipeline.program, atomic_buffers.len())?;
        let Some(shape) = dispatch_shape(
            pipeline,
            workgroups,
            inputs,
            input_layouts,
            output,
            output_layout,
        )?
        else {
            return Ok(ComputeStats::default());
        };

        if uses_workgroup_executor(&pipeline.program) {
            return dispatch_workgroups_scalar(
                pipeline,
                workgroups,
                inputs,
                input_layouts,
                atomic_buffers,
                output,
                output_layout,
                shape,
            );
        }

        let mut results = vec![Vec4::ZERO; shape.invocations];
        let mut stores = StagedWrites::new(output.len());
        let mut stats = ComputeStats {
            workgroups: shape.group_count as u64,
            invocations: shape.invocations as u64,
            ..ComputeStats::default()
        };
        let [size_x, size_y, _] = shape.global_size;
        for wz in 0..workgroups[2] {
            for wy in 0..workgroups[1] {
                for wx in 0..workgroups[0] {
                    for lz in 0..pipeline.local_size[2] {
                        for ly in 0..pipeline.local_size[1] {
                            for lx in 0..pipeline.local_size[0] {
                                let global = [
                                    wx * pipeline.local_size[0] + lx,
                                    wy * pipeline.local_size[1] + ly,
                                    wz * pipeline.local_size[2] + lz,
                                ];
                                let linear = global[0] as usize
                                    + size_x as usize
                                        * (global[1] as usize
                                            + size_y as usize * global[2] as usize);
                                let (value, instructions) = scalar_invocation(
                                    pipeline,
                                    workgroups,
                                    inputs,
                                    input_layouts,
                                    [global, [lx, ly, lz], [wx, wy, wz]],
                                    linear,
                                    &mut stores,
                                )?;
                                results[linear] = value;
                                stats.instructions += instructions;
                            }
                        }
                    }
                }
            }
        }
        commit_results(
            output,
            output_layout,
            &results,
            stores,
            !pipeline.storage_only,
        )?;
        Ok(stats)
    }

    /// Dispatches through the existing four-lane SIR executor (NEON on ARM64,
    /// SSE on x86-64); a short final packet uses the scalar reference path.
    pub fn dispatch_compute_simd(
        &self,
        pipeline: &ComputePipeline,
        workgroups: [u32; 3],
        inputs: &[&StorageBuffer],
        output: &mut StorageBuffer,
    ) -> Result<ComputeStats> {
        self.dispatch_compute_simd_with_atomics(pipeline, workgroups, inputs, &mut [], output)
    }

    /// SIMD-requested dispatch with mutable storage bindings for atomics.
    pub fn dispatch_compute_simd_with_atomics(
        &self,
        pipeline: &ComputePipeline,
        workgroups: [u32; 3],
        inputs: &[&StorageBuffer],
        atomic_buffers: &mut [&mut StorageBuffer],
        output: &mut StorageBuffer,
    ) -> Result<ComputeStats> {
        let layouts = packed_layouts(inputs.len())?;
        self.dispatch_compute_simd_with_layouts_and_atomics(
            pipeline,
            workgroups,
            inputs,
            &layouts[..inputs.len()],
            atomic_buffers,
            output,
            StorageLayout::PACKED,
        )
    }

    /// SIMD4 equivalent of `dispatch_compute_with_layouts`.
    pub fn dispatch_compute_simd_with_layouts(
        &self,
        pipeline: &ComputePipeline,
        workgroups: [u32; 3],
        inputs: &[&StorageBuffer],
        input_layouts: &[StorageLayout],
        output: &mut StorageBuffer,
        output_layout: StorageLayout,
    ) -> Result<ComputeStats> {
        self.dispatch_compute_simd_with_layouts_and_atomics(
            pipeline,
            workgroups,
            inputs,
            input_layouts,
            &mut [],
            output,
            output_layout,
        )
    }

    /// SIMD-requested layout dispatch with mutable storage bindings for atomics.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_compute_simd_with_layouts_and_atomics(
        &self,
        pipeline: &ComputePipeline,
        workgroups: [u32; 3],
        inputs: &[&StorageBuffer],
        input_layouts: &[StorageLayout],
        atomic_buffers: &mut [&mut StorageBuffer],
        output: &mut StorageBuffer,
        output_layout: StorageLayout,
    ) -> Result<ComputeStats> {
        if uses_workgroup_executor(&pipeline.program) {
            return self.dispatch_compute_with_layouts_and_atomics(
                pipeline,
                workgroups,
                inputs,
                input_layouts,
                atomic_buffers,
                output,
                output_layout,
            );
        }
        validate_atomic_bindings(&pipeline.program, atomic_buffers.len())?;
        let Some(shape) = dispatch_shape(
            pipeline,
            workgroups,
            inputs,
            input_layouts,
            output,
            output_layout,
        )?
        else {
            return Ok(ComputeStats::default());
        };
        let input_count = 4 + inputs.len();
        let mut results = vec![Vec4::ZERO; shape.invocations];
        let mut stores = StagedWrites::new(output.len());
        let mut stats = ComputeStats {
            workgroups: shape.group_count as u64,
            invocations: shape.invocations as u64,
            ..ComputeStats::default()
        };
        let mut first = 0;
        while first + SIMT_WIDTH <= shape.invocations {
            let mut lane_inputs = [[Vec4::ZERO; 16]; SIMT_WIDTH];
            for (lane, values) in lane_inputs.iter_mut().enumerate() {
                let linear = first + lane;
                let [global, local, group] =
                    invocation_ids(linear, shape.global_size, pipeline.local_size);
                values[0] = id(global);
                values[1] = id(local);
                values[2] = id(group);
                values[3] = id(workgroups);
                for (slot, buffer) in inputs.iter().enumerate() {
                    values[4 + slot] = buffer.values[input_layouts[slot].index(linear)];
                }
            }
            let input_lanes = [
                &lane_inputs[0][..input_count],
                &lane_inputs[1][..input_count],
                &lane_inputs[2][..input_count],
                &lane_inputs[3][..input_count],
            ];
            let executions = pipeline.program.execute4_with_storage(
                input_lanes,
                &[],
                [&[], &[], &[], &[]],
                0b1111,
                |_, _, _| Err("compute programs do not sample textures".into()),
                |_, buffer, index| {
                    inputs
                        .get(buffer)
                        .and_then(|buffer| buffer.values.get(index))
                        .copied()
                        .ok_or_else(|| {
                            format!(
                                "storage load from input buffer {buffer} at vec4 {index} is out of bounds"
                            )
                        })
                },
                |_, index, value| stores.stage(index, value),
                [false; SIMT_WIDTH],
            )?;
            for (lane, execution) in executions.into_iter().enumerate() {
                results[first + lane] = execution.outputs[0];
                stats.instructions += execution.instructions as u64;
            }
            first += SIMT_WIDTH;
        }
        for (linear, result) in results.iter_mut().enumerate().skip(first) {
            let [global, local, group] =
                invocation_ids(linear, shape.global_size, pipeline.local_size);
            let (value, instructions) = scalar_invocation(
                pipeline,
                workgroups,
                inputs,
                input_layouts,
                [global, local, group],
                linear,
                &mut stores,
            )?;
            *result = value;
            stats.instructions += instructions;
        }
        commit_results(
            output,
            output_layout,
            &results,
            stores,
            !pipeline.storage_only,
        )?;
        Ok(stats)
    }
}

struct DispatchShape {
    group_count: usize,
    invocations: usize,
    global_size: [u32; 3],
}

fn uses_workgroup_executor(program: &Program) -> bool {
    program.instructions().iter().any(|op| {
        matches!(
            op,
            Instruction::AtomicAdd { .. }
                | Instruction::AtomicExchange { .. }
                | Instruction::AtomicCompareExchange { .. }
                | Instruction::SharedLoad { .. }
                | Instruction::SharedStore { .. }
                | Instruction::WorkgroupBarrier
        )
    })
}

fn uses_atomic_operations(program: &Program) -> bool {
    program.instructions().iter().any(|op| {
        matches!(
            op,
            Instruction::AtomicAdd { .. }
                | Instruction::AtomicExchange { .. }
                | Instruction::AtomicCompareExchange { .. }
        )
    })
}

fn validate_atomic_bindings(program: &Program, count: usize) -> Result<()> {
    if count > MAX_ATOMIC_BUFFERS {
        return Err("compute dispatch accepts at most 12 atomic buffers".into());
    }
    if let Some(buffer) = program.instructions().iter().find_map(|op| match op {
        Instruction::AtomicAdd { buffer, .. }
        | Instruction::AtomicExchange { buffer, .. }
        | Instruction::AtomicCompareExchange { buffer, .. }
            if *buffer as usize >= count =>
        {
            Some(*buffer)
        }
        _ => None,
    }) {
        return Err(format!("compute program uses unbound atomic buffer {buffer}").into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn dispatch_workgroups_scalar(
    pipeline: &ComputePipeline,
    workgroups: [u32; 3],
    inputs: &[&StorageBuffer],
    input_layouts: &[StorageLayout],
    atomic_buffers: &mut [&mut StorageBuffer],
    output: &mut StorageBuffer,
    output_layout: StorageLayout,
    shape: DispatchShape,
) -> Result<ComputeStats> {
    let input_count = 4 + inputs.len();
    let local_count = pipeline.local_size.iter().map(|&v| v as usize).product();
    let mut results = vec![Vec4::ZERO; shape.invocations];
    let mut stores = StagedWrites::new(output.len());
    let mut stats = ComputeStats {
        workgroups: shape.group_count as u64,
        invocations: shape.invocations as u64,
        ..ComputeStats::default()
    };
    let atomic_storage: Vec<_> = if uses_atomic_operations(&pipeline.program) {
        atomic_buffers
            .iter()
            .map(|buffer| AtomicStorage::new(buffer))
            .collect()
    } else {
        Vec::new()
    };
    let [size_x, size_y, _] = shape.global_size;

    for wz in 0..workgroups[2] {
        for wy in 0..workgroups[1] {
            for wx in 0..workgroups[0] {
                let [local_x, local_y, local_z] = pipeline.local_size;
                let mut local_inputs = Vec::with_capacity(local_count);
                let mut linears = Vec::with_capacity(local_count);
                for lz in 0..local_z {
                    for ly in 0..local_y {
                        for lx in 0..local_x {
                            let global = [wx * local_x + lx, wy * local_y + ly, wz * local_z + lz];
                            let linear = global[0] as usize
                                + size_x as usize
                                    * (global[1] as usize + size_y as usize * global[2] as usize);
                            let mut shader_inputs = [Vec4::ZERO; 16];
                            shader_inputs[0] = id(global);
                            shader_inputs[1] = id([lx, ly, lz]);
                            shader_inputs[2] = id([wx, wy, wz]);
                            shader_inputs[3] = id(workgroups);
                            for (slot, (buffer, layout)) in
                                inputs.iter().zip(input_layouts).enumerate()
                            {
                                shader_inputs[4 + slot] = buffer.values[layout.index(linear)];
                            }
                            local_inputs.push(shader_inputs);
                            linears.push(linear);
                        }
                    }
                }
                let input_views: Vec<_> = local_inputs
                    .iter()
                    .map(|values| &values[..input_count])
                    .collect();
                let mut shared = vec![Vec4::ZERO; pipeline.shared_memory_vec4s];
                let executions = pipeline.program.execute_workgroup(
                    &input_views,
                    &mut shared,
                    |local, buffer, index| {
                        inputs
                            .get(buffer)
                            .and_then(|buffer| buffer.values.get(index))
                            .copied()
                            .ok_or_else(|| {
                                format!(
                                    "local invocation {local}: storage load from input buffer {buffer} at vec4 {index} is out of bounds"
                                )
                            })
                    },
                    |local, index, value| {
                        stores
                            .stage(index, value)
                            .map_err(|error| format!("local invocation {local}: {error}"))
                    },
                    |local, buffer, index, operation| {
                        atomic_storage
                            .get(buffer)
                            .ok_or_else(|| format!("local invocation {local}: unbound atomic buffer {buffer}"))?
                            .apply(index, operation)
                            .map_err(|error| format!("local invocation {local}: {error}"))
                    },
                )?;
                for (linear, execution) in linears.into_iter().zip(executions) {
                    results[linear] = execution.outputs[0];
                    stats.instructions += execution.instructions as u64;
                }
            }
        }
    }
    commit_results(
        output,
        output_layout,
        &results,
        stores,
        !pipeline.storage_only,
    )?;
    for (buffer, atomic) in atomic_buffers.iter_mut().zip(&atomic_storage) {
        atomic.commit_into(buffer);
    }
    Ok(stats)
}

fn packed_layouts(count: usize) -> Result<[StorageLayout; MAX_INPUT_BUFFERS]> {
    if count > MAX_INPUT_BUFFERS {
        return Err("compute dispatch accepts at most 12 input buffers".into());
    }
    Ok([StorageLayout::PACKED; MAX_INPUT_BUFFERS])
}

fn dispatch_shape(
    pipeline: &ComputePipeline,
    workgroups: [u32; 3],
    inputs: &[&StorageBuffer],
    input_layouts: &[StorageLayout],
    output: &StorageBuffer,
    output_layout: StorageLayout,
) -> Result<Option<DispatchShape>> {
    if let Some(count) = pipeline.storage_input_count
        && count != inputs.len()
    {
        return Err(format!(
            "compute SPIR-V requires exactly {} input storage buffers",
            count
        )
        .into());
    }
    if inputs.len() > MAX_INPUT_BUFFERS {
        return Err("compute dispatch accepts at most 12 input buffers".into());
    }
    if input_layouts.len() != inputs.len() {
        return Err("compute dispatch requires one layout per input buffer".into());
    }
    let input_count = 4 + inputs.len();
    if pipeline
        .program
        .instructions()
        .iter()
        .any(|op| matches!(op, Instruction::Input { slot, .. } if *slot as usize >= input_count))
    {
        return Err("compute program reads an unbound input slot".into());
    }
    if pipeline
        .program
        .instructions()
        .iter()
        .any(|op| matches!(op, Instruction::StorageLoad { buffer, .. } if *buffer as usize >= inputs.len()))
    {
        return Err("compute program reads an unbound storage buffer".into());
    }
    let group_count = workgroups
        .into_iter()
        .try_fold(1usize, |n, axis| n.checked_mul(axis as usize))
        .ok_or("compute workgroup count overflows")?;
    if group_count == 0 {
        return Ok(None);
    }
    let local_count = pipeline
        .local_size
        .into_iter()
        .map(|axis| axis as usize)
        .product::<usize>();
    let invocations = group_count
        .checked_mul(local_count)
        .filter(|&count| count <= MAX_DISPATCH_INVOCATIONS)
        .ok_or("compute dispatch exceeds 1048576 invocations")?;
    let global_size = std::array::from_fn(|i| workgroups[i] * pipeline.local_size[i]);
    if !pipeline.storage_only {
        for (slot, (buffer, layout)) in inputs.iter().zip(input_layouts).enumerate() {
            validate_storage_range(layout, invocations, buffer.len())
                .map_err(|reason| format!("compute input buffer {slot}: {reason}"))?;
        }
        validate_storage_range(&output_layout, invocations, output.len())
            .map_err(|reason| format!("compute output buffer: {reason}"))?;
    }
    Ok(Some(DispatchShape {
        group_count,
        invocations,
        global_size,
    }))
}

fn validate_storage_range(layout: &StorageLayout, count: usize, len: usize) -> Result<usize> {
    let last = layout
        .stride
        .checked_mul(count - 1)
        .and_then(|step| layout.offset.checked_add(step))
        .ok_or("storage layout address overflows")?;
    if last >= len {
        return Err(format!(
            "layout offset {} stride {} addresses vec4 {last}, buffer length is {len}",
            layout.offset, layout.stride
        )
        .into());
    }
    Ok(last)
}

struct StagedWrites {
    output_len: usize,
    values: Vec<(usize, Vec4)>,
}

struct AtomicStorage {
    values: Vec<AtomicU32>,
}

impl AtomicStorage {
    fn new(buffer: &StorageBuffer) -> Self {
        Self {
            values: buffer
                .values
                .iter()
                .map(|value| AtomicU32::new(value.x.to_bits()))
                .collect(),
        }
    }

    fn apply(&self, index: usize, operation: AtomicOperation) -> std::result::Result<Vec4, String> {
        let atomic = self.values.get(index).ok_or_else(|| {
            format!(
                "atomic access to vec4 {index} is out of bounds for buffer length {}",
                self.values.len()
            )
        })?;
        let old = match operation {
            AtomicOperation::Add(value) => {
                if !value.is_finite() {
                    return Err("atomic add value must be finite".into());
                }
                atomic
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |bits| {
                        let sum = f32::from_bits(bits) + value;
                        sum.is_finite().then_some(sum.to_bits())
                    })
                    .map_err(|_| "atomic add would produce a non-finite value")?
            }
            AtomicOperation::Exchange(value) => {
                if !value.is_finite() {
                    return Err("atomic exchange value must be finite".into());
                }
                atomic.swap(value.to_bits(), Ordering::SeqCst)
            }
            AtomicOperation::CompareExchange {
                expected,
                replacement,
            } => {
                if !expected.is_finite() || !replacement.is_finite() {
                    return Err("atomic compare-exchange values must be finite".into());
                }
                atomic
                    .compare_exchange(
                        expected.to_bits(),
                        replacement.to_bits(),
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                    .unwrap_or_else(|old| old)
            }
        };
        let old = f32::from_bits(old);
        Ok(Vec4::new(old, old, old, old))
    }

    fn commit_into(&self, buffer: &mut StorageBuffer) {
        for (value, atomic) in buffer.values.iter_mut().zip(&self.values) {
            value.x = f32::from_bits(atomic.load(Ordering::SeqCst));
        }
    }
}

impl StagedWrites {
    fn new(output_len: usize) -> Self {
        Self {
            output_len,
            values: Vec::new(),
        }
    }

    fn stage(&mut self, index: usize, value: Vec4) -> std::result::Result<(), String> {
        if index >= self.output_len {
            return Err(format!(
                "storage store to vec4 {index} is out of bounds for output length {}",
                self.output_len
            ));
        }
        if self.values.len() >= MAX_STORAGE_WRITES {
            return Err(format!(
                "compute dispatch exceeds {MAX_STORAGE_WRITES} staged storage writes"
            ));
        }
        self.values.push((index, value));
        Ok(())
    }
}

fn commit_results(
    output: &mut StorageBuffer,
    layout: StorageLayout,
    results: &[Vec4],
    mut stores: StagedWrites,
    write_map_output: bool,
) -> Result<()> {
    stores.values.sort_unstable_by_key(|(index, _)| *index);
    if let Some(pair) = stores.values.windows(2).find(|pair| pair[0].0 == pair[1].0) {
        return Err(format!(
            "compute storage writes contain duplicate destination vec4 {}",
            pair[0].0
        )
        .into());
    }
    if write_map_output {
        if layout.stride == 1 {
            let end = layout.offset + results.len();
            output.values[layout.offset..end].copy_from_slice(results);
        } else {
            for (invocation, value) in results.iter().copied().enumerate() {
                output.values[layout.index(invocation)] = value;
            }
        }
    }
    // Explicit stores commit after map writes, or alone for SPIR-V compute.
    for (index, value) in stores.values {
        output.values[index] = value;
    }
    Ok(())
}

fn invocation_ids(linear: usize, global_size: [u32; 3], local_size: [u32; 3]) -> [[u32; 3]; 3] {
    let size_x = global_size[0] as usize;
    let size_y = global_size[1] as usize;
    let global = [
        (linear % size_x) as u32,
        ((linear / size_x) % size_y) as u32,
        (linear / (size_x * size_y)) as u32,
    ];
    let local = std::array::from_fn(|i| global[i] % local_size[i]);
    let group = std::array::from_fn(|i| global[i] / local_size[i]);
    [global, local, group]
}

fn scalar_invocation(
    pipeline: &ComputePipeline,
    workgroups: [u32; 3],
    inputs: &[&StorageBuffer],
    input_layouts: &[StorageLayout],
    ids: [[u32; 3]; 3],
    linear: usize,
    stores: &mut StagedWrites,
) -> Result<(Vec4, u64)> {
    let mut shader_inputs = [Vec4::ZERO; 16];
    let input_count = 4 + inputs.len();
    let [global, local, group] = ids;
    shader_inputs[0] = id(global);
    shader_inputs[1] = id(local);
    shader_inputs[2] = id(group);
    shader_inputs[3] = id(workgroups);
    for (slot, (buffer, layout)) in inputs.iter().zip(input_layouts).enumerate() {
        shader_inputs[4 + slot] = buffer.values[layout.index(linear)];
    }
    let execution = pipeline.program.execute_with_lod_and_storage(
        &shader_inputs[..input_count],
        &[],
        &[],
        |_, _| Err("compute programs do not sample textures".into()),
        |buffer, index| {
            inputs
                .get(buffer)
                .and_then(|buffer| buffer.values.get(index))
                .copied()
                .ok_or_else(|| {
                    format!(
                        "storage load from input buffer {buffer} at vec4 {index} is out of bounds"
                    )
                })
        },
        |index, value| stores.stage(index, value),
        false,
    )?;
    Ok((execution.outputs[0], execution.instructions as u64))
}

fn id(value: [u32; 3]) -> Vec4 {
    Vec4::new(value[0] as f32, value[1] as f32, value[2] as f32, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_program() -> Program {
        use Instruction::*;
        Program::new(vec![
            Input { dst: 0, slot: 0 },
            Input { dst: 1, slot: 4 },
            Input { dst: 2, slot: 5 },
            Add { dst: 3, a: 1, b: 2 },
            Output { slot: 0, src: 3 },
        ])
        .unwrap()
    }

    #[test]
    fn dispatches_glsl_spirv_vec4_storage_add() {
        let device = Device::new();
        let pipeline = device
            .create_compute_pipeline_from_spirv(include_bytes!(
                "../../../assets/shaders/compute_vector_add.comp.spv"
            ))
            .unwrap();
        assert_eq!(pipeline.local_size(), [64, 1, 1]);

        let a = device
            .create_storage_buffer(
                (0..64)
                    .map(|i| Vec4::new(i as f32, 2.0, 3.0, 4.0))
                    .collect(),
            )
            .unwrap();
        let b = device
            .create_storage_buffer(
                (0..64)
                    .map(|i| Vec4::new(5.0, i as f32, 7.0, 8.0))
                    .collect(),
            )
            .unwrap();
        let mut output = device.create_storage_buffer(vec![Vec4::ZERO; 64]).unwrap();
        let mut simd_output = device.create_storage_buffer(vec![Vec4::ZERO; 64]).unwrap();
        let stats = device
            .dispatch_compute(&pipeline, [1, 1, 1], &[&a, &b], &mut output)
            .unwrap();
        let simd_stats = device
            .dispatch_compute_simd(&pipeline, [1, 1, 1], &[&a, &b], &mut simd_output)
            .unwrap();
        assert_eq!(stats.invocations, 64);
        assert_eq!(simd_stats, stats);
        assert_eq!(simd_output.as_slice(), output.as_slice());
        assert!(
            output
                .as_slice()
                .iter()
                .zip(a.as_slice().iter().zip(b.as_slice()))
                .all(|(actual, (left, right))| *actual == *left + *right)
        );
        assert!(
            device
                .dispatch_compute(&pipeline, [1, 1, 1], &[&a], &mut output)
                .unwrap_err()
                .to_string()
                .contains("requires exactly 2 input storage buffers")
        );
    }

    #[test]
    fn storage_only_commit_preserves_unwritten_elements() {
        let original = Vec4::new(9.0, 8.0, 7.0, 6.0);
        let replacement = Vec4::new(1.0, 2.0, 3.0, 4.0);
        let mut output = StorageBuffer {
            values: vec![original, original],
        };
        let mut stores = StagedWrites::new(output.len());
        stores.stage(1, replacement).unwrap();

        commit_results(
            &mut output,
            StorageLayout::PACKED,
            &[Vec4::ZERO; 2],
            stores,
            false,
        )
        .unwrap();
        assert_eq!(output.as_slice(), &[original, replacement]);
    }

    #[test]
    fn dispatches_glsl_spirv_vec4_inversion() {
        let device = Device::new();
        let pipeline = device
            .create_compute_pipeline_from_spirv(include_bytes!(
                "../../../assets/shaders/compute_invert.comp.spv"
            ))
            .unwrap();
        let input = device
            .create_storage_buffer(
                (0..64)
                    .map(|i| {
                        let value = i as f32 / 64.0;
                        Vec4::new(value, value * 0.5, 1.0 - value, 1.0)
                    })
                    .collect(),
            )
            .unwrap();
        let mut output = device.create_storage_buffer(vec![Vec4::ZERO; 64]).unwrap();

        let stats = device
            .dispatch_compute(&pipeline, [1, 1, 1], &[&input], &mut output)
            .unwrap();
        assert_eq!(stats.invocations, 64);
        assert!(
            output
                .as_slice()
                .iter()
                .zip(input.as_slice())
                .all(|(actual, source)| *actual == Vec4::new(1.0, 1.0, 1.0, 1.0) - *source)
        );
    }

    #[test]
    fn dispatch_maps_vec4_inputs_over_3d_workgroups() {
        let device = Device::new();
        let pipeline = device
            .create_compute_pipeline(add_program(), [2, 2, 1])
            .unwrap();
        let a = device
            .create_storage_buffer(
                (0..32)
                    .map(|i| Vec4::new(i as f32, 1.0, 2.0, 3.0))
                    .collect(),
            )
            .unwrap();
        let b = device
            .create_storage_buffer(
                (0..32)
                    .map(|i| Vec4::new(10.0, i as f32, 4.0, 5.0))
                    .collect(),
            )
            .unwrap();
        let mut output = device.create_storage_buffer(vec![Vec4::ZERO; 32]).unwrap();
        let mut simd_output = device.create_storage_buffer(vec![Vec4::ZERO; 32]).unwrap();

        let stats = device
            .dispatch_compute(&pipeline, [2, 2, 2], &[&a, &b], &mut output)
            .unwrap();
        let simd_stats = device
            .dispatch_compute_simd(&pipeline, [2, 2, 2], &[&a, &b], &mut simd_output)
            .unwrap();
        assert_eq!(stats.workgroups, 8);
        assert_eq!(stats.invocations, 32);
        assert_eq!(stats.instructions, 160);
        assert_eq!(simd_stats, stats);
        assert_eq!(simd_output.as_slice(), output.as_slice());
        for i in 0..32 {
            assert_eq!(output.as_slice()[i], a.as_slice()[i] + b.as_slice()[i]);
        }
    }

    #[test]
    fn shader_selected_storage_reads_and_writes_match_scalar_and_simd() {
        use Instruction::*;
        let device = Device::new();
        let program = Program::new(vec![
            Input { dst: 0, slot: 0 },
            Const {
                dst: 1,
                value: Vec4::new(4.0, 0.0, 0.0, 0.0),
            },
            Sub { dst: 2, a: 1, b: 0 },
            StorageLoad {
                dst: 3,
                buffer: 0,
                index: 2,
            },
            Output { slot: 0, src: 3 },
            Const {
                dst: 4,
                value: Vec4::new(5.0, 0.0, 0.0, 0.0),
            },
            Add { dst: 5, a: 0, b: 4 },
            StorageStore { index: 5, src: 3 },
        ])
        .unwrap();
        let pipeline = device.create_compute_pipeline(program, [1, 1, 1]).unwrap();
        let input = device
            .create_storage_buffer(
                (0..5)
                    .map(|i| Vec4::new(i as f32, i as f32, i as f32, i as f32))
                    .collect(),
            )
            .unwrap();
        let mut scalar = device.create_storage_buffer(vec![Vec4::ZERO; 10]).unwrap();
        let mut simd = device.create_storage_buffer(vec![Vec4::ZERO; 10]).unwrap();

        device
            .dispatch_compute(&pipeline, [5, 1, 1], &[&input], &mut scalar)
            .unwrap();
        device
            .dispatch_compute_simd(&pipeline, [5, 1, 1], &[&input], &mut simd)
            .unwrap();

        let expected: Vec<_> = (0..5)
            .rev()
            .map(|i| Vec4::new(i as f32, i as f32, i as f32, i as f32))
            .collect();
        assert_eq!(scalar.as_slice(), simd.as_slice());
        assert_eq!(&scalar.as_slice()[..5], expected);
        assert_eq!(&scalar.as_slice()[5..], expected);
    }

    #[test]
    fn shader_selected_storage_errors_are_bounded_and_atomic() {
        use Instruction::*;
        let device = Device::new();
        let sentinel = Vec4::new(-7.0, -7.0, -7.0, -7.0);
        let out_of_bounds_load = Program::new(vec![
            Input { dst: 0, slot: 0 },
            StorageLoad {
                dst: 1,
                buffer: 0,
                index: 0,
            },
            Output { slot: 0, src: 1 },
        ])
        .unwrap();
        let pipeline = device
            .create_compute_pipeline(out_of_bounds_load, [1, 1, 1])
            .unwrap();
        let input = device.create_storage_buffer(vec![Vec4::ZERO; 3]).unwrap();
        let mut scalar = device.create_storage_buffer(vec![sentinel; 4]).unwrap();
        let mut simd = device.create_storage_buffer(vec![sentinel; 4]).unwrap();
        assert!(
            device
                .dispatch_compute(&pipeline, [1, 1, 1], &[], &mut scalar)
                .is_err()
        );
        assert!(
            device
                .dispatch_compute(&pipeline, [4, 1, 1], &[&input], &mut scalar)
                .is_err()
        );
        assert!(
            device
                .dispatch_compute_simd(&pipeline, [4, 1, 1], &[&input], &mut simd)
                .is_err()
        );
        assert_eq!(scalar.as_slice(), &[sentinel; 4]);
        assert_eq!(simd.as_slice(), &[sentinel; 4]);

        let invalid_index = Program::new(vec![
            Const {
                dst: 0,
                value: Vec4::new(1.5, 0.0, 0.0, 0.0),
            },
            StorageLoad {
                dst: 1,
                buffer: 0,
                index: 0,
            },
            Output { slot: 0, src: 1 },
        ])
        .unwrap();
        let pipeline = device
            .create_compute_pipeline(invalid_index, [1, 1, 1])
            .unwrap();
        let one_value = device.create_storage_buffer(vec![Vec4::ZERO]).unwrap();
        assert!(
            device
                .dispatch_compute(&pipeline, [4, 1, 1], &[&one_value], &mut scalar)
                .is_err()
        );
        assert!(
            device
                .dispatch_compute_simd(&pipeline, [4, 1, 1], &[&one_value], &mut simd)
                .is_err()
        );
        assert_eq!(scalar.as_slice(), &[sentinel; 4]);
        assert_eq!(simd.as_slice(), &[sentinel; 4]);

        let out_of_bounds_store = Program::new(vec![
            Input { dst: 0, slot: 0 },
            Const {
                dst: 1,
                value: Vec4::new(4.0, 0.0, 0.0, 0.0),
            },
            StorageStore { index: 1, src: 0 },
            Output { slot: 0, src: 0 },
        ])
        .unwrap();
        let pipeline = device
            .create_compute_pipeline(out_of_bounds_store, [1, 1, 1])
            .unwrap();
        assert!(
            device
                .dispatch_compute(&pipeline, [4, 1, 1], &[], &mut scalar)
                .is_err()
        );
        assert!(
            device
                .dispatch_compute_simd(&pipeline, [4, 1, 1], &[], &mut simd)
                .is_err()
        );
        assert_eq!(scalar.as_slice(), &[sentinel; 4]);
        assert_eq!(simd.as_slice(), &[sentinel; 4]);

        let duplicate_store = Program::new(vec![
            Const {
                dst: 0,
                value: Vec4::ZERO,
            },
            Input { dst: 1, slot: 0 },
            StorageStore { index: 0, src: 1 },
            Output { slot: 0, src: 1 },
        ])
        .unwrap();
        let pipeline = device
            .create_compute_pipeline(duplicate_store, [1, 1, 1])
            .unwrap();
        let mut output = device.create_storage_buffer(vec![sentinel; 4]).unwrap();
        let mut scalar_output = device.create_storage_buffer(vec![sentinel; 4]).unwrap();
        assert!(
            device
                .dispatch_compute(&pipeline, [4, 1, 1], &[], &mut scalar_output)
                .is_err()
        );
        assert!(
            device
                .dispatch_compute_simd(&pipeline, [4, 1, 1], &[], &mut output)
                .is_err()
        );
        assert_eq!(scalar_output.as_slice(), &[sentinel; 4]);
        assert_eq!(output.as_slice(), &[sentinel; 4]);
    }

    #[test]
    fn shared_memory_barriers_exchange_values_within_each_workgroup() {
        use Instruction::*;
        let device = Device::new();
        let program = Program::new(vec![
            Input { dst: 0, slot: 0 },
            StorageLoad {
                dst: 1,
                buffer: 0,
                index: 0,
            },
            Input { dst: 2, slot: 1 },
            SharedStore { index: 2, src: 1 },
            WorkgroupBarrier,
            Const {
                dst: 3,
                value: Vec4::new(3.0, 0.0, 0.0, 0.0),
            },
            Sub { dst: 4, a: 3, b: 2 },
            SharedLoad { dst: 5, index: 4 },
            Output { slot: 0, src: 5 },
        ])
        .unwrap();
        let pipeline = device
            .create_compute_pipeline_with_shared_memory(program, [4, 1, 1], 4)
            .unwrap();
        let input = device
            .create_storage_buffer(
                (1..=8)
                    .map(|i| Vec4::new(i as f32, i as f32, i as f32, i as f32))
                    .collect(),
            )
            .unwrap();
        let mut scalar = device.create_storage_buffer(vec![Vec4::ZERO; 8]).unwrap();
        let mut simd = device.create_storage_buffer(vec![Vec4::ZERO; 8]).unwrap();
        let scalar_stats = device
            .dispatch_compute(&pipeline, [2, 1, 1], &[&input], &mut scalar)
            .unwrap();
        let simd_stats = device
            .dispatch_compute_simd(&pipeline, [2, 1, 1], &[&input], &mut simd)
            .unwrap();

        assert_eq!(pipeline.shared_memory_vec4s(), 4);
        assert_eq!(scalar_stats, simd_stats);
        assert_eq!(scalar.as_slice(), simd.as_slice());
        assert_eq!(
            scalar.as_slice(),
            &[
                Vec4::new(4.0, 4.0, 4.0, 4.0),
                Vec4::new(3.0, 3.0, 3.0, 3.0),
                Vec4::new(2.0, 2.0, 2.0, 2.0),
                Vec4::new(1.0, 1.0, 1.0, 1.0),
                Vec4::new(8.0, 8.0, 8.0, 8.0),
                Vec4::new(7.0, 7.0, 7.0, 7.0),
                Vec4::new(6.0, 6.0, 6.0, 6.0),
                Vec4::new(5.0, 5.0, 5.0, 5.0),
            ]
        );
    }

    #[test]
    fn workgroup_barrier_divergence_and_shared_races_fail_atomically() {
        use Instruction::*;
        let device = Device::new();
        let sentinel = Vec4::new(-9.0, -9.0, -9.0, -9.0);
        let divergent = Program::new(vec![
            Input { dst: 0, slot: 0 },
            If { condition: 0 },
            WorkgroupBarrier,
            Else,
            EndIf,
            Output { slot: 0, src: 0 },
        ])
        .unwrap();
        let pipeline = device
            .create_compute_pipeline(divergent, [4, 1, 1])
            .unwrap();
        let mut output = device.create_storage_buffer(vec![sentinel; 4]).unwrap();
        let error = device
            .dispatch_compute(&pipeline, [1, 1, 1], &[], &mut output)
            .unwrap_err()
            .to_string();
        assert!(error.contains("barrier divergence"));
        assert_eq!(output.as_slice(), &[sentinel; 4]);

        let conflicting_writes = Program::new(vec![
            Input { dst: 0, slot: 1 },
            Const {
                dst: 1,
                value: Vec4::ZERO,
            },
            SharedStore { index: 1, src: 0 },
            Output { slot: 0, src: 0 },
        ])
        .unwrap();
        assert!(
            device
                .create_compute_pipeline(conflicting_writes.clone(), [4, 1, 1])
                .is_err()
        );
        let pipeline = device
            .create_compute_pipeline_with_shared_memory(conflicting_writes, [4, 1, 1], 1)
            .unwrap();
        let error = device
            .dispatch_compute_simd(&pipeline, [1, 1, 1], &[], &mut output)
            .unwrap_err()
            .to_string();
        assert!(error.contains("shared-memory race"));
        assert_eq!(output.as_slice(), &[sentinel; 4]);

        let read_then_write = Program::new(vec![
            Input { dst: 0, slot: 1 },
            Const {
                dst: 1,
                value: Vec4::ZERO,
            },
            SharedLoad { dst: 2, index: 1 },
            SharedStore { index: 1, src: 0 },
            Output { slot: 0, src: 2 },
        ])
        .unwrap();
        let pipeline = device
            .create_compute_pipeline_with_shared_memory(read_then_write, [4, 1, 1], 1)
            .unwrap();
        let error = device
            .dispatch_compute(&pipeline, [1, 1, 1], &[], &mut output)
            .unwrap_err()
            .to_string();
        assert!(error.contains("shared-memory race"));
        assert_eq!(output.as_slice(), &[sentinel; 4]);

        let out_of_bounds = Program::new(vec![
            Input { dst: 0, slot: 1 },
            Const {
                dst: 1,
                value: Vec4::new(1.0, 0.0, 0.0, 0.0),
            },
            SharedStore { index: 1, src: 0 },
            Output { slot: 0, src: 0 },
        ])
        .unwrap();
        let pipeline = device
            .create_compute_pipeline_with_shared_memory(out_of_bounds, [4, 1, 1], 1)
            .unwrap();
        let error = device
            .dispatch_compute(&pipeline, [1, 1, 1], &[], &mut output)
            .unwrap_err()
            .to_string();
        assert!(error.contains("shared-memory vec4 1 is out of bounds"));
        assert_eq!(output.as_slice(), &[sentinel; 4]);
    }

    #[test]
    fn atomic_add_is_linearizable_across_workgroups() {
        use Instruction::*;
        let device = Device::new();
        let program = Program::new(vec![
            Const {
                dst: 0,
                value: Vec4::ZERO,
            },
            Const {
                dst: 1,
                value: Vec4::new(1.0, 0.0, 0.0, 0.0),
            },
            AtomicAdd {
                dst: 2,
                buffer: 0,
                index: 0,
                value: 1,
            },
            Output { slot: 0, src: 2 },
        ])
        .unwrap();
        let pipeline = device.create_compute_pipeline(program, [4, 1, 1]).unwrap();
        let initial = Vec4::new(10.0, 2.0, 3.0, 4.0);
        let mut scalar_atomic = device.create_storage_buffer(vec![initial]).unwrap();
        let mut simd_atomic = device.create_storage_buffer(vec![initial]).unwrap();
        let mut scalar_output = device.create_storage_buffer(vec![Vec4::ZERO; 8]).unwrap();
        let mut simd_output = device.create_storage_buffer(vec![Vec4::ZERO; 8]).unwrap();

        let mut scalar_bindings = [&mut scalar_atomic];
        let scalar_stats = device
            .dispatch_compute_with_atomics(
                &pipeline,
                [2, 1, 1],
                &[],
                &mut scalar_bindings,
                &mut scalar_output,
            )
            .unwrap();
        let mut simd_bindings = [&mut simd_atomic];
        let simd_stats = device
            .dispatch_compute_simd_with_atomics(
                &pipeline,
                [2, 1, 1],
                &[],
                &mut simd_bindings,
                &mut simd_output,
            )
            .unwrap();

        assert_eq!(scalar_stats, simd_stats);
        assert_eq!(scalar_output.as_slice(), simd_output.as_slice());
        assert_eq!(
            scalar_output
                .as_slice()
                .iter()
                .map(|value| value.x)
                .collect::<Vec<_>>(),
            (10..18).map(|value| value as f32).collect::<Vec<_>>()
        );
        assert_eq!(scalar_atomic.as_slice()[0], Vec4::new(18.0, 2.0, 3.0, 4.0));
        assert_eq!(simd_atomic.as_slice(), scalar_atomic.as_slice());
    }

    #[test]
    fn atomic_exchange_and_compare_exchange_return_previous_values() {
        use Instruction::*;
        let device = Device::new();
        let program = Program::new(vec![
            Const {
                dst: 0,
                value: Vec4::ZERO,
            },
            Const {
                dst: 1,
                value: Vec4::new(5.0, 0.0, 0.0, 0.0),
            },
            AtomicExchange {
                dst: 2,
                buffer: 0,
                index: 0,
                value: 1,
            },
            Const {
                dst: 3,
                value: Vec4::ZERO,
            },
            StorageStore { index: 3, src: 2 },
            Const {
                dst: 4,
                value: Vec4::new(5.0, 0.0, 0.0, 0.0),
            },
            Const {
                dst: 5,
                value: Vec4::new(7.0, 0.0, 0.0, 0.0),
            },
            AtomicCompareExchange {
                dst: 6,
                buffer: 0,
                index: 0,
                expected: 4,
                replacement: 5,
            },
            Const {
                dst: 7,
                value: Vec4::new(1.0, 0.0, 0.0, 0.0),
            },
            StorageStore { index: 7, src: 6 },
            Const {
                dst: 8,
                value: Vec4::new(5.0, 0.0, 0.0, 0.0),
            },
            Const {
                dst: 9,
                value: Vec4::new(9.0, 0.0, 0.0, 0.0),
            },
            AtomicCompareExchange {
                dst: 10,
                buffer: 0,
                index: 0,
                expected: 8,
                replacement: 9,
            },
            Const {
                dst: 11,
                value: Vec4::new(2.0, 0.0, 0.0, 0.0),
            },
            StorageStore { index: 11, src: 10 },
            Output { slot: 0, src: 2 },
        ])
        .unwrap();
        let pipeline = device.create_compute_pipeline(program, [1, 1, 1]).unwrap();
        let mut atomics = device
            .create_storage_buffer(vec![Vec4::new(3.0, 4.0, 5.0, 6.0)])
            .unwrap();
        let mut output = device.create_storage_buffer(vec![Vec4::ZERO; 3]).unwrap();
        let mut bindings = [&mut atomics];
        device
            .dispatch_compute_with_atomics(&pipeline, [1, 1, 1], &[], &mut bindings, &mut output)
            .unwrap();

        assert_eq!(
            output.as_slice(),
            &[
                Vec4::new(3.0, 3.0, 3.0, 3.0),
                Vec4::new(5.0, 5.0, 5.0, 5.0),
                Vec4::new(7.0, 7.0, 7.0, 7.0),
            ]
        );
        assert_eq!(atomics.as_slice()[0], Vec4::new(7.0, 4.0, 5.0, 6.0));
    }

    #[test]
    fn atomic_dispatch_errors_do_not_mutate_bound_buffers() {
        use Instruction::*;
        let device = Device::new();
        let sentinel = Vec4::new(-9.0, -9.0, -9.0, -9.0);
        let pipeline = device
            .create_compute_pipeline(
                Program::new(vec![
                    Const {
                        dst: 0,
                        value: Vec4::ZERO,
                    },
                    Const {
                        dst: 1,
                        value: Vec4::new(1.0, 0.0, 0.0, 0.0),
                    },
                    AtomicAdd {
                        dst: 2,
                        buffer: 0,
                        index: 0,
                        value: 1,
                    },
                    Const {
                        dst: 3,
                        value: Vec4::new(1.0, 0.0, 0.0, 0.0),
                    },
                    StorageStore { index: 3, src: 2 },
                    Output { slot: 0, src: 2 },
                ])
                .unwrap(),
                [1, 1, 1],
            )
            .unwrap();
        let mut atomics = device
            .create_storage_buffer(vec![Vec4::new(10.0, 2.0, 3.0, 4.0)])
            .unwrap();
        let mut output = device.create_storage_buffer(vec![sentinel]).unwrap();
        let error = device
            .dispatch_compute_with_atomics(&pipeline, [1, 1, 1], &[], &mut [], &mut output)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unbound atomic buffer 0"));
        assert_eq!(output.as_slice(), &[sentinel]);

        let mut bindings = [&mut atomics];
        let error = device
            .dispatch_compute_with_atomics(&pipeline, [1, 1, 1], &[], &mut bindings, &mut output)
            .unwrap_err()
            .to_string();
        assert!(error.contains("storage store to vec4 1 is out of bounds"));
        assert_eq!(atomics.as_slice()[0], Vec4::new(10.0, 2.0, 3.0, 4.0));
        assert_eq!(output.as_slice(), &[sentinel]);

        let out_of_bounds = device
            .create_compute_pipeline(
                Program::new(vec![
                    Const {
                        dst: 0,
                        value: Vec4::new(1.0, 0.0, 0.0, 0.0),
                    },
                    Const {
                        dst: 1,
                        value: Vec4::new(2.0, 0.0, 0.0, 0.0),
                    },
                    AtomicExchange {
                        dst: 2,
                        buffer: 0,
                        index: 0,
                        value: 1,
                    },
                    Output { slot: 0, src: 2 },
                ])
                .unwrap(),
                [1, 1, 1],
            )
            .unwrap();
        let mut bindings = [&mut atomics];
        let error = device
            .dispatch_compute_with_atomics(
                &out_of_bounds,
                [1, 1, 1],
                &[],
                &mut bindings,
                &mut output,
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("atomic access to vec4 1 is out of bounds"));
        assert_eq!(atomics.as_slice()[0], Vec4::new(10.0, 2.0, 3.0, 4.0));
        assert_eq!(output.as_slice(), &[sentinel]);
    }

    #[test]
    fn simd_dispatch_handles_a_short_final_packet() {
        use Instruction::*;
        let device = Device::new();
        let program =
            Program::new(vec![Input { dst: 0, slot: 0 }, Output { slot: 0, src: 0 }]).unwrap();
        let pipeline = device.create_compute_pipeline(program, [1, 1, 1]).unwrap();
        let mut output = device.create_storage_buffer(vec![Vec4::ZERO; 5]).unwrap();
        let stats = device
            .dispatch_compute_simd(&pipeline, [5, 1, 1], &[], &mut output)
            .unwrap();
        assert_eq!(stats.invocations, 5);
        assert_eq!(stats.instructions, 10);
        assert_eq!(
            output.as_slice(),
            &[
                id([0, 0, 0]),
                id([1, 0, 0]),
                id([2, 0, 0]),
                id([3, 0, 0]),
                id([4, 0, 0]),
            ]
        );
    }

    #[test]
    fn dispatch_reads_and_writes_strided_struct_fields() {
        let device = Device::new();
        let input_values: Vec<_> = (0..5)
            .flat_map(|i| {
                [
                    Vec4::new(i as f32, 1.0, 2.0, 3.0),
                    Vec4::new(10.0, i as f32, 4.0, 5.0),
                    Vec4::new(-9.0, -9.0, -9.0, -9.0),
                ]
            })
            .collect();
        let input = device.create_storage_buffer(input_values).unwrap();
        let layouts = [
            StorageLayout::new(0, 3).unwrap(),
            StorageLayout::new(1, 3).unwrap(),
        ];
        let inputs = [&input, &input];
        let pipeline = device
            .create_compute_pipeline(add_program(), [1, 1, 1])
            .unwrap();
        let sentinel = Vec4::new(-7.0, -7.0, -7.0, -7.0);
        let mut scalar = device.create_storage_buffer(vec![sentinel; 15]).unwrap();
        let mut simd = device.create_storage_buffer(vec![sentinel; 15]).unwrap();
        let output_layout = StorageLayout::new(1, 3).unwrap();

        let scalar_stats = device
            .dispatch_compute_with_layouts(
                &pipeline,
                [5, 1, 1],
                &inputs,
                &layouts,
                &mut scalar,
                output_layout,
            )
            .unwrap();
        let simd_stats = device
            .dispatch_compute_simd_with_layouts(
                &pipeline,
                [5, 1, 1],
                &inputs,
                &layouts,
                &mut simd,
                output_layout,
            )
            .unwrap();
        assert_eq!(scalar_stats, simd_stats);
        assert_eq!(scalar.as_slice(), simd.as_slice());
        for i in 0..5 {
            assert_eq!(scalar.as_slice()[i * 3], sentinel);
            assert_eq!(
                scalar.as_slice()[i * 3 + 1],
                Vec4::new(i as f32 + 10.0, i as f32 + 1.0, 6.0, 8.0)
            );
            assert_eq!(scalar.as_slice()[i * 3 + 2], sentinel);
        }
    }

    #[test]
    fn strided_dispatch_checks_ranges_and_keeps_output_unchanged_on_error() {
        use Instruction::*;
        let device = Device::new();
        assert!(StorageLayout::new(0, 0).is_err());
        let pipeline = device
            .create_compute_pipeline(add_program(), [1, 1, 1])
            .unwrap();
        let input = device
            .create_storage_buffer(vec![Vec4::new(1.0, 2.0, 3.0, 4.0); 8])
            .unwrap();
        let inputs = [&input, &input];
        let sentinel = Vec4::new(7.0, 7.0, 7.0, 7.0);
        let mut output = device.create_storage_buffer(vec![sentinel; 10]).unwrap();
        let layouts = [StorageLayout::new(0, 2).unwrap(); 2];
        let output_layout = StorageLayout::new(0, 2).unwrap();
        assert!(
            device
                .dispatch_compute_with_layouts(
                    &pipeline,
                    [5, 1, 1],
                    &inputs,
                    &layouts,
                    &mut output,
                    output_layout,
                )
                .is_err()
        );
        assert_eq!(output.as_slice(), &[sentinel; 10]);
        let overflow = Program::new(vec![
            Const {
                dst: 0,
                value: Vec4::new(f32::MAX, 1.0, 1.0, 1.0),
            },
            Const {
                dst: 1,
                value: Vec4::new(2.0, 1.0, 1.0, 1.0),
            },
            Mul { dst: 2, a: 0, b: 1 },
            Output { slot: 0, src: 2 },
        ])
        .unwrap();
        let overflow = device.create_compute_pipeline(overflow, [1, 1, 1]).unwrap();
        let mut strided_output = device.create_storage_buffer(vec![sentinel; 15]).unwrap();
        let output_layout = StorageLayout::new(1, 3).unwrap();
        assert!(
            device
                .dispatch_compute_simd_with_layouts(
                    &overflow,
                    [5, 1, 1],
                    &[],
                    &[],
                    &mut strided_output,
                    output_layout,
                )
                .is_err()
        );
        assert_eq!(strided_output.as_slice(), &[sentinel; 15]);
        let overflow_layout = StorageLayout::new(1, usize::MAX).unwrap();
        assert!(
            device
                .dispatch_compute_with_layouts(
                    &overflow,
                    [5, 1, 1],
                    &[],
                    &[],
                    &mut strided_output,
                    overflow_layout,
                )
                .is_err()
        );
        assert_eq!(strided_output.as_slice(), &[sentinel; 15]);
    }

    #[test]
    fn dispatch_exposes_global_ids_for_each_3d_invocation() {
        use Instruction::*;
        let device = Device::new();
        let program =
            Program::new(vec![Input { dst: 0, slot: 0 }, Output { slot: 0, src: 0 }]).unwrap();
        let pipeline = device.create_compute_pipeline(program, [2, 2, 1]).unwrap();
        let mut output = device.create_storage_buffer(vec![Vec4::ZERO; 32]).unwrap();
        let mut simd_output = device.create_storage_buffer(vec![Vec4::ZERO; 32]).unwrap();
        device
            .dispatch_compute(&pipeline, [2, 2, 2], &[], &mut output)
            .unwrap();
        device
            .dispatch_compute_simd(&pipeline, [2, 2, 2], &[], &mut simd_output)
            .unwrap();
        assert_eq!(simd_output.as_slice(), output.as_slice());

        for z in 0..2 {
            for y in 0..4 {
                for x in 0..4 {
                    let index = x + 4 * (y + 4 * z);
                    assert_eq!(
                        output.as_slice()[index],
                        Vec4::new(x as f32, y as f32, z as f32, 0.0)
                    );
                }
            }
        }
    }

    #[test]
    fn dispatch_exposes_global_local_and_workgroup_ids() {
        use Instruction::*;
        let device = Device::new();
        let program = Program::new(vec![
            Input { dst: 0, slot: 0 },
            Input { dst: 1, slot: 1 },
            Input { dst: 2, slot: 2 },
            Input { dst: 3, slot: 3 },
            Compose {
                dst: 4,
                sources: [0, 1, 2, 3],
                lanes: [0, 0, 0, 0],
            },
            Output { slot: 0, src: 4 },
        ])
        .unwrap();
        let pipeline = device.create_compute_pipeline(program, [2, 2, 1]).unwrap();
        let mut output = device.create_storage_buffer(vec![Vec4::ZERO; 32]).unwrap();
        device
            .dispatch_compute(&pipeline, [2, 2, 2], &[], &mut output)
            .unwrap();

        for z in 0..2 {
            for y in 0..4 {
                for x in 0..4 {
                    let index = x + 4 * (y + 4 * z);
                    assert_eq!(
                        output.as_slice()[index],
                        Vec4::new(x as f32, (x % 2) as f32, (x / 2) as f32, 2.0)
                    );
                }
            }
        }
    }

    #[test]
    fn dispatch_rejects_oversized_or_unbound_work_without_writing_output() {
        let device = Device::new();
        let oversized = device.create_compute_pipeline(add_program(), [1025, 1, 1]);
        assert!(oversized.is_err());
        let pipeline = device
            .create_compute_pipeline(add_program(), [1, 1, 1])
            .unwrap();
        let input = device
            .create_storage_buffer(vec![Vec4::new(1.0, 2.0, 3.0, 4.0)])
            .unwrap();
        let mut output = device
            .create_storage_buffer(vec![Vec4::new(7.0, 7.0, 7.0, 7.0)])
            .unwrap();

        assert!(
            device
                .dispatch_compute(&pipeline, [1, 1, 1], &[&input], &mut output)
                .is_err()
        );
        assert_eq!(output.as_slice()[0], Vec4::new(7.0, 7.0, 7.0, 7.0));

        use Instruction::*;
        let overflow = Program::new(vec![
            Const {
                dst: 0,
                value: Vec4::new(f32::MAX, 1.0, 1.0, 1.0),
            },
            Const {
                dst: 1,
                value: Vec4::new(2.0, 1.0, 1.0, 1.0),
            },
            Mul { dst: 2, a: 0, b: 1 },
            Output { slot: 0, src: 2 },
        ])
        .unwrap();
        let overflow = device.create_compute_pipeline(overflow, [1, 1, 1]).unwrap();
        assert!(
            device
                .dispatch_compute(&overflow, [1, 1, 1], &[], &mut output)
                .is_err()
        );
        assert_eq!(output.as_slice()[0], Vec4::new(7.0, 7.0, 7.0, 7.0));
        assert!(
            device
                .dispatch_compute_simd(&overflow, [1, 1, 1], &[], &mut output)
                .is_err()
        );
        assert_eq!(output.as_slice()[0], Vec4::new(7.0, 7.0, 7.0, 7.0));
    }
}
