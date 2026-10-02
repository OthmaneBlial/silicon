use shader::{Comparison, Instruction::*, Program};
use silicon::{SampleCount, Vec4, shader};

fn add_branch_weights(module: &[u8]) -> Vec<u8> {
    let words: Vec<_> = module
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
        .collect();
    let mut weighted = words[..5].to_vec();
    let mut branches = 0;
    let mut pc = 5;
    while pc < words.len() {
        let header = words[pc];
        let count = (header >> 16) as usize;
        let opcode = header as u16;
        assert!(count > 0 && pc + count <= words.len());
        if opcode == 250 {
            assert!(matches!(count, 4 | 6));
            branches += 1;
        }
        let added_weights = usize::from(opcode == 250 && count == 4) * 2;
        weighted.push((((count + added_weights) as u32) << 16) | (header & 0xffff));
        weighted.extend_from_slice(&words[pc + 1..pc + count]);
        if added_weights != 0 {
            weighted.extend([1, 1]);
        }
        pc += count;
    }
    assert!(branches > 0);
    weighted.into_iter().flat_map(u32::to_le_bytes).collect()
}
fn bits(v: Vec4) -> [u32; 4] {
    v.to_array().map(f32::to_bits)
}
#[test]
fn nested_divergence_discard_and_mutable_registers_match_scalar() {
    let p = Program::new(vec![
        Input { dst: 0, slot: 0 },
        Const {
            dst: 1,
            value: Vec4::new(0.5, 0.5, 0.5, 0.5),
        },
        Compare {
            dst: 2,
            a: 0,
            b: 1,
            kind: Comparison::Less,
        },
        If { condition: 2 },
        Swizzle {
            dst: 3,
            src: 0,
            lanes: [3; 4],
        },
        Const {
            dst: 4,
            value: Vec4::ZERO,
        },
        Compare {
            dst: 3,
            a: 3,
            b: 4,
            kind: Comparison::Less,
        },
        If { condition: 3 },
        Discard,
        Else,
        EndIf,
        Sample {
            dst: 5,
            uv: 0,
            texture: 0,
        },
        Else,
        Const {
            dst: 5,
            value: Vec4::new(0.8, 0.2, 0.1, 1.),
        },
        EndIf,
        Merge { dst: 6, a: 5, b: 5 },
        Output { slot: 0, src: 6 },
        Return,
    ])
    .unwrap();
    let inputs = [
        [Vec4::new(0.25, 0.4, 0., -1.)],
        [Vec4::new(0.75, 0.4, 0., 1.)],
        [Vec4::new(0.25, 0.4, 0., 1.)],
        [Vec4::new(0.75, 0.4, 0., 1.)],
    ];
    for mask in 0..16 {
        let mut samples = Vec::new();
        let packet = p
            .execute4(
                inputs.each_ref().map(|v| v.as_slice()),
                &[],
                [&[]; 4],
                mask,
                |i, slot, uv| {
                    samples.push((i, slot, bits(uv)));
                    Ok(Vec4::new(uv.x, uv.y, 0.6, 1.))
                },
                [true; 4],
            )
            .unwrap();
        for i in 0..4 {
            if mask & (1 << i) == 0 {
                assert_eq!(packet[i].instructions, 0);
                continue;
            }
            let mut calls = Vec::new();
            let reference = p
                .execute(
                    &inputs[i],
                    &[],
                    |slot, uv| {
                        calls.push((i, slot, bits(uv)));
                        Ok(Vec4::new(uv.x, uv.y, 0.6, 1.))
                    },
                    true,
                )
                .unwrap();
            assert_eq!(reference.discarded, i == 0);
            assert_eq!(reference.samples, usize::from(i == 2));
            assert_eq!(packet[i].discarded, reference.discarded);
            assert_eq!(packet[i].instructions, reference.instructions);
            assert_eq!(packet[i].samples, reference.samples);
            assert_eq!(packet[i].outputs.map(bits), reference.outputs.map(bits));
            let traces = |e: &shader::Execution| {
                e.trace
                    .iter()
                    .map(|t| (t.instruction, bits(t.value)))
                    .collect::<Vec<_>>()
            };
            assert_eq!(traces(&packet[i]), traces(&reference));
            assert_eq!(
                samples
                    .iter()
                    .filter(|(lane, _, _)| *lane == i)
                    .cloned()
                    .collect::<Vec<_>>(),
                calls
            );
        }
    }
    // A branch must preserve the other branch's previous value of a mutable register.
    let p = Program::new(vec![
        Input { dst: 0, slot: 0 },
        Const {
            dst: 1,
            value: Vec4::new(2., 2., 2., 2.),
        },
        If { condition: 0 },
        Const {
            dst: 1,
            value: Vec4::new(3., 3., 3., 3.),
        },
        Else,
        Add { dst: 1, a: 1, b: 1 },
        EndIf,
        Output { slot: 0, src: 1 },
    ])
    .unwrap();
    let inputs = [
        [Vec4::ZERO],
        [Vec4::new(1., 1., 1., 1.)],
        [Vec4::ZERO],
        [Vec4::new(1., 1., 1., 1.)],
    ];
    let e = p
        .execute4(
            inputs.each_ref().map(|v| v.as_slice()),
            &[],
            [&[]; 4],
            15,
            |_, _, _| unreachable!(),
            [false; 4],
        )
        .unwrap();
    assert_eq!(e.map(|e| e.outputs[0].x), [4., 3., 4., 3.]);
}
#[test]
fn selections_validate_definitions_and_skip_inactive_resources() {
    let c = Const {
        dst: 0,
        value: Vec4::ZERO,
    };
    for tail in [
        vec![Else],
        vec![EndIf],
        vec![If { condition: 0 }],
        vec![If { condition: 0 }, Else, Else, EndIf],
        vec![If { condition: 0 }, Output { slot: 0, src: 0 }, Else, EndIf],
        vec![
            If { condition: 0 },
            Const {
                dst: 1,
                value: Vec4::ZERO,
            },
            Else,
            Output { slot: 0, src: 1 },
            EndIf,
        ],
        vec![Merge { dst: 1, a: 0, b: 0 }],
        vec![Return],
    ] {
        let mut ops = vec![c.clone()];
        ops.extend(tail);
        assert!(Program::new(ops).is_err());
    }
    let mut ops = vec![c.clone()];
    for _ in 0..65 {
        ops.push(If { condition: 0 });
    }
    assert!(Program::new(ops).unwrap_err().contains("nesting"));
    let p = Program::new(vec![
        c,
        If { condition: 0 },
        Input { dst: 1, slot: 15 },
        Uniform { dst: 2, slot: 63 },
        SampleImplicit {
            dst: 3,
            uv: 1,
            texture: 15,
        },
        Div { dst: 4, a: 0, b: 0 },
        Output { slot: 0, src: 0 },
        Else,
        Output { slot: 0, src: 0 },
        EndIf,
    ])
    .unwrap();
    p.execute(&[], &[], |_, _| unreachable!(), true).unwrap();
    p.execute4(
        [&[]; 4],
        &[],
        [&[]; 4],
        15,
        |_, _, _| unreachable!(),
        [true; 4],
    )
    .unwrap();
}

