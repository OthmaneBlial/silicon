use silicon::api::{Device, Vec4};
use silicon::shader::{Instruction, Program};

fn main() -> silicon::api::Result<()> {
    const LOCAL_SIZE: usize = 4;
    const WORKGROUPS: usize = 4;
    const ELEMENTS: usize = LOCAL_SIZE * WORKGROUPS;

    let device = Device::new();
    let program = Program::new(vec![
        Instruction::Input { dst: 0, slot: 0 },
        Instruction::StorageLoad {
            dst: 1,
            buffer: 0,
            index: 0,
        },
        Instruction::Input { dst: 2, slot: 1 },
        Instruction::SharedStore { index: 2, src: 1 },
        Instruction::WorkgroupBarrier,
        Instruction::Const {
            dst: 3,
            value: Vec4::new((LOCAL_SIZE - 1) as f32, 0.0, 0.0, 0.0),
        },
        Instruction::Sub { dst: 4, a: 3, b: 2 },
        Instruction::SharedLoad { dst: 5, index: 4 },
        Instruction::Output { slot: 0, src: 5 },
    ])?;
    let pipeline = device.create_compute_pipeline_with_shared_memory(
        program,
        [LOCAL_SIZE as u32, 1, 1],
        LOCAL_SIZE,
    )?;
    let input = device.create_storage_buffer(
        (0..ELEMENTS)
            .map(|i| Vec4::new(i as f32, 0.0, 0.0, 0.0))
            .collect(),
    )?;
    let mut output = device.create_storage_buffer(vec![Vec4::ZERO; ELEMENTS])?;
    let stats =
        device.dispatch_compute(&pipeline, [WORKGROUPS as u32, 1, 1], &[&input], &mut output)?;

    for group in 0..WORKGROUPS {
        for local in 0..LOCAL_SIZE {
            let index = group * LOCAL_SIZE + local;
            assert_eq!(
                output.as_slice()[index].x,
                (group * LOCAL_SIZE + LOCAL_SIZE - 1 - local) as f32
            );
        }
    }
    println!(
        "SILICON shared compute: {} workgroups, {} invocations, {} SIR instructions; per-workgroup reversal verified",
        stats.workgroups, stats.invocations, stats.instructions
    );
    Ok(())
}
