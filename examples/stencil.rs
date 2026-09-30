use silicon::*;
fn output(v: &Vertex) -> VertexOutput {
    VertexOutput {
        position: v.position.extend(1.),
        varyings: [
            v.color,
            Vec4::new(v.uv.x, v.uv.y, 0., 0.),
            Vec4::ZERO,
            Vec4::ZERO,
        ],
    }
}
fn main() -> Result<()> {
    let mut r = Renderer::new(800, 600)?;
    r.clear(Color::new(0.025, 0.035, 0.055, 1.));
    let st = StencilState {
        compare: Compare::Always,
        reference: 1,
        read_mask: 255,
        write_mask: 255,
        fail: StencilOp::Keep,
        depth_fail: StencilOp::Keep,
        pass: StencilOp::Replace,
    };
    let mut mask = Vec::new();
    for i in 0..64 {
        let a = i as f32 * std::f32::consts::TAU / 64.;
        let b = (i + 1) as f32 * std::f32::consts::TAU / 64.;
        for (x, y) in [
            (0., 0.),
            (a.cos() * 0.6, a.sin() * 0.8),
            (b.cos() * 0.6, b.sin() * 0.8),
        ] {
            mask.push(Vertex::new(Vec3::new(x, y, 0.5), Color::WHITE));
        }
    }
    r.draw(
        &mask,
        None,
        Pipeline {
            color_write: false,
            depth_write: false,
            stencil: Some(st),
            ..Default::default()
        },
        output,
        |f| Some(f.color()),
    )?;
    let texture = Texture::checker(128)?;
    let cube = Mesh::cube();
    let mvp = Mat4::perspective(0.85, 4. / 3., 0.1, 20.)
        * Mat4::look_at(Vec3::new(3., 2., 4.), Vec3::ZERO, Vec3::new(0., 1., 0.))
        * Mat4::rotation_y(0.6);
    r.try_draw(
        &cube.vertices,
        Some(&cube.indices),
        Pipeline {
            stencil: Some(StencilState {
                compare: Compare::Equal,
                write_mask: 0,
                pass: StencilOp::Keep,
                ..st
            }),
            ..Default::default()
        },
        |v| {
            let mut o = output(v);
            o.position = mvp.transform(v.position.extend(1.));
            Ok(o)
        },
        |f| {
            Ok(Some(texture.sample(
                f.uv(),
                texture.lod(f.uv_dx, f.uv_dy),
                Sampler::default(),
            )?))
        },
    )?;
    let overlay = [
        Vertex::new(Vec3::new(-0.9, -0.6, 0.1), Color::new(1., 0.2, 0.1, 0.45)),
        Vertex::new(Vec3::new(0.9, -0.6, 0.1), Color::new(0.1, 1., 0.6, 0.45)),
        Vertex::new(Vec3::new(0., 0.7, 0.1), Color::new(0.2, 0.3, 1., 0.45)),
    ];
    r.draw(
        &overlay,
        None,
        Pipeline {
            blend: Blend::Alpha,
            depth_write: false,
            ..Default::default()
        },
        output,
        |f| Some(f.color()),
    )?;
    r.framebuffer.save_png("assets/screenshots/stencil.png")
}
