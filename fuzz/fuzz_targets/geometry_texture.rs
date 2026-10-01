#![no_main]

use libfuzzer_sys::fuzz_target;
use silicon_core::{
    Color, Pipeline, Renderer, Sampler, Texture, Texture3D, TextureArray, TextureFormat, Vec2,
    Vec3, Vec4, Vertex, VertexOutput,
};

fuzz_target!(|bytes: &[u8]| {
    let word = |offset| {
        u32::from_le_bytes(std::array::from_fn(|i| {
            bytes.get(offset + i).copied().unwrap_or_default()
        }))
    };
    let coordinate = |offset| (word(offset) as f32 / u32::MAX as f32) * 4. - 2.;
    let vertices: [Vertex; 3] = std::array::from_fn(|i| {
        let offset = i * 12;
        Vertex::new(
            Vec3::new(
                coordinate(offset),
                coordinate(offset + 4),
                coordinate(offset + 8),
            ),
            Color::WHITE,
        )
    });
    let mut renderer = Renderer::new(8, 8).unwrap();
    let _ = renderer.draw(
        &vertices,
        None,
        Pipeline::default(),
        |v| VertexOutput {
            position: v.position.extend(1.),
            varyings: [v.color, Vec4::ZERO, Vec4::ZERO, Vec4::ZERO],
        },
        |_| Some(Color::WHITE),
    );

    let texels: [u8; 16] = std::array::from_fn(|i| bytes.get(i).copied().unwrap_or_default());
    let texture = Texture::new(2, 2, TextureFormat::Rgba8, &texels).unwrap();
    let uv = Vec2::new(f32::from_bits(word(36)), f32::from_bits(word(40)));
    let _ = texture.sample(uv, f32::from_bits(word(44)), Sampler::default());
    let dx = Vec2::new(f32::from_bits(word(48)), f32::from_bits(word(52)));
    let dy = Vec2::new(f32::from_bits(word(56)), f32::from_bits(word(60)));
    let anisotropy = bytes.first().copied().unwrap_or(4) % 32;
    let _ = texture.sample_anisotropic(uv, dx, dy, Sampler::default(), anisotropy);

    let second_layer: [u8; 16] =
        std::array::from_fn(|i| bytes.get(i + 16).copied().unwrap_or_default());
    let layers = TextureArray::new(vec![
        texture.clone(),
        Texture::new(2, 2, TextureFormat::Rgba8, &second_layer).unwrap(),
    ])
    .unwrap();
    let _ = layers.sample(
        uv,
        f32::from_bits(word(64)),
        layers.lod(dx, dy),
        Sampler::default(),
    );

    let volume_bytes: [u8; 32] =
        std::array::from_fn(|i| bytes.get(i + 72).copied().unwrap_or_default());
    let mut volume = Texture3D::new(2, 2, 2, TextureFormat::Rgba8, &volume_bytes).unwrap();
    volume.generate_mips();
    let coordinate = Vec3::new(
        f32::from_bits(word(104)),
        f32::from_bits(word(108)),
        f32::from_bits(word(112)),
    );
    let volume_lod = volume.lod(
        Vec3::new(
            f32::from_bits(word(116)),
            f32::from_bits(word(120)),
            f32::from_bits(word(124)),
        ),
        Vec3::new(
            f32::from_bits(word(128)),
            f32::from_bits(word(132)),
            f32::from_bits(word(136)),
        ),
    );
    let _ = volume.sample(coordinate, volume_lod, Sampler::default());
});
