use silicon::*;

#[test]
fn portal_stencil_constrains_alpha_blending_across_scalar_and_bands() {
    let time = 0.37;
    let reference = demo::render("stencil", 240, 160, time).unwrap();
    let clear = reference.framebuffer.pixel(0, 0).unwrap();
    assert_eq!(reference.framebuffer.stencil_at(120, 80), Some(1));
    assert_eq!(reference.framebuffer.stencil_at(50, 115), Some(0));
    assert_eq!(reference.framebuffer.pixel(50, 115), Some(clear));
    assert_ne!(reference.framebuffer.pixel(120, 115), Some(clear));
    assert!(reference.stats.stencil_rejected > 0);

    let mut parallel = Renderer::new(240, 160).unwrap();
    parallel.backend = Backend::Simd;
    parallel
        .render_bands(4, |band| demo::render_into(band, "stencil", time))
        .unwrap();
    assert_eq!(parallel.framebuffer.bytes(), reference.framebuffer.bytes());
    for y in 0..160 {
        for x in 0..240 {
            assert_eq!(
                parallel.framebuffer.depth_at(x, y),
                reference.framebuffer.depth_at(x, y)
            );
            assert_eq!(
                parallel.framebuffer.stencil_at(x, y),
                reference.framebuffer.stencil_at(x, y)
            );
        }
    }
}
