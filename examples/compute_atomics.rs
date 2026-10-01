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
        Instruction::Const {
            dst: 2,
            value: Vec4::ZERO,
        },
        Instruction::AtomicAdd {
            dst: 3,
            buffer: 0,
            index: 2,
            value: 1,
        },
        Instruction::Output { slot: 0, src: 3 },
    ])?;
    let pipeline = device.create_compute_pipeline(program, [LOCAL_SIZE as u32, 1, 1])?;
    let input = device.create_storage_buffer(
        (1..=ELEMENTS)
            .map(|value| Vec4::new(value as f32, 0.0, 0.0, 0.0))
            .collect(),
    )?;
    let mut total = device.create_storage_buffer(vec![Vec4::ZERO])?;
    let mut output = device.create_storage_buffer(vec![Vec4::ZERO; ELEMENTS])?;
    let mut atomics = [&mut total];
    let stats = device.dispatch_compute_with_atomics(
        &pipeline,
        [WORKGROUPS as u32, 1, 1],
        &[&input],
        &mut atomics,
        &mut output,
    )?;

    assert_eq!(total.as_slice()[0].x, 136.0);
    for (invocation, result) in output.as_slice().iter().enumerate() {
        let prefix = invocation * (invocation + 1) / 2;
        assert_eq!(result.x, prefix as f32);
    }
    println!(
        "SILICON atomic compute: {} workgroups, {} invocations, {} SIR instructions; sum and atomic prefixes verified",
        stats.workgroups, stats.invocations, stats.instructions
    );
    Ok(())
}
