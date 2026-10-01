use silicon::api::{Device, Vec4};
use silicon::shader::{Instruction, Program};
use std::time::Instant;

fn main() -> silicon::api::Result<()> {
    const ELEMENTS: usize = 4096;
    let device = Device::new();
    let program = Program::new(vec![
        Instruction::Input { dst: 0, slot: 4 },
        Instruction::Input { dst: 1, slot: 5 },
        Instruction::Add { dst: 2, a: 0, b: 1 },
        Instruction::Output { slot: 0, src: 2 },
    ])?;
    let pipeline = device.create_compute_pipeline(program, [64, 1, 1])?;
    let a = device.create_storage_buffer(
        (0..ELEMENTS)
            .map(|i| Vec4::new(i as f32, 2.0, 3.0, 4.0))
            .collect(),
    )?;
    let b = device.create_storage_buffer(
        (0..ELEMENTS)
            .map(|i| Vec4::new(5.0, i as f32, 7.0, 8.0))
            .collect(),
    )?;
    let mut output = device.create_storage_buffer(vec![Vec4::ZERO; ELEMENTS])?;

    let start = Instant::now();
    let stats = device.dispatch_compute(&pipeline, [64, 1, 1], &[&a, &b], &mut output)?;
    let elapsed = start.elapsed();
    for i in 0..ELEMENTS {
        assert_eq!(output.as_slice()[i], a.as_slice()[i] + b.as_slice()[i]);
    }
    println!(
        "SILICON SIR compute: {} invocations, {} workgroups, {} instructions, {:.3} ms; vector-add verified",
        stats.invocations,
        stats.workgroups,
        stats.instructions,
        elapsed.as_secs_f64() * 1000.0
    );
    Ok(())
}
