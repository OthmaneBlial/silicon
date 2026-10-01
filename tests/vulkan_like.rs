use silicon::{api, vulkan_like as vk};

#[test]
fn compatibility_subset_submits_an_indexed_textured_draw() {
    let instance = vk::Instance::new();
    let physical_devices = instance.enumerate_physical_devices();
    assert_eq!(physical_devices.len(), 1);
    assert_eq!(physical_devices[0].name(), "SILICON CPU device");
    let mut device = physical_devices[0].create_device(32, 32).unwrap();
    let vertex_shader = device
        .create_shader_module(include_bytes!("../assets/shaders/textured.vert.spv"))
        .unwrap();
    let fragment_shader = device
        .create_shader_module(include_bytes!("../assets/shaders/textured.frag.spv"))
        .unwrap();
    let pipeline = device
        .create_graphics_pipeline(&vertex_shader, &fragment_shader, api::Pipeline::default())
        .unwrap();
    let vertices = device
        .create_vertex_buffer(vec![
            vertex_at(-0.8, -0.7, 0., 0.),
            vertex_at(0.8, -0.7, 1., 0.),
            vertex_at(0., 0.8, 0.5, 1.),
        ])
        .unwrap();
    let indices = device.create_index_buffer(vec![0, 1, 2]).unwrap();
    let uniforms = device
        .create_uniform_buffer(vec![
            api::Vec4::new(1., 0., 0., 0.),
            api::Vec4::new(0., 1., 0., 0.),
            api::Vec4::new(0., 0., 1., 0.),
            api::Vec4::new(0., 0., 0., 1.),
        ])
        .unwrap();
    let image = device
        .create_image_rgba8(1, 1, &[255, 255, 255, 255])
        .unwrap();
    let mut descriptors = device.create_descriptor_set();
    descriptors.bind_uniform_buffer(&uniforms).unwrap();
    descriptors.bind_image(0, &image);

    let mut commands = device.create_command_buffer();
    commands.begin_render_pass(device.create_render_pass(api::Color::BLACK));
    commands.bind_pipeline(&pipeline);
    assert!(commands.bind_vertex_buffer(&indices).is_err());
    commands.bind_vertex_buffer(&vertices).unwrap();
    commands.bind_index_buffer(&indices).unwrap();
    commands.bind_descriptor_set(&descriptors).unwrap();
    commands.draw_indexed(0, 3);
    commands.end_render_pass();
    commands.validate().unwrap();
    let submission = device.graphics_queue().submit(&commands).unwrap();

    assert_eq!(submission.draws, 1);
    assert!(
        device
            .framebuffer()
            .bytes()
            .chunks_exact(4)
            .any(|pixel| pixel != api::Color::BLACK.rgba8())
    );
}

#[test]
fn compatibility_subset_submits_four_fragment_color_targets() {
    let physical = vk::Instance::new().enumerate_physical_devices()[0];
    let mut device = physical.create_device(32, 32).unwrap();
    let vertex_shader = device
        .create_shader_module(include_bytes!("../assets/shaders/mrt.vert.spv"))
        .unwrap();
    let fragment_shader = device
        .create_shader_module(include_bytes!("../assets/shaders/mrt.frag.spv"))
        .unwrap();
    let pipeline = device
        .create_graphics_pipeline(&vertex_shader, &fragment_shader, api::Pipeline::default())
        .unwrap();
    let vertices = device
        .create_vertex_buffer(vec![
            vertex_at(-0.8, -0.7, 0., 0.),
            vertex_at(0.8, -0.7, 1., 0.),
            vertex_at(0., 0.8, 0.5, 1.),
        ])
        .unwrap();
    let clear = [api::Color::BLACK; 4];
    assert!(device.create_render_pass_with_colors(&[]).is_err());
    let render_pass = device.create_render_pass_with_colors(&clear).unwrap();
    let mut commands = device.create_command_buffer();
    commands.begin_render_pass(render_pass);
    commands.bind_pipeline(&pipeline);
    commands.bind_vertex_buffer(&vertices).unwrap();
    commands.draw(0, 3);
    commands.end_render_pass();
    let submission = device.graphics_queue().submit(&commands).unwrap();

    assert_eq!(submission.draws, 1);
    let expected = [
        [64, 128, 191, 255],
        [191, 128, 64, 255],
        [0, 255, 0, 255],
        [255, 0, 255, 255],
    ];
    for (attachment, color) in expected.into_iter().enumerate() {
        assert_eq!(
            device
                .framebuffer()
                .color_attachment_pixel(attachment, 16, 16)
                .unwrap()
                .rgba8(),
            color
        );
    }
}

fn vertex_at(x: f32, y: f32, u: f32, v: f32) -> api::Vertex {
    api::Vertex {
        position: api::Vec3::new(x, y, 0.),
        normal: api::Vec3::new(0., 0., 1.),
        uv: api::Vec2::new(u, v),
        color: api::Vec4::new(1., 1., 1., 1.),
    }
}
