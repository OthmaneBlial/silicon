// Adapted from KhronosGroup/Vulkan-Samples hello_triangle at
// 177edebf0cd7d4f669667e49f052cfb56b17e004. Original sample: Copyright (c)
// 2018-2026 Arm Limited and Contributors, and (c) 2025-2026 Sascha Willems.
// SPDX-License-Identifier: Apache-2.0. See docs/third-party-demo.md.
use silicon::{api, vulkan_like as vk};

fn main() -> api::Result<()> {
    let physical = vk::Instance::new().enumerate_physical_devices()[0];
    let mut device = physical.create_device(320, 240)?;
    let vertex = device
        .create_shader_module(include_bytes!(
            "../assets/shaders/khronos_hello_triangle.vert.spv"
        ))
        .expect("adapted Khronos vertex shader is valid");
    let fragment = device
        .create_shader_module(include_bytes!(
            "../assets/shaders/khronos_hello_triangle.frag.spv"
        ))
        .expect("Khronos fragment shader is valid");
    let pipeline = device
        .create_graphics_pipeline(&vertex, &fragment, api::Pipeline::default())
        .expect("Khronos vertex and fragment interfaces match");

    // Preserve the upstream sample's non-indexed positions and RGB values;
    // append alpha 1 to each color for SILICON's fixed vertex layout.
    let vertices = device.create_vertex_buffer(vec![
        vertex_at(0.5, -0.5, 0.5, [1., 0., 0.]),
        vertex_at(0.5, 0.5, 0.5, [0., 1., 0.]),
        vertex_at(-0.5, 0.5, 0.5, [0., 0., 1.]),
    ])?;

    let mut commands = device.create_command_buffer();
    commands.begin_render_pass(device.create_render_pass(api::Color::BLACK));
    commands.bind_pipeline(&pipeline);
    commands.bind_vertex_buffer(&vertices).unwrap();
    commands.draw(0, 3);
    commands.end_render_pass();
    commands.validate()?;
    let submission = device.graphics_queue().submit(&commands)?;

    std::fs::create_dir_all("output")?;
    device
        .framebuffer()
        .save_png("output/khronos_hello_triangle.png")?;
    println!("Submitted {} Khronos sample draw", submission.draws);
    Ok(())
}

fn vertex_at(x: f32, y: f32, z: f32, color: [f32; 3]) -> api::Vertex {
    api::Vertex {
        position: api::Vec3::new(x, y, z),
        normal: api::Vec3::new(0., 0., 1.),
        uv: api::Vec2::ZERO,
        color: api::Vec4::new(color[0], color[1], color[2], 1.),
    }
}
