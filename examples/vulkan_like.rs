use silicon::{api, vulkan_like as vk};

fn main() -> api::Result<()> {
    let physical = vk::Instance::new().enumerate_physical_devices()[0];
    let mut device = physical.create_device(320, 240)?;
    let vertex_shader = device
        .create_shader_module(include_bytes!("../assets/shaders/textured.vert.spv"))
        .expect("valid bundled vertex shader");
    let fragment_shader = device
        .create_shader_module(include_bytes!("../assets/shaders/textured.frag.spv"))
        .expect("valid bundled fragment shader");
    let pipeline = device
        .create_graphics_pipeline(&vertex_shader, &fragment_shader, api::Pipeline::default())
        .expect("compatible bundled shaders");
    let vertices = device.create_vertex_buffer(vec![
        vertex_at(-0.8, -0.7, 0., 0.),
        vertex_at(0.8, -0.7, 1., 0.),
        vertex_at(0., 0.8, 0.5, 1.),
    ])?;
    let indices = device.create_index_buffer(vec![0, 1, 2])?;
    let uniforms = device.create_uniform_buffer(vec![
        api::Vec4::new(1., 0., 0., 0.),
        api::Vec4::new(0., 1., 0., 0.),
        api::Vec4::new(0., 0., 1., 0.),
        api::Vec4::new(0., 0., 0., 1.),
    ])?;
    let white = device.create_image_rgba8(1, 1, &[255, 255, 255, 255])?;
    let mut descriptors = device.create_descriptor_set();
    descriptors
        .bind_uniform_buffer(&uniforms)
        .expect("uniform binding has the correct buffer usage");
    descriptors.bind_image(0, &white);

    let mut commands = device.create_command_buffer();
    commands.begin_render_pass(device.create_render_pass(api::Color::new(0.025, 0.035, 0.055, 1.)));
    commands.bind_pipeline(&pipeline);
    commands.bind_vertex_buffer(&vertices).unwrap();
    commands.bind_index_buffer(&indices).unwrap();
    commands.bind_descriptor_set(&descriptors).unwrap();
    commands.draw_indexed(0, 3);
    commands.end_render_pass();
    commands.validate()?;
    let submission = device.graphics_queue().submit(&commands)?;

    std::fs::create_dir_all("output")?;
    device
        .framebuffer()
        .save_png("output/vulkan_like_triangle.png")?;
    println!("Submitted {} draw(s)", submission.draws);
    Ok(())
}

fn vertex_at(x: f32, y: f32, u: f32, v: f32) -> api::Vertex {
    api::Vertex {
        position: api::Vec3::new(x, y, 0.),
        normal: api::Vec3::new(0., 0., 1.),
        uv: api::Vec2::new(u, v),
        color: api::Vec4::new(1., 1., 1., 1.),
    }
}