#[test]
fn bounded_loops_match_scalar_for_divergent_packet_lanes() {
    let program = Program::new(vec![
        Input { dst: 0, slot: 0 },
        Input { dst: 5, slot: 1 },
        Const {
            dst: 1,
            value: Vec4::ZERO,
        },
        Const {
            dst: 2,
            value: Vec4::new(1., 1., 1., 1.),
        },
        Const {
            dst: 3,
            value: Vec4::ZERO,
        },
        Compare {
            dst: 4,
            a: 0,
            b: 1,
            kind: Comparison::Greater,
        },
        LoopStart { condition: 4 },
        Add { dst: 3, a: 3, b: 2 },
        Sample {
            dst: 7,
            uv: 5,
            texture: 0,
        },
        Sub { dst: 0, a: 0, b: 2 },
        Compare {
            dst: 4,
            a: 0,
            b: 1,
            kind: Comparison::Greater,
        },
        LoopEnd,
        Output { slot: 0, src: 3 },
    ])
    .unwrap();
    let inputs = [
        [Vec4::ZERO, Vec4::new(0.1, 0.2, 0., 0.)],
        [Vec4::new(1., 1., 1., 1.), Vec4::new(0.2, 0.3, 0., 0.)],
        [Vec4::new(3., 3., 3., 3.), Vec4::new(0.3, 0.4, 0., 0.)],
        [Vec4::new(5., 5., 5., 5.), Vec4::new(0.4, 0.5, 0., 0.)],
    ];
    let mut packet_samples = [0; 4];
    let packet = program
        .execute4(
            inputs.each_ref().map(|lane| lane.as_slice()),
            &[],
            [&[]; 4],
            0b1101,
            |lane, texture, uv| {
                assert_eq!(texture, 0);
                assert_eq!(uv, inputs[lane][1]);
                packet_samples[lane] += 1;
                Ok(Vec4::new(0.1, 0.2, 0.3, 1.))
            },
            [true; 4],
        )
        .unwrap();
    for lane in [0, 2, 3] {
        let scalar = program
            .execute(
                &inputs[lane],
                &[],
                |texture, uv| {
                    assert_eq!(texture, 0);
                    assert_eq!(uv, inputs[lane][1]);
                    Ok(Vec4::new(0.1, 0.2, 0.3, 1.))
                },
                true,
            )
            .unwrap();
        assert_eq!(bits(packet[lane].outputs[0]), bits(scalar.outputs[0]));
        assert_eq!(packet[lane].instructions, scalar.instructions);
        assert_eq!(packet[lane].samples, scalar.samples);
        assert_eq!(packet_samples[lane], scalar.samples);
        assert_eq!(
            packet[lane]
                .trace
                .iter()
                .map(|trace| (trace.instruction, bits(trace.value)))
                .collect::<Vec<_>>(),
            scalar
                .trace
                .iter()
                .map(|trace| (trace.instruction, bits(trace.value)))
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(packet_samples, [0, 0, 3, 5]);
    assert_eq!(packet[1].instructions, 0);
}

#[test]
fn loops_join_zero_iteration_definitions_and_enforce_the_instruction_budget() {
    let undefined_after_loop = Program::new(vec![
        Const {
            dst: 0,
            value: Vec4::ZERO,
        },
        Const {
            dst: 1,
            value: Vec4::new(1., 1., 1., 1.),
        },
        LoopStart { condition: 0 },
        Const {
            dst: 2,
            value: Vec4::new(1., 1., 1., 1.),
        },
        LoopEnd,
        Output { slot: 0, src: 2 },
    ])
    .unwrap_err();
    assert!(undefined_after_loop.contains("undefined register r2"));

    let unbounded = Program::new(vec![
        Const {
            dst: 0,
            value: Vec4::new(1., 1., 1., 1.),
        },
        LoopStart { condition: 0 },
        LoopEnd,
        Output { slot: 0, src: 0 },
    ])
    .unwrap();
    let error = unbounded
        .execute(&[], &[], |_, _| unreachable!(), false)
        .unwrap_err();
    assert!(error.contains("dynamic instruction limit"));

    let crosses_selection = Program::new(vec![
        Const {
            dst: 0,
            value: Vec4::new(1., 1., 1., 1.),
        },
        Const {
            dst: 1,
            value: Vec4::new(1., 1., 1., 1.),
        },
        If { condition: 0 },
        LoopStart { condition: 1 },
        Else,
        EndIf,
        LoopEnd,
        Output { slot: 0, src: 0 },
    ])
    .unwrap_err();
    assert!(crosses_selection.contains("loop crosses a selection boundary"));
}

#[test]
fn nested_loops_keep_independent_conditions_and_counters() {
    let program = Program::new(vec![
        Const {
            dst: 0,
            value: Vec4::new(2., 2., 2., 2.),
        },
        Const {
            dst: 1,
            value: Vec4::ZERO,
        },
        Const {
            dst: 2,
            value: Vec4::new(1., 1., 1., 1.),
        },
        Compare {
            dst: 3,
            a: 0,
            b: 1,
            kind: Comparison::Greater,
        },
        Const {
            dst: 4,
            value: Vec4::ZERO,
        },
        LoopStart { condition: 3 },
        Const {
            dst: 5,
            value: Vec4::new(3., 3., 3., 3.),
        },
        Compare {
            dst: 6,
            a: 5,
            b: 1,
            kind: Comparison::Greater,
        },
        LoopStart { condition: 6 },
        Add { dst: 4, a: 4, b: 2 },
        Sub { dst: 5, a: 5, b: 2 },
        Compare {
            dst: 6,
            a: 5,
            b: 1,
            kind: Comparison::Greater,
        },
        LoopEnd,
        Sub { dst: 0, a: 0, b: 2 },
        Compare {
            dst: 3,
            a: 0,
            b: 1,
            kind: Comparison::Greater,
        },
        LoopEnd,
        Output { slot: 0, src: 4 },
    ])
    .unwrap();
    assert_eq!(
        program
            .execute(&[], &[], |_, _| unreachable!(), false)
            .unwrap()
            .outputs[0],
        Vec4::new(6., 6., 6., 6.)
    );
}

#[test]
fn loop_headers_recompute_conditions_and_move_carried_values() {
    let program = Program::new(vec![
        Const {
            dst: 0,
            value: Vec4::ZERO,
        },
        Const {
            dst: 1,
            value: Vec4::new(1., 1., 1., 1.),
        },
        Const {
            dst: 2,
            value: Vec4::new(3., 3., 3., 3.),
        },
        LoopHeader,
        Compare {
            dst: 3,
            a: 0,
            b: 2,
            kind: Comparison::Less,
        },
        LoopStart { condition: 3 },
        Add { dst: 4, a: 0, b: 1 },
        Move { dst: 0, src: 4 },
        LoopEnd,
        Output { slot: 0, src: 0 },
    ])
    .unwrap();
    let scalar = program
        .execute(&[], &[], |_, _| unreachable!(), false)
        .unwrap();
    assert_eq!(scalar.outputs[0], Vec4::new(3., 3., 3., 3.));

    let packet = program
        .execute4(
            [&[]; 4],
            &[],
            [&[]; 4],
            0b1111,
            |_, _, _| unreachable!(),
            [false; 4],
        )
        .unwrap();
    assert!(
        packet
            .iter()
            .all(|lane| lane.outputs[0] == scalar.outputs[0])
    );
}

#[test]
fn loop_break_unwinds_nested_selections_and_preserves_loop_values() {
    let program = Program::new(vec![
        Const {
            dst: 0,
            value: Vec4::ZERO,
        },
        Const {
            dst: 1,
            value: Vec4::new(1., 1., 1., 1.),
        },
        Const {
            dst: 2,
            value: Vec4::new(3., 3., 3., 3.),
        },
        Const {
            dst: 3,
            value: Vec4::new(1., 1., 1., 1.),
        },
        LoopStart { condition: 3 },
        Compare {
            dst: 4,
            a: 0,
            b: 2,
            kind: Comparison::GreaterEqual,
        },
        If { condition: 4 },
        If { condition: 4 },
        LoopBreak,
        Else,
        Move { dst: 5, src: 0 },
        EndIf,
        Else,
        Move { dst: 5, src: 0 },
        EndIf,
        Add { dst: 0, a: 0, b: 1 },
        LoopEnd,
        Output { slot: 0, src: 0 },
    ])
    .unwrap();
    let scalar = program
        .execute(&[], &[], |_, _| unreachable!(), false)
        .unwrap();
    assert_eq!(scalar.outputs[0], Vec4::new(3., 3., 3., 3.));

    let packet = program
        .execute4(
            [&[]; 4],
            &[],
            [&[]; 4],
            0b1111,
            |_, _, _| unreachable!(),
            [false; 4],
        )
        .unwrap();
    assert!(
        packet
            .iter()
            .all(|lane| lane.outputs[0] == scalar.outputs[0])
    );
}

#[test]
fn loop_break_outside_a_loop_is_rejected() {
    let error = Program::new(vec![
        LoopBreak,
        Const {
            dst: 0,
            value: Vec4::ZERO,
        },
    ])
    .unwrap_err();
    assert!(error.contains("LoopBreak without LoopStart"));
}

#[test]
fn bounded_loops_survive_capture_round_trip() {
    use silicon::{
        Color, Device, FrameCapture, Pipeline, SampleCount, ShaderPipeline, Vec3, Vertex,
    };
    use std::sync::Arc;

    let vertex = Program::new(vec![
        Input { dst: 0, slot: 0 },
        Output { slot: 0, src: 0 },
        Input { dst: 1, slot: 2 },
        Output { slot: 2, src: 1 },
    ])
    .unwrap();
    let fragment = Program::new(vec![
        Const {
            dst: 0,
            value: Vec4::ZERO,
        },
        Const {
            dst: 1,
            value: Vec4::ZERO,
        },
        Const {
            dst: 2,
            value: Vec4::new(1., 1., 1., 1.),
        },
        Const {
            dst: 3,
            value: Vec4::new(0.25, 0.1, 0., 0.25),
        },
        Const {
            dst: 4,
            value: Vec4::new(2., 2., 2., 2.),
        },
        Compare {
            dst: 5,
            a: 0,
            b: 4,
            kind: Comparison::Less,
        },
        LoopStart { condition: 5 },
        Add { dst: 0, a: 0, b: 2 },
        Add { dst: 1, a: 1, b: 3 },
        Compare {
            dst: 5,
            a: 0,
            b: 4,
            kind: Comparison::Less,
        },
        LoopEnd,
        Output { slot: 0, src: 1 },
    ])
    .unwrap();
    let mut commands = Device.commands();
    commands.begin_render_pass(Color::BLACK);
    commands.bind_pipeline(Arc::new(ShaderPipeline {
        vertex,
        fragment,
        state: Pipeline::default(),
    }));
    let vertices = [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)]
        .map(|(x, y)| Vertex::new(Vec3::new(x, y, 0.5), Color::WHITE));
    commands.bind_vertex_buffer(Device.create_vertex_buffer(vertices.to_vec()).unwrap());
    commands.bind_index_buffer(Device.create_index_buffer(vec![0, 1, 2, 0, 2, 3]).unwrap());
    commands.draw_indexed(0, 6);
    commands.end_render_pass();
    let capture = FrameCapture {
        version: 2,
        width: 8,
        height: 8,
        sample_count: SampleCount::One,
        commands,
    };
    let before = capture.replay().unwrap();
    let path = std::env::temp_dir().join(format!(
        "silicon-loop-capture-{}.silicon",
        std::process::id()
    ));
    capture.save(&path).unwrap();
    let replay = FrameCapture::load(&path).unwrap().replay().unwrap();
    std::fs::remove_file(path).unwrap();

    assert!(before.stats.shader_instructions > 0);
    assert_eq!(
        before.stats.shader_instructions,
        replay.stats.shader_instructions
    );
    for y in 0..8 {
        for x in 0..8 {
            assert_eq!(
                before.framebuffer.pixel(x, y).unwrap().rgba8(),
                replay.framebuffer.pixel(x, y).unwrap().rgba8()
            );
        }
    }
}

#[test]
fn glsl_selections_locals_phi_and_early_returns_match_independent_reference() {
    use shader::spirv::{Module, link};
    let vertex = Module::parse(include_bytes!("../assets/shaders/textured.vert.spv"))
        .unwrap()
        .translate()
        .unwrap();
    let source = include_bytes!("../assets/shaders/control.frag.spv");
    let ssa = include_bytes!("../assets/shaders/control.ssa.frag.spv");
    let weighted = add_branch_weights(ssa);
    assert!(
        Module::parse(ssa)
            .unwrap()
            .instructions()
            .iter()
            .any(|op| op.opcode == 245)
    );
    for bytes in [source.as_slice(), ssa.as_slice(), weighted.as_slice()] {
        let fragment = Module::parse(bytes).unwrap().translate().unwrap();
        link(&vertex, &fragment).unwrap();
        let p = fragment.program;
        for coords in [
            [(0.1, 0.2), (0.4, 0.2), (0.9, 0.2), (0.4, 0.9)],
            [(0.25, 0.5), (0.65, 0.8), (0.1, 0.9), (0.9, 0.9)],
        ] {
            let color = Vec4::new(0.7, 0.6, 0.5, 1.);
            let texel = Vec4::new(0.8, 0.4, 0.2, 1.);
            let inputs = coords.map(|(x, y)| [color, Vec4::new(x, y, 17., 23.)]);
            for mask in 0..16 {
                let mut calls = [0; 4];
                let packet = p
                    .execute4(
                        inputs.each_ref().map(|v| v.as_slice()),
                        &[],
                        [&[2.]; 4],
                        mask,
                        |i, slot, uv| {
                            assert_eq!(slot, 0);
                            assert_eq!(bits(uv), bits(Vec4::new(coords[i].0, coords[i].1, 2., 0.)));
                            calls[i] += 1;
                            Ok(texel)
                        },
                        [true; 4],
                    )
                    .unwrap();
                for i in 0..4 {
                    if mask & (1 << i) == 0 {
                        assert_eq!(calls[i], 0);
                        continue;
                    }
                    let scalar = p
                        .execute_with_lod(&inputs[i], &[], &[2.], |_, _| Ok(texel), true)
                        .unwrap();
                    let (x, y) = coords[i];
                    let discard = x < 0.25 && y < 0.5;
                    let samples = usize::from(!discard && x < 0.65);
                    assert_eq!(scalar.discarded, discard);
                    assert_eq!(scalar.samples, samples);
                    assert_eq!(calls[i], samples);
                    if !discard {
                        let base = if x < 0.65 {
                            texel.component_mul(color)
                        } else {
                            Vec4::new(0.95, 0.28, 0.08, 1.)
                        };
                        let expected = if y > 0.8 {
                            base
                        } else {
                            base.component_mul(Vec4::new(0.5, 0.8, 1., 1.))
                        };
                        assert_eq!(bits(scalar.outputs[0]), bits(expected));
                    }
                    assert_eq!(packet[i].outputs.map(bits), scalar.outputs.map(bits));
                    assert_eq!(packet[i].discarded, scalar.discarded);
                    assert_eq!(packet[i].instructions, scalar.instructions);
                    assert_eq!(packet[i].samples, scalar.samples);
                    assert_eq!(
                        packet[i]
                            .trace
                            .iter()
                            .map(|t| (t.instruction, bits(t.value)))
                            .collect::<Vec<_>>(),
                        scalar
                            .trace
                            .iter()
                            .map(|t| (t.instruction, bits(t.value)))
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
    }
}

#[test]
fn discard_preserves_color_depth_stencil_and_capture_in_every_backend() {
    use silicon::{
        Backend, Color, Compare as DepthCompare, Device, FrameCapture, Pipeline, Renderer,
        ShaderPipeline, StencilOp, StencilState, Vec2, Vec3, Vertex,
    };
    use std::sync::Arc;
    let vertex = Program::new(vec![
        Input { dst: 0, slot: 0 },
        Output { slot: 0, src: 0 },
        Input { dst: 1, slot: 2 },
        Output { slot: 2, src: 1 },
    ])
    .unwrap();
    let fragment = Program::new(vec![
        Input { dst: 0, slot: 1 },
        Const {
            dst: 1,
            value: Vec4::new(0.5, 0.5, 0.5, 0.5),
        },
        Compare {
            dst: 2,
            a: 0,
            b: 1,
            kind: Comparison::Less,
        },
        If { condition: 2 },
        Discard,
        Else,
        Const {
            dst: 3,
            value: Vec4::new(0.8, 0.2, 0.1, 1.),
        },
        Output { slot: 0, src: 3 },
        EndIf,
        Return,
    ])
    .unwrap();
    let pipeline = Arc::new(ShaderPipeline {
        vertex,
        fragment,
        state: Pipeline {
            stencil: Some(StencilState {
                compare: DepthCompare::Always,
                reference: 7,
                read_mask: 255,
                write_mask: 255,
                fail: StencilOp::Keep,
                depth_fail: StencilOp::Keep,
                pass: StencilOp::Replace,
            }),
            ..Default::default()
        },
    });
    let mut commands = Device.commands();
    let clear = Color::new(0.1, 0.2, 0.3, 1.);
    commands.begin_render_pass(clear);
    commands.bind_pipeline(pipeline.clone());
    let vertices = [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)].map(|(x, y)| {
        let mut v = Vertex::new(Vec3::new(x, y, 0.5), Color::WHITE);
        v.uv = Vec2::new((x + 1.) * 0.5, (y + 1.) * 0.5);
        v
    });
    commands.bind_vertex_buffer(Device.create_vertex_buffer(vertices.to_vec()).unwrap());
    commands.bind_index_buffer(Device.create_index_buffer(vec![0, 1, 2, 0, 2, 3]).unwrap());
    commands.draw_indexed(0, 6);
    commands.end_render_pass();
    let capture = FrameCapture {
        version: 2,
        width: 4,
        height: 2,
        sample_count: SampleCount::One,
        commands,
    };
    let path = std::env::temp_dir().join(format!("silicon-control-{}.silicon", std::process::id()));
    capture.save(&path).unwrap();
    let capture = FrameCapture::load(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    for (backend, workers) in [(Backend::Scalar, 1), (Backend::Simd, 1), (Backend::Simd, 2)] {
        let mut r = Renderer::new(4, 2).unwrap();
        r.backend = backend;
        if workers == 1 {
            r.debug_pixel = Some((0, 0));
            let stats = Device.submit(&capture.commands, &mut r).unwrap();
            assert_eq!(stats.shader_instructions, 72);
            assert!(
                stats
                    .shader_traces
                    .iter()
                    .any(|(_, t)| t.iter().any(|t| matches!(t.operation, Discard)))
            );
        } else {
            r.render_bands(workers, |r| Device.submit(&capture.commands, r).map(|_| ()))
                .unwrap();
        }
        assert_eq!(r.stats.shaded, 8);
        assert_eq!(r.stats.discarded, 4);
        assert_eq!(r.stats.texture_samples, 0);
        assert_eq!(r.stats.shader_instructions, 56 + 16 * workers as u64);
        for y in 0..2 {
            for x in 0..4 {
                let killed = x < 2;
                assert_eq!(
                    r.framebuffer.pixel(x, y).unwrap().rgba8(),
                    if killed {
                        clear.rgba8()
                    } else {
                        Color::new(0.8, 0.2, 0.1, 1.).rgba8()
                    }
                );
                assert_eq!(
                    r.framebuffer.depth_at(x, y),
                    Some(if killed { 1. } else { 0.5 })
                );
                assert_eq!(
                    r.framebuffer.stencil_at(x, y),
                    Some(if killed { 0 } else { 7 })
                );
            }
        }
    }
    let mut invalid = Device.commands();
    invalid.begin_render_pass(Color::BLACK);
    invalid.bind_pipeline(Arc::new(ShaderPipeline {
        vertex: Program::new(vec![Discard]).unwrap(),
        ..(*pipeline).clone()
    }));
    invalid.end_render_pass();
    let mut r = Renderer::new(4, 2).unwrap();
    r.clear(Color::WHITE);
    let before = r.framebuffer.bytes().to_vec();
    assert!(
        Device
            .submit(&invalid, &mut r)
            .unwrap_err()
            .to_string()
            .contains("fragment")
    );
    assert_eq!(before, r.framebuffer.bytes());
}

#[test]
fn real_glsl_boolean_math_and_select_execute_in_divergent_packets() {
    for source in [
        include_bytes!("../assets/shaders/boolean.frag.spv").as_slice(),
        include_bytes!("../assets/shaders/boolean.locals.frag.spv").as_slice(),
    ] {
        let p = shader::spirv::Module::parse(source)
            .unwrap()
            .translate()
            .unwrap()
            .program;
        assert!(
            p.instructions()
                .iter()
                .any(|op| matches!(op, Select { .. }))
        );
        let color = Vec4::new(0.7, 0.6, 0.5, 1.);
        let coords = [(0.2, 0.2), (0.2, 0.8), (0.8, 0.2), (0.8, 0.8)];
        let inputs = coords.map(|(x, y)| [color, Vec4::new(x, y, 0., 0.)]);
        let packet = p
            .execute4(
                inputs.each_ref().map(|v| v.as_slice()),
                &[],
                [&[]; 4],
                15,
                |_, _, _| unreachable!(),
                [true; 4],
            )
            .unwrap();
        for i in 0..4 {
            let (x, y) = coords[i];
            let (a, b) = (x <= 0.5, y >= 0.5);
            let weight = if a && b { 0.2 } else { 0.8 };
            let scalar = p
                .execute(&inputs[i], &[], |_, _| unreachable!(), true)
                .unwrap();
            assert_eq!(bits(scalar.outputs[0]), bits(color * weight));
            assert_eq!(packet[i].outputs.map(bits), scalar.outputs.map(bits));
        }
    }
}

#[test]
fn malformed_selection_graphs_phi_and_path_definitions_are_rejected() {
    use shader::spirv::Module;
    let source = include_bytes!("../assets/shaders/control.ssa.frag.spv");
    let module = Module::parse(source).unwrap();
    let original: Vec<_> = source
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let reject = |w: Vec<u32>| {
        let b: Vec<_> = w.into_iter().flat_map(u32::to_le_bytes).collect();
        assert!(Module::parse(&b).and_then(|m| m.translate()).is_err());
    };
    let find = |opcode| {
        module
            .instructions()
            .iter()
            .find(|op| op.opcode == opcode)
            .unwrap()
    };
    let branch = find(250);
    let label = find(248).operands[0];
    let phi = find(245);
    let merge = find(247);
    let float_constant = find(43).operands[1];
    for (op, operand, value) in [
        (branch, 0, float_constant),     // non-bool condition
        (branch, 1, label),              // back edge
        (branch, 1, float_constant),     // non-label target
        (branch, 2, branch.operands[1]), // overlapping arms
        (merge, 0, label),               // invalid merge
        (merge, 1, 1),                   // unsupported control
        (phi, 3, phi.operands[5]),       // duplicate predecessor
        (phi, 2, phi.operands[1]),       // phi self-reference
        (phi, 4, phi.operands[2]),       // false arm reads a true-only value
        (phi, 0, find(20).operands[0]),  // wrong result type
    ] {
        let mut w = original.clone();
        w[op.word + 1 + operand] = value;
        reject(w);
    }
    // SPIR-V 1.0 requires a vector bool condition for vector Select results.
    let constants: Vec<_> = module
        .instructions()
        .iter()
        .filter(|op| op.opcode == 44)
        .collect();
    let mut w = original.clone();
    let result = w[3];
    w[3] += 1;
    w.splice(
        merge.word..merge.word,
        [
            (6 << 16) | 169,
            constants[0].operands[0],
            result,
            branch.operands[0],
            constants[0].operands[1],
            constants[1].operands[1],
        ],
    );
    let b: Vec<_> = w.into_iter().flat_map(u32::to_le_bytes).collect();
    let error = Module::parse(&b).unwrap().translate().unwrap_err();
    assert!(
        error.contains("OpSelect") && error.contains("vector bool"),
        "{error}"
    );
    // Remove the local's else initialization in the non-SSA version.
    let local_source = include_bytes!("../assets/shaders/control.frag.spv");
    let local_module = Module::parse(local_source).unwrap();
    let local = local_module
        .instructions()
        .iter()
        .find(|op| op.opcode == 59 && op.operands[2] == 7)
        .unwrap()
        .operands[1];
    let store = local_module
        .instructions()
        .iter()
        .filter(|op| op.opcode == 62 && op.operands[0] == local)
        .nth(1)
        .unwrap();
    let mut w: Vec<_> = local_source
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    w.drain(store.word..store.word + 3);
    reject(w);
    // Exercise control-flow validation with deterministic arbitrary binary mutations too.
    let mut seed = 0x62ba7129u32;
    for _ in 0..1000 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let index = seed as usize % original.len();
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let mut w = original.clone();
        w[index] ^= seed;
        let b: Vec<_> = w.into_iter().flat_map(u32::to_le_bytes).collect();
        assert!(
            std::panic::catch_unwind(|| Module::parse(&b).and_then(|m| m.translate())).is_ok(),
            "word {index}"
        );
    }
}

#[test]
fn compiled_cutout_scene_replays_exactly_with_scalar_packets_and_worker_bands() {
    use silicon::{Backend, Device, Renderer, demo};
    let capture = demo::spirv_cutout(97, 65, 0.37).unwrap();
    let reference = capture.replay().unwrap();
    assert!(reference.stats.discarded > 0);
    assert!(reference.stats.texture_samples > 0);
    assert!(reference.stats.texture_samples < reference.stats.shaded - reference.stats.discarded);
    for workers in [1, 4] {
        let mut actual = Renderer::new(97, 65).unwrap();
        actual.backend = Backend::Simd;
        actual
            .render_bands(workers, |r| Device.submit(&capture.commands, r).map(|_| ()))
            .unwrap();
        assert_eq!(actual.framebuffer.bytes(), reference.framebuffer.bytes());
        assert_eq!(actual.stats.discarded, reference.stats.discarded);
        assert_eq!(
            actual.stats.texture_samples,
            reference.stats.texture_samples
        );
        for y in 0..65 {
            for x in 0..97 {
                assert_eq!(
                    actual.framebuffer.depth_at(x, y),
                    reference.framebuffer.depth_at(x, y)
                );
                assert_eq!(
                    actual.framebuffer.stencil_at(x, y),
                    reference.framebuffer.stencil_at(x, y)
                );
            }
        }
    }
    let unoptimized =
        shader::spirv::Module::parse(include_bytes!("../assets/shaders/control.frag.spv"))
            .unwrap()
            .translate()
            .unwrap()
            .program;
    let vertex =
        shader::spirv::Module::parse(include_bytes!("../assets/shaders/textured.vert.spv"))
            .unwrap()
            .translate()
            .unwrap()
            .program;
    let actual = demo::shader_cube_with_programs(97, 65, 0.37, vertex, unoptimized)
        .unwrap()
        .replay()
        .unwrap();
    assert_eq!(actual.framebuffer.bytes(), reference.framebuffer.bytes());
    assert_eq!(actual.stats.discarded, reference.stats.discarded);
    assert_eq!(
        actual.stats.texture_samples,
        reference.stats.texture_samples
    );
}
