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
