use silicon::{api, vulkan_like as vk};
use std::collections::HashSet;

#[test]
fn khronos_vulkan_hello_triangle_renders_interpolated_rgb() {
    let physical = vk::Instance::new().enumerate_physical_devices()[0];
    let mut device = physical.create_device(96, 96).unwrap();
    let vertex = device
        .create_shader_module(include_bytes!(
            "../assets/shaders/khronos_hello_triangle.vert.spv"
        ))
        .unwrap();
    let fragment = device
        .create_shader_module(include_bytes!(
            "../assets/shaders/khronos_hello_triangle.frag.spv"
        ))
        .unwrap();
    let pipeline = device
        .create_graphics_pipeline(&vertex, &fragment, api::Pipeline::default())
        .unwrap();
    let vertices = device
        .create_vertex_buffer(vec![
            vertex_at(0.5, -0.5, 0.5, [1., 0., 0.]),
            vertex_at(0.5, 0.5, 0.5, [0., 1., 0.]),
            vertex_at(-0.5, 0.5, 0.5, [0., 0., 1.]),
        ])
        .unwrap();
    let mut commands = device.create_command_buffer();
    commands.begin_render_pass(device.create_render_pass(api::Color::BLACK));
    commands.bind_pipeline(&pipeline);
    commands.bind_vertex_buffer(&vertices).unwrap();
    commands.draw(0, 3);
    commands.end_render_pass();
    let submission = device.graphics_queue().submit(&commands).unwrap();

    assert_eq!(submission.draws, 1);
    let mut colors = HashSet::new();
    let mut max_rgb = [0; 3];
    for pixel in device.framebuffer().bytes().chunks_exact(4) {
        if pixel[..3] != [0, 0, 0] {
            colors.insert([pixel[0], pixel[1], pixel[2]]);
            for channel in 0..3 {
                max_rgb[channel] = max_rgb[channel].max(pixel[channel]);
            }
        }
    }
    assert!(colors.len() > 100, "the fragment color should interpolate");
    assert!(max_rgb.into_iter().all(|channel| channel > 100));
}

fn vertex_at(x: f32, y: f32, z: f32, color: [f32; 3]) -> api::Vertex {
    api::Vertex {
        position: api::Vec3::new(x, y, z),
        normal: api::Vec3::new(0., 0., 1.),
        uv: api::Vec2::ZERO,
        color: api::Vec4::new(color[0], color[1], color[2], 1.),
    }
}
