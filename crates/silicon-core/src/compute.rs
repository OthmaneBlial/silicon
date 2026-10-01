use crate::{Device, Result, Vec4};
use silicon_shader::{Instruction, Program};

const MAX_WORKGROUP_SIZE: usize = 1024;
const MAX_DISPATCH_INVOCATIONS: usize = 1_048_576;
const MAX_STORAGE_VECTORS: usize = 1_048_576;
const MAX_INPUT_BUFFERS: usize = 12;
const SIMT_WIDTH: usize = 4;

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
}

impl ComputePipeline {
    pub fn local_size(&self) -> [u32; 3] {
        self.local_size
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
        let invocations = local_size
            .into_iter()
            .try_fold(1usize, |n, axis| n.checked_mul(axis as usize))
            .ok_or("compute workgroup size overflows")?;
        if local_size.contains(&0) || invocations > MAX_WORKGROUP_SIZE {
            return Err("compute workgroup requires 1..1024 local invocations".into());
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
        let Some(shape) = dispatch_shape(pipeline, workgroups, inputs, output)? else {
            return Ok(ComputeStats::default());
        };

        let input_count = 4 + inputs.len();
        let mut results = vec![Vec4::ZERO; shape.invocations];
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
                                    input_count,
                                    [global, [lx, ly, lz], [wx, wy, wz]],
                                    linear,
                                )?;
                                results[linear] = value;
                                stats.instructions += instructions;
                            }
                        }
                    }
                }
            }
        }
        output.values[..shape.invocations].copy_from_slice(&results);
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
        let Some(shape) = dispatch_shape(pipeline, workgroups, inputs, output)? else {
            return Ok(ComputeStats::default());
        };
        let input_count = 4 + inputs.len();
        let mut results = vec![Vec4::ZERO; shape.invocations];
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
                    values[4 + slot] = buffer.values[linear];
                }
            }
            let input_lanes = [
                &lane_inputs[0][..input_count],
                &lane_inputs[1][..input_count],
                &lane_inputs[2][..input_count],
                &lane_inputs[3][..input_count],
            ];
            let executions = pipeline.program.execute4(
                input_lanes,
                &[],
                [&[], &[], &[], &[]],
                0b1111,
                |_, _, _| Err("compute programs do not sample textures".into()),
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
                input_count,
                [global, local, group],
                linear,
            )?;
            *result = value;
            stats.instructions += instructions;
        }
        output.values[..shape.invocations].copy_from_slice(&results);
        Ok(stats)
    }
}

struct DispatchShape {
    group_count: usize,
    invocations: usize,
    global_size: [u32; 3],
}

fn dispatch_shape(
    pipeline: &ComputePipeline,
    workgroups: [u32; 3],
    inputs: &[&StorageBuffer],
    output: &StorageBuffer,
) -> Result<Option<DispatchShape>> {
    if inputs.len() > MAX_INPUT_BUFFERS {
        return Err("compute dispatch accepts at most 12 input buffers".into());
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
    if output.len() < invocations || inputs.iter().any(|buffer| buffer.len() < invocations) {
        return Err("compute storage buffer is shorter than the dispatch".into());
    }
    Ok(Some(DispatchShape {
        group_count,
        invocations,
        global_size,
    }))
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
    input_count: usize,
    ids: [[u32; 3]; 3],
    linear: usize,
) -> Result<(Vec4, u64)> {
    let mut shader_inputs = [Vec4::ZERO; 16];
    let [global, local, group] = ids;
    shader_inputs[0] = id(global);
    shader_inputs[1] = id(local);
    shader_inputs[2] = id(group);
    shader_inputs[3] = id(workgroups);
    for (slot, buffer) in inputs.iter().enumerate() {
        shader_inputs[4 + slot] = buffer.values[linear];
    }
    let execution = pipeline.program.execute(
        &shader_inputs[..input_count],
        &[],
        |_, _| Err("compute programs do not sample textures".into()),
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
