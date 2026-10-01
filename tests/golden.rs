use silicon::*;
#[test]
fn approved_shader_cube_pixels() {
    let fb = demo::shader_cube(96, 64, 0.)
        .unwrap()
        .replay()
        .unwrap()
        .framebuffer;
    let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!(
        "golden/shader_cube.png"
    )));
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!((info.width, info.height), (96, 64));
    assert_eq!(info.color_type, png::ColorType::Rgba);
    let differences: Vec<_> = fb
        .bytes()
        .iter()
        .zip(&bytes)
        .enumerate()
        .filter(|(_, (a, b))| a.abs_diff(**b) > 1)
        .collect();
    if !differences.is_empty() {
        let diff: Vec<u8> = fb
            .bytes()
            .iter()
            .zip(&bytes)
            .flat_map(|(actual, expected)| {
                let value = actual.abs_diff(*expected).saturating_mul(4);
                [value, value, value, 255]
            })
            .collect();
        std::fs::create_dir_all("output").unwrap();
        let mut encoder = png::Encoder::new(
            std::fs::File::create("output/shader_cube.diff.png").unwrap(),
            info.width,
            info.height,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&diff)
            .unwrap();
    }
    assert!(
        differences.is_empty(),
        "{} channels differ beyond one quantization step; see output/shader_cube.diff.png",
        differences.len()
    );
}
