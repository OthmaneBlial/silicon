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
    assert!(
        differences.is_empty(),
        "{} channels differ beyond one quantization step",
        differences.len()
    );
}
