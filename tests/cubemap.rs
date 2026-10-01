use silicon::*;

#[test]
fn cubemap_reflection_scene_matches_scalar_and_parallel_rendering() {
    let reference = demo::render("cubemap_showcase", 240, 160, 0.3).unwrap();
    assert_ne!(
        reference.framebuffer.pixel(0, 0),
        reference.framebuffer.pixel(0, 159)
    );
    assert!(reference.stats.shaded > 10_000);

    let mut parallel = Renderer::new(240, 160).unwrap();
    parallel.backend = Backend::Simd;
    parallel
        .render_bands(4, |band| demo::render_into(band, "cubemap_showcase", 0.3))
        .unwrap();
    assert_eq!(parallel.framebuffer.bytes(), reference.framebuffer.bytes());
}

#[test]
fn implicit_cube_map_spirv_uses_direction_mips_in_scalar_and_simd() {
    let device = Device;
    let vertex = device
        .create_shader(include_bytes!(
            "../assets/shaders/cubemap_implicit.vert.spv"
        ))
        .unwrap();
    let fragment = device
        .create_shader(include_bytes!(
            "../assets/shaders/cubemap_implicit.frag.spv"
        ))
        .unwrap();
    let pipeline = device
        .create_pipeline(&vertex, &fragment, Pipeline::default())
        .unwrap();
    let vertices = [
        Vertex::new(Vec3::new(-1., -1., 0.5), Color::WHITE),
        Vertex::new(Vec3::new(3., -1., 0.5), Color::WHITE),
        Vertex::new(Vec3::new(-1., 3., 0.5), Color::WHITE),
    ];
    let colors = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 0, 255],
        [255, 0, 255, 255],
        [0, 255, 255, 255],
    ];
    let faces = colors
        .into_iter()
        .enumerate()
        .map(|(face, color)| {
            let pixels = if face == 4 {
                let mut pixels = Vec::with_capacity(16 * 16 * 4);
                for y in 0..16 {
                    for x in 0..16 {
                        pixels.extend(if (x + y) % 2 == 0 {
                            [255, 0, 0, 255]
                        } else {
                            [0, 0, 255, 255]
                        });
                    }
                }
                pixels
            } else {
                color.repeat(16 * 16)
            };
            let mut texture = Texture::new(16, 16, TextureFormat::Rgba8, &pixels).unwrap();
            texture.generate_mips();
            texture
        })
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let cube_map = std::sync::Arc::new(CubeMap::new(faces).unwrap());
    let mut commands = device.commands();
    commands.begin_render_pass(Color::BLACK);
    commands.bind_pipeline(pipeline);
    commands.bind_vertex_buffer(device.create_vertex_buffer(vertices.to_vec()).unwrap());
    commands.bind_cube_map(
        0,
        cube_map,
        Sampler {
            filter: Filter::Nearest,
            mip: MipFilter::Nearest,
            ..Default::default()
        },
    );
    commands.draw(0, 3);
    commands.end_render_pass();

    let mut images = Vec::new();
    for backend in [Backend::Scalar, Backend::Simd] {
        let mut renderer = Renderer::new(16, 16).unwrap();
        renderer.backend = backend;
        device.submit(&commands, &mut renderer).unwrap();
        assert!(renderer.stats.texture_samples > 0);
        let center = (8 * 16 + 8) * 4;
        assert_eq!(
            &renderer.framebuffer.bytes()[center..center + 4],
            &[128, 0, 128, 255]
        );
        images.push(renderer.framebuffer.bytes().to_vec());
    }
    assert_eq!(images[0], images[1]);
}
