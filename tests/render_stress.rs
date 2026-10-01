use silicon::*;

#[test]
fn tile_and_overdraw_workloads_match_parallel_rendering() {
    let (width, height) = (64, 48);
    for (scene, triangles) in [("tile_stress", 12), ("overdraw", 256)] {
        let expected = demo::render(scene, width, height, 0.).unwrap();
        assert_eq!(expected.stats.triangles, triangles, "{scene}");
        if scene == "overdraw" {
            assert_eq!(expected.stats.shaded, u64::from(width * height));
            assert!(expected.stats.early_z_rejected >= u64::from(width * height) * 127);
            assert_eq!(
                expected
                    .framebuffer
                    .pixel(width / 2, height / 2)
                    .unwrap()
                    .rgba8(),
                [242, 31, 20, 255]
            );
        }

        for backend in [Backend::Scalar, Backend::Simd] {
            let mut parallel = Renderer::new(width, height).unwrap();
            parallel.backend = backend;
            parallel
                .render_bands_shared_vertices(4, |band| demo::render_into(band, scene, 0.))
                .unwrap();
            assert_eq!(
                parallel.framebuffer.bytes(),
                expected.framebuffer.bytes(),
                "{scene}/{backend:?}"
            );
            assert_eq!(parallel.stats.triangles, expected.stats.triangles);
            assert_eq!(
                parallel.stats.vertex_shader_invocations,
                expected.stats.vertex_shader_invocations
            );
        }
    }
}
