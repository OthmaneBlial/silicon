use silicon::*;

fn vertex(v: &Vertex) -> VertexOutput {
    VertexOutput {
        position: v.position.extend(1.),
        varyings: [v.color, Vec4::ZERO, Vec4::ZERO, v.position.extend(1.)],
    }
}

fn rect(width: f32, height: f32, x0: f32, x1: f32, z: f32, color: Color) -> Vec<Vertex> {
    let point = |x, y| {
        Vertex::new(
            Vec3::new(x / width * 2. - 1., 1. - y / height * 2., z),
            color,
        )
    };
    let (top_left, top_right, bottom_right, bottom_left) = (
        point(x0, 0.),
        point(x1, 0.),
        point(x1, height),
        point(x0, height),
    );
    vec![
        top_left,
        top_right,
        bottom_right,
        top_left,
        bottom_right,
        bottom_left,
    ]
}

fn sloped_rect(
    width: f32,
    height: f32,
    x0: f32,
    x1: f32,
    z0: f32,
    z1: f32,
    color: Color,
) -> Vec<Vertex> {
    let point = |x, y, z| {
        Vertex::new(
            Vec3::new(x / width * 2. - 1., 1. - y / height * 2., z),
            color,
        )
    };
    let (top_left, top_right, bottom_right, bottom_left) = (
        point(x0, 0., z0),
        point(x1, 0., z1),
        point(x1, height, z1),
        point(x0, height, z0),
    );
    vec![
        top_left,
        top_right,
        bottom_right,
        top_left,
        bottom_right,
        bottom_left,
    ]
}

fn render_msaa(r: &mut Renderer) -> Result<()> {
    r.clear(Color::BLACK);
    let (width, height) = r.surface_size();
    let (width, height) = (width as f32, height as f32);
    let red = rect(
        width,
        height,
        0.,
        width * 0.5625,
        0.25,
        Color::new(1., 0., 0., 1.),
    );
    let blue = rect(width, height, 0., width, 0.75, Color::new(0., 0., 1., 1.));
    r.draw(
        &red,
        None,
        Pipeline {
            stencil: Some(StencilState {
                compare: Compare::Always,
                reference: 1,
                read_mask: 255,
                write_mask: 255,
                fail: StencilOp::Keep,
                depth_fail: StencilOp::Keep,
                pass: StencilOp::Replace,
            }),
            ..Default::default()
        },
        vertex,
        |f| Some(f.color()),
    )?;
    r.draw(
        &blue,
        None,
        Pipeline {
            stencil: Some(StencilState {
                compare: Compare::Equal,
                reference: 0,
                read_mask: 255,
                write_mask: 0,
                fail: StencilOp::Keep,
                depth_fail: StencilOp::Keep,
                pass: StencilOp::Keep,
            }),
            ..Default::default()
        },
        vertex,
        |f| Some(f.color()),
    )?;
    Ok(())
}

#[test]
fn two_and_four_sample_resolves_track_coverage_and_depth() {
    for count in [SampleCount::Two, SampleCount::Four] {
        let mut renderer = Renderer::new(8, 8).unwrap();
        renderer.set_sample_count(count).unwrap();
        render_msaa(&mut renderer).unwrap();
        assert_eq!(
            renderer.framebuffer.pixel(3, 3).unwrap().rgba8(),
            [255, 0, 0, 255]
        );
        assert_eq!(
            renderer.framebuffer.pixel(4, 3).unwrap().rgba8(),
            [128, 0, 128, 255]
        );
        assert_eq!(
            renderer.framebuffer.pixel(5, 3).unwrap().rgba8(),
            [0, 0, 255, 255]
        );
        assert_eq!(renderer.framebuffer.depth_at(4, 3), Some(0.25));
        assert_eq!(renderer.framebuffer.stencil_at(4, 3), Some(1));
    }
}

#[test]
fn four_sample_depth_is_interpolated_at_each_covered_position() {
    let mut renderer = Renderer::new(8, 8).unwrap();
    renderer.set_sample_count(SampleCount::Four).unwrap();
    renderer.clear(Color::BLACK);
    let red = sloped_rect(8., 8., 0., 4.5, 0.2, 0.8, Color::new(1., 0., 0., 1.));
    let blue = rect(8., 8., 0., 8., 0.765, Color::new(0., 0., 1., 1.));
    for geometry in [&red, &blue] {
        renderer
            .draw(geometry, None, Pipeline::default(), vertex, |f| {
                Some(f.color())
            })
            .unwrap();
    }
    assert_eq!(
        renderer.framebuffer.pixel(4, 3).unwrap().rgba8(),
        [64, 0, 191, 255]
    );
}

#[test]
fn four_sample_resolve_matches_simd_worker_bands() {
    let mut scalar = Renderer::new(32, 24).unwrap();
    scalar.set_sample_count(SampleCount::Four).unwrap();
    render_msaa(&mut scalar).unwrap();

    let mut parallel = Renderer::new(32, 24).unwrap();
    parallel.set_sample_count(SampleCount::Four).unwrap();
    parallel.backend = Backend::Simd;
    parallel.render_bands(4, render_msaa).unwrap();
    assert_eq!(parallel.framebuffer.bytes(), scalar.framebuffer.bytes());
    for y in 0..24 {
        for x in 0..32 {
            assert_eq!(
                parallel.framebuffer.depth_at(x, y),
                scalar.framebuffer.depth_at(x, y)
            );
            assert_eq!(
                parallel.framebuffer.stencil_at(x, y),
                scalar.framebuffer.stencil_at(x, y)
            );
        }
    }
}

#[test]
fn multisample_resolve_preserves_bgra_storage() {
    let mut renderer = Renderer::new(8, 8).unwrap();
    renderer.framebuffer = Framebuffer::with_format(8, 8, PixelFormat::Bgra8).unwrap();
    renderer.set_sample_count(SampleCount::Four).unwrap();
    render_msaa(&mut renderer).unwrap();
    assert_eq!(
        renderer.framebuffer.pixel(3, 3).unwrap().rgba8(),
        [255, 0, 0, 255]
    );
    assert_eq!(
        &renderer.framebuffer.bytes()[3 * 8 * 4 + 3 * 4..][..4],
        &[0, 0, 255, 255]
    );
}
