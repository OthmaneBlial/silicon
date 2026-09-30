use silicon::*;
#[test]
fn bounded_geometry_and_shader_mutation_check() {
    let mut seed = 0x53494c49u32;
    let mut next = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        seed
    };
    let mut r = Renderer::new(8, 8).unwrap();
    for _ in 0..500 {
        let mut inputs = Vec::new();
        for _ in 0..3 {
            let mut v = Vertex::new(Vec3::ZERO, Color::WHITE);
            v.position = Vec3::new(
                (next() as i32 as f32) / 1e8,
                (next() as i32 as f32) / 1e8,
                (next() as i32 as f32) / 1e8,
            );
            inputs.push(v);
        }
        r.clear(Color::BLACK);
        r.draw(
            &inputs,
            None,
            Pipeline::default(),
            |v| VertexOutput {
                position: v.position.extend(1.),
                varyings: [v.color, Vec4::ZERO, Vec4::ZERO, Vec4::ZERO],
            },
            |_| Some(Color::WHITE),
        )
        .unwrap();
        assert_eq!(r.framebuffer.bytes().len(), 256);
        use shader::{Instruction::*, Program};
        let reg = (next() % 256) as u8;
        let p = Program::new(vec![
            Input { dst: 0, slot: 0 },
            Add {
                dst: 1,
                a: 0,
                b: reg,
            },
            Output { slot: 0, src: 1 },
        ]);
        if let Ok(p) = p {
            let e = p
                .execute(
                    &[Vec4::new(1., 1., 1., 1.)],
                    &[],
                    |_, _| Err("missing texture".into()),
                    false,
                )
                .unwrap();
            assert_eq!(e.outputs[0], Vec4::new(2., 2., 2., 2.));
        }
    }
}
