use silicon::*;
fn vertex(v: &Vertex) -> VertexOutput {
    VertexOutput {
        position: v.position.extend(1.),
        varyings: [
            v.color,
            Vec4::new(v.uv.x, v.uv.y, 0., 0.),
            v.normal.extend(0.),
            v.position.extend(1.),
        ],
    }
}
fn triangle(z: f32, c: Color) -> [Vertex; 3] {
    [
        Vertex::new(Vec3::new(-1., -1., z), c),
        Vertex::new(Vec3::new(1., -1., z), c),
        Vertex::new(Vec3::new(-1., 1., z), c),
    ]
}
#[test]
fn shared_edges_exactly_once() {
    let mut r = Renderer::new(16, 16).unwrap();
    r.clear(Color::BLACK);
    let c = Color::new(1., 0., 0., 0.5);
    let mut v = triangle(0.5, c).to_vec();
    v.extend([
        Vertex::new(Vec3::new(1., -1., 0.5), c),
        Vertex::new(Vec3::new(1., 1., 0.5), c),
        Vertex::new(Vec3::new(-1., 1., 0.5), c),
    ]);
    r.draw(
        &v,
        None,
        Pipeline {
            depth_compare: Compare::Always,
            depth_write: false,
            blend: Blend::Alpha,
            ..Default::default()
        },
        vertex,
        |f| Some(f.color()),
    )
    .unwrap();
    assert_eq!(r.stats.fragments, 256);
    for y in 0..16 {
        for x in 0..16 {
            assert_eq!(r.framebuffer.pixel(x, y).unwrap().rgba8(), [128, 0, 0, 255]);
        }
    }
}
#[test]
fn depth_discard_culling_and_invalid_indices() {
    let mut r = Renderer::new(32, 32).unwrap();
    r.clear(Color::BLACK);
    r.draw(
        &triangle(0.2, Color::WHITE),
        None,
        Pipeline::default(),
        vertex,
        |_| None,
    )
    .unwrap();
    assert_eq!(r.framebuffer.depth_at(4, 20), Some(1.));
    r.draw(
        &triangle(0.3, Color::WHITE),
        None,
        Pipeline::default(),
        vertex,
        |f| Some(f.color()),
    )
    .unwrap();
    let before = r.framebuffer.bytes().to_vec();
    r.draw(
        &triangle(0.6, Color::new(1., 0., 0., 1.)),
        None,
        Pipeline::default(),
        vertex,
        |f| Some(f.color()),
    )
    .unwrap();
    assert_eq!(before, r.framebuffer.bytes());
    assert!(r.stats.early_z_rejected > 0);
    assert!(
        r.draw(
            &triangle(0.2, Color::WHITE),
            Some(&[0, 1, 3]),
            Pipeline::default(),
            vertex,
            |f| Some(f.color())
        )
        .is_err()
    );
    r.clear(Color::BLACK);
    r.draw(
        &triangle(0.2, Color::WHITE),
        None,
        Pipeline {
            cull: Cull::Front,
            ..Default::default()
        },
        vertex,
        |f| Some(f.color()),
    )
    .unwrap();
    assert_eq!(r.stats.shaded, 0);
    assert_eq!(r.stats.culled, 1);
}
#[test]
fn perspective_correct_varying_and_affine_depth() {
    let mut r = Renderer::new(8, 8).unwrap();
    r.clear(Color::BLACK);
    r.debug_pixel = Some((2, 4));
    let mut vs = triangle(0.5, Color::WHITE);
    vs[0].uv.x = 0.;
    vs[1].uv.x = 1.;
    vs[2].uv.x = 0.;
    r.draw(
        &vs,
        None,
        Pipeline::default(),
        |v| {
            let mut o = vertex(v);
            let w = if v.position.x > 0. { 2. } else { 1. };
            o.position = o.position * w;
            o
        },
        |f| Some(f.color()),
    )
    .unwrap();
    let f = r.traces[0].fragment;
    let b = f.barycentric;
    let correct = (b.y / 2.) / (b.x + b.y / 2. + b.z);
    assert!((f.uv().x - correct).abs() < 1e-6);
    assert!((f.uv().x - b.y).abs() > 0.02);
    assert!((f.depth - 0.5).abs() < 1e-6);
}
#[test]
fn lines_clip_and_cover_endpoints() {
    let mut fb = Framebuffer::new(8, 8).unwrap();
    fb.clear(Color::BLACK);
    draw_line(
        &mut fb,
        Vec2::new(-100., 4.),
        Vec2::new(100., 4.),
        Color::WHITE,
    )
    .unwrap();
    for x in 0..8 {
        assert_eq!(fb.pixel(x, 4), Some(Color::WHITE));
    }
    draw_line(&mut fb, Vec2::new(0., 0.), Vec2::new(7., 7.), Color::WHITE).unwrap();
    assert_eq!(fb.pixel(7, 7), Some(Color::WHITE));
}
#[test]
fn scalar_simd_and_parallel_frames_are_identical() {
    let c = demo::shader_cube(97, 65, 0.4).unwrap();
    let expected = c.replay().unwrap();
    for backend in [Backend::Scalar, Backend::Simd] {
        for threads in [1, 2, 4] {
            let mut r = Renderer::new(97, 65).unwrap();
            r.backend = backend;
            r.render_bands(threads, |r| Device.submit(&c.commands, r).map(|_| ()))
                .unwrap();
            assert_eq!(
                r.framebuffer.bytes(),
                expected.framebuffer.bytes(),
                "{backend:?}/{threads}"
            );
            assert_eq!(r.stats.fragments, expected.stats.fragments);
            assert_eq!(r.stats.shaded, expected.stats.shaded);
            assert_eq!(r.stats.triangles, 12);
        }
    }
}
#[test]
fn stencil_masks_restrict_color_and_pass_ops() {
    let mut r = Renderer::new(16, 16).unwrap();
    r.clear(Color::BLACK);
    let st = StencilState {
        compare: Compare::Always,
        reference: 3,
        read_mask: 255,
        write_mask: 15,
        fail: StencilOp::Keep,
        depth_fail: StencilOp::Keep,
        pass: StencilOp::Replace,
    };
    r.draw(
        &triangle(0.5, Color::WHITE),
        None,
        Pipeline {
            color_write: false,
            depth_write: false,
            stencil: Some(st),
            ..Default::default()
        },
        vertex,
        |f| Some(f.color()),
    )
    .unwrap();
    assert!(
        r.framebuffer
            .bytes()
            .chunks_exact(4)
            .all(|p| p == [0, 0, 0, 255])
    );
    assert_eq!(r.framebuffer.stencil_at(2, 12), Some(3));
    let st = StencilState {
        compare: Compare::Equal,
        write_mask: 0,
        pass: StencilOp::Keep,
        ..st
    };
    let quad = [
        Vertex::new(Vec3::new(-1., -1., 0.2), Color::WHITE),
        Vertex::new(Vec3::new(1., -1., 0.2), Color::WHITE),
        Vertex::new(Vec3::new(-1., 1., 0.2), Color::WHITE),
        Vertex::new(Vec3::new(1., -1., 0.2), Color::WHITE),
        Vertex::new(Vec3::new(1., 1., 0.2), Color::WHITE),
        Vertex::new(Vec3::new(-1., 1., 0.2), Color::WHITE),
    ];
    r.draw(
        &quad,
        None,
        Pipeline {
            stencil: Some(st),
            ..Default::default()
        },
        vertex,
        |f| Some(f.color()),
    )
    .unwrap();
    assert_eq!(r.framebuffer.pixel(2, 12), Some(Color::WHITE));
    assert_eq!(r.framebuffer.pixel(12, 2), Some(Color::BLACK));
    assert!(r.stats.stencil_rejected > 0);
}
#[test]
fn homogeneous_scale_and_extreme_coordinates_remain_safe() {
    let v = triangle(0.5, Color::WHITE);
    let mut reference = Renderer::new(8, 8).unwrap();
    reference.clear(Color::BLACK);
    reference
        .draw(&v, None, Pipeline::default(), vertex, |f| Some(f.color()))
        .unwrap();
    for scale in [1e-20, 1e20] {
        let mut r = Renderer::new(8, 8).unwrap();
        r.clear(Color::BLACK);
        r.draw(
            &v,
            None,
            Pipeline::default(),
            |v| {
                let mut o = vertex(v);
                o.position = o.position * scale;
                o
            },
            |f| Some(f.color()),
        )
        .unwrap();
        assert_eq!(r.framebuffer.bytes(), reference.framebuffer.bytes());
    }
    let v = |p| VertexOutput {
        position: p,
        varyings: [Color::WHITE.0; 4],
    };
    let clipped = clip_triangle([
        v(Vec4::new(f32::MAX, 0., 1., 2.)),
        v(Vec4::new(0., -1., 1., 2.)),
        v(Vec4::new(0., 1., 1., 2.)),
    ]);
    assert!(!clipped.is_empty());
    assert!(clipped.iter().all(VertexOutput::is_finite));
    assert!(clipped.iter().all(|v| v.position.x.abs() <= v.position.w));
}
