//! Reproducible CPU-only scenes. Native Rust closures are the first shader backend.
use crate::*;
use std::sync::OnceLock;
#[derive(Clone, Copy, Debug)]
pub struct Material {
    pub color: Color,
    pub textured: bool,
    pub metallic: f32,
    pub roughness: f32,
    pub emission: f32,
    pub normal_map_strength: f32,
}
impl Material {
    pub fn matte(color: Color) -> Self {
        Self {
            color,
            textured: true,
            metallic: 0.1,
            roughness: 0.82,
            emission: 0.,
            normal_map_strength: 0.,
        }
    }
}
pub fn render(name: &str, width: u32, height: u32, time: f32) -> Result<Renderer> {
    let mut r = Renderer::new(width, height)?;
    render_into(&mut r, name, time)?;
    Ok(r)
}
pub fn render_into(r: &mut Renderer, name: &str, time: f32) -> Result<()> {
    if !time.is_finite() {
        return Err("scene time must be finite".into());
    }
    if name == "anisotropy_showcase" {
        return anisotropy_showcase(r);
    }
    if name == "shadow_showcase" {
        let (width, height) = r.surface_size();
        let (capture, shadow_stats) = shadow_showcase(width, height, time)?;
        Device.submit(&capture.commands, r)?;
        r.stats += &shadow_stats;
        return Ok(());
    }
    if name == "stencil" {
        return stencil_showcase(r, time);
    }
    if matches!(
        name,
        "shader_cube" | "spirv_cube" | "spirv_showcase" | "spirv_cutout" | "pbr_showcase"
    ) {
        let (width, height) = r.surface_size();
        let capture = match name {
            "spirv_showcase" => spirv_showcase(width, height, time)?,
            "pbr_showcase" => pbr_showcase(width, height, time)?,
            "spirv_cube" => spirv_cube(width, height, time)?,
            "spirv_cutout" => spirv_cutout(width, height, time)?,
            _ => shader_cube(width, height, time)?,
        };
        Device.submit(&capture.commands, r)?;
        return Ok(());
    }
    r.clear(Color::new(0.022, 0.032, 0.05, 1.));
    static TEXTURE: OnceLock<Texture> = OnceLock::new();
    let texture = TEXTURE.get_or_init(|| Texture::checker(128).expect("valid built-in checker"));
    let eye = if matches!(name, "showcase" | "cubemap_showcase") {
        if name == "cubemap_showcase" {
            Vec3::new(7.5, 4.6, 10.)
        } else {
            Vec3::new(7.5, 5.8, 10.)
        }
    } else {
        Vec3::new(4., 3., 5.)
    };
    let view = Mat4::look_at(
        eye,
        if matches!(name, "showcase" | "cubemap_showcase") {
            Vec3::new(0., 1.2, 0.)
        } else {
            Vec3::ZERO
        },
        Vec3::new(0., 1., 0.),
    );
    let proj = Mat4::perspective(
        0.78,
        r.surface_size().0 as f32 / r.surface_size().1 as f32,
        0.1,
        60.,
    );
    let environment = if name == "cubemap_showcase" {
        static ENVIRONMENT: OnceLock<CubeMap> = OnceLock::new();
        Some(ENVIRONMENT.get_or_init(|| environment_cubemap().expect("valid built-in cubemap")))
    } else {
        None
    };
    if let Some(environment) = environment {
        let sky = Mesh::cube();
        let model = Mat4::translation(eye) * Mat4::scale(Vec3::new(25., 25., 25.));
        let mvp = proj * view * model;
        let display = |v: f32| (v.max(0.) / (1. + v.max(0.))).powf(1. / 2.2);
        r.draw(
            &sky.vertices,
            Some(&sky.indices),
            Pipeline {
                cull: Cull::None,
                depth_compare: Compare::LessEqual,
                ..Default::default()
            },
            |v| {
                let world = model.transform(v.position.extend(1.));
                VertexOutput {
                    position: mvp.transform(v.position.extend(1.)),
                    varyings: [world, Vec4::ZERO, Vec4::ZERO, Vec4::ZERO],
                }
            },
            |f| {
                let direction = (f.varyings[0].xyz() - eye).normalize();
                let color = environment
                    .sample(
                        direction,
                        0.,
                        Sampler {
                            mip: MipFilter::None,
                            ..Default::default()
                        },
                    )
                    .expect("finite sky direction")
                    .0;
                Some(Color::new(
                    display(color.x),
                    display(color.y),
                    display(color.z),
                    1.,
                ))
            },
        )?;
    }
    let draw = |mesh: &Mesh, model: Mat4, mat: Material, blend: Blend| -> Result<()> {
        let mvp = proj * view * model;
        let normal = Mat3::normal_matrix(model).ok_or("singular model transform")?;
        let pipeline = Pipeline {
            cull: Cull::Back,
            blend,
            depth_write: blend == Blend::Replace,
            ..Default::default()
        };
        r.draw(
            &mesh.vertices,
            Some(&mesh.indices),
            pipeline,
            |v| {
                let world = model.transform(v.position.extend(1.));
                VertexOutput {
                    position: mvp.transform(v.position.extend(1.)),
                    varyings: [
                        v.color,
                        Vec4::new(v.uv.x, v.uv.y, 0., 0.),
                        normal.transform(v.normal).extend(0.),
                        world,
                    ],
                }
            },
            |f| {
                let tex = if mat.textured && environment.is_none() {
                    texture
                        .sample(f.uv(), texture.lod(f.uv_dx, f.uv_dy), Sampler::default())
                        .expect("finite raster UV")
                        .0
                } else {
                    Color::WHITE.0
                };
                let base = mat.color.0.component_mul(tex).component_mul(f.color().0);
                let n = f.normal();
                let world = f.world();
                let light = Vec3::new(-0.4, 0.85, 0.6).normalize();
                let diffuse = n.dot(light).max(0.);
                let view = (eye - world).normalize();
                let spec = n.dot((view + light).normalize()).max(0.).powf(64.)
                    * (0.35 + mat.metallic * 1.5);
                let delta = Vec3::new(2., 3., -2.) - world;
                let point = n.dot(delta.normalize()).max(0.) * 5. / (1. + delta.dot(delta));
                let ambient = 0.16 + 0.12 * n.y.max(0.);
                let rgb = base.xyz() * (ambient + diffuse * 0.85 + mat.emission)
                    + Vec3::new(1., 0.86, 0.68) * spec
                    + Vec3::new(0.08, 0.65, 0.95) * point;
                let rgb = if let Some(environment) = environment {
                    let incident = (world - eye).normalize();
                    let reflected = incident - n * (2. * incident.dot(n));
                    let env = environment
                        .sample(
                            reflected,
                            mat.roughness.clamp(0., 1.) * (environment.mip_levels() - 1) as f32,
                            Sampler::default(),
                        )
                        .expect("finite reflection direction")
                        .0
                        .xyz();
                    rgb.lerp(env, 0.12 + mat.metallic.clamp(0., 1.) * 0.68)
                } else {
                    rgb
                };
                let fog = (world - eye).length() / 45.;
                let rgb = rgb.lerp(Vec3::new(0.022, 0.032, 0.05), fog.clamp(0., 0.7));
                // A display transfer is part of this demo shader, not the framebuffer.
                let display = |v: f32| {
                    let v = v.max(0.);
                    (v / (1. + v)).powf(1. / 2.2)
                };
                Some(Color::new(
                    display(rgb.x),
                    display(rgb.y),
                    display(rgb.z),
                    base.w,
                ))
            },
        )
    };
    visit_scene(
        if name == "cubemap_showcase" {
            "showcase"
        } else {
            name
        },
        time,
        draw,
    )
}

fn anisotropy_showcase(r: &mut Renderer) -> Result<()> {
    static TEXTURE: OnceLock<Texture> = OnceLock::new();
    let texture = TEXTURE.get_or_init(|| {
        let mut pixels = Vec::with_capacity(128 * 128 * 4);
        for _ in 0..128 {
            for x in 0usize..128 {
                pixels.extend(if (x / 8).is_multiple_of(2) {
                    [32, 194, 224, 255]
                } else {
                    [244, 166, 50, 255]
                });
            }
        }
        let mut texture = Texture::new(128, 128, TextureFormat::Rgba8, &pixels)
            .expect("valid anisotropy texture");
        texture.generate_mips();
        texture
    });
    let (width, height) = r.surface_size();
    let view = Mat4::look_at(
        Vec3::new(0., 7., 9.),
        Vec3::new(0., 0., -12.),
        Vec3::new(0., 1., 0.),
    );
    let projection = Mat4::perspective(0.88, width as f32 / height as f32, 0.1, 60.);
    let mvp = projection * view;
    r.clear(Color::new(0.018, 0.026, 0.04, 1.));
    for (min_x, max_x, anisotropic) in [(-10., 0., false), (0., 10., true)] {
        let vertices = [
            (min_x, 1., 2., 0., 0.),
            (max_x, 1., 2., 8., 0.),
            (max_x, 1., -36., 8., 96.),
            (min_x, 1., -36., 0., 96.),
        ]
        .map(|(x, y, z, u, v)| Vertex {
            position: Vec3::new(x, y, z),
            normal: Vec3::new(0., 1., 0.),
            uv: Vec2::new(u, v),
            color: Color::WHITE.0,
        });
        r.try_draw(
            &vertices,
            Some(&[0, 1, 2, 0, 2, 3]),
            Pipeline {
                cull: Cull::None,
                ..Default::default()
            },
            |v| {
                Ok(VertexOutput {
                    position: mvp.transform(v.position.extend(1.)),
                    varyings: [
                        Vec4::new(1., 1., 1., 1.),
                        Vec4::new(v.uv.x, v.uv.y, 0., 0.),
                        v.normal.extend(0.),
                        v.position.extend(1.),
                    ],
                })
            },
            |f| {
                let sampler = Sampler::default();
                let color = if anisotropic {
                    texture.sample_anisotropic(f.uv(), f.uv_dx, f.uv_dy, sampler, MAX_ANISOTROPY)?
                } else {
                    texture.sample(f.uv(), texture.lod(f.uv_dx, f.uv_dy), sampler)?
                };
                Ok(Some(color))
            },
        )?;
    }
    Ok(())
}

fn environment_cubemap() -> Result<CubeMap> {
    let sun = Vec3::new(-0.45, 0.68, 0.58).normalize();
    let mut faces = Vec::with_capacity(6);
    for face in CubeFace::ALL {
        let mut pixels = Vec::with_capacity(128 * 128 * 4);
        for y in 0..128 {
            for x in 0..128 {
                let d =
                    CubeMap::face_direction(face, (x as f32 + 0.5) / 128., (y as f32 + 0.5) / 128.);
                let horizon = Vec3::new(0.92, 0.38, 0.16);
                let ground = Vec3::new(0.16, 0.09, 0.07);
                let sky = Vec3::new(0.015, 0.09, 0.34);
                let color = if d.y >= 0. {
                    horizon.lerp(sky, (d.y / 0.08).clamp(0., 1.))
                } else {
                    horizon.lerp(ground, (-d.y / 0.18).clamp(0., 1.))
                };
                let sun_disk = ((d.dot(sun) - 0.992) / 0.008).clamp(0., 1.);
                let color = color + Vec3::new(1., 0.58, 0.22) * sun_disk;
                pixels.extend([
                    (color.x.clamp(0., 1.) * 255.).round() as u8,
                    (color.y.clamp(0., 1.) * 255.).round() as u8,
                    (color.z.clamp(0., 1.) * 255.).round() as u8,
                    255,
                ]);
            }
        }
        let mut texture = Texture::new(128, 128, TextureFormat::Rgba8, &pixels)?;
        texture.generate_mips();
        faces.push(texture);
    }
    CubeMap::new(faces.try_into().expect("six cubemap faces"))
}

fn stencil_vertex(v: &Vertex) -> VertexOutput {
    VertexOutput {
        position: v.position.extend(1.),
        varyings: [
            v.color,
            Vec4::new(v.uv.x, v.uv.y, 0., 0.),
            Vec4::ZERO,
            Vec4::ZERO,
        ],
    }
}

fn stencil_showcase(r: &mut Renderer, time: f32) -> Result<()> {
    let (width, height) = r.surface_size();
    r.clear(Color::new(0.025, 0.035, 0.055, 1.));
    let stencil = StencilState {
        compare: Compare::Always,
        reference: 1,
        read_mask: 255,
        write_mask: 255,
        fail: StencilOp::Keep,
        depth_fail: StencilOp::Keep,
        pass: StencilOp::Replace,
    };
    let radius_y = 0.8;
    let radius_x = radius_y * height as f32 / width as f32;
    let mut mask = Vec::with_capacity(64 * 3);
    for i in 0..64 {
        let a = i as f32 * std::f32::consts::TAU / 64.;
        let b = (i + 1) as f32 * std::f32::consts::TAU / 64.;
        for (x, y) in [
            (0., 0.),
            (a.cos() * radius_x, a.sin() * radius_y),
            (b.cos() * radius_x, b.sin() * radius_y),
        ] {
            mask.push(Vertex::new(Vec3::new(x, y, 0.5), Color::WHITE));
        }
    }
    r.draw(
        &mask,
        None,
        Pipeline {
            color_write: false,
            depth_write: false,
            stencil: Some(stencil),
            ..Default::default()
        },
        stencil_vertex,
        |f| Some(f.color()),
    )?;

    static TEXTURE: OnceLock<Texture> = OnceLock::new();
    let texture = TEXTURE.get_or_init(|| Texture::checker(128).expect("valid checker texture"));
    let cube = Mesh::cube();
    let mvp = Mat4::perspective(0.85, width as f32 / height as f32, 0.1, 20.)
        * Mat4::look_at(Vec3::new(3., 2., 4.), Vec3::ZERO, Vec3::new(0., 1., 0.))
        * Mat4::rotation_y(time + 0.6);
    r.try_draw(
        &cube.vertices,
        Some(&cube.indices),
        Pipeline {
            stencil: Some(StencilState {
                compare: Compare::Equal,
                write_mask: 0,
                pass: StencilOp::Keep,
                ..stencil
            }),
            ..Default::default()
        },
        |v| {
            let mut out = stencil_vertex(v);
            out.position = mvp.transform(v.position.extend(1.));
            Ok(out)
        },
        |f| {
            Ok(Some(texture.sample(
                f.uv(),
                texture.lod(f.uv_dx, f.uv_dy),
                Sampler::default(),
            )?))
        },
    )?;

    let overlay = [
        Vertex::new(Vec3::new(-0.9, -0.6, 0.1), Color::new(1., 0.2, 0.1, 0.45)),
        Vertex::new(Vec3::new(0.9, -0.6, 0.1), Color::new(0.1, 1., 0.6, 0.45)),
        Vertex::new(Vec3::new(0., 0.7, 0.1), Color::new(0.2, 0.3, 1., 0.45)),
    ];
    r.draw(
        &overlay,
        None,
        Pipeline {
            blend: Blend::Alpha,
            depth_write: false,
            stencil: Some(StencilState {
                compare: Compare::Equal,
                write_mask: 0,
                pass: StencilOp::Keep,
                ..stencil
            }),
            ..Default::default()
        },
        stencil_vertex,
        |f| Some(f.color()),
    )
}

/// Both shader backends draw the same geometry, transforms and materials.
fn visit_scene(
    name: &str,
    time: f32,
    mut draw: impl FnMut(&Mesh, Mat4, Material, Blend) -> Result<()>,
) -> Result<()> {
    let cube = Mesh::cube();
    match name {
        "cube" | "textured_cube" | "triangle_3d" => {
            let mesh = if name == "triangle_3d" {
                Mesh {
                    vertices: vec![
                        Vertex::new(Vec3::new(-1., -1., 0.), Color::new(1., 0., 0., 1.)),
                        Vertex::new(Vec3::new(1., -1., 0.), Color::new(0., 1., 0., 1.)),
                        Vertex::new(Vec3::new(0., 1., 0.), Color::new(0., 0., 1., 1.)),
                    ],
                    indices: vec![0, 1, 2],
                }
            } else {
                cube
            };
            draw(
                &mesh,
                Mat4::rotation_y(time + 0.55) * Mat4::rotation_x(0.2),
                Material {
                    color: Color::WHITE,
                    textured: name == "textured_cube",
                    metallic: 0.2,
                    roughness: 0.62,
                    emission: 0.,
                    normal_map_strength: 0.,
                },
                Blend::Replace,
            )?;
        }
        "showcase" => {
            draw(
                &cube,
                Mat4::translation(Vec3::new(0., -0.28, 0.)) * Mat4::scale(Vec3::new(7., 0.25, 6.)),
                Material::matte(Color::new(0.32, 0.38, 0.45, 1.)),
                Blend::Replace,
            )?;
            // A real OBJ file exercises the same asset loader as user models.
            static SCULPTURE: OnceLock<Mesh> = OnceLock::new();
            let sculpture = SCULPTURE.get_or_init(|| {
                Mesh::from_obj(include_str!("../../../assets/models/sculpture.obj"))
                    .expect("tested built-in OBJ")
            });
            draw(
                sculpture,
                Mat4::translation(Vec3::new(0., 2.2, 0.))
                    * Mat4::rotation_y(time * 0.4)
                    * Mat4::rotation_x(1.15),
                Material {
                    color: Color::new(0.95, 0.48, 0.16, 1.),
                    textured: true,
                    metallic: 0.8,
                    roughness: 0.24,
                    emission: 0.,
                    normal_map_strength: 0.55,
                },
                Blend::Replace,
            )?;
            for i in 0..12 {
                let a = i as f32 * std::f32::consts::TAU / 12.;
                let (s, c) = a.sin_cos();
                let height = 0.45 + (i as f32 * 1.8).sin().abs() * 0.8;
                let pos = Vec3::new(c * 3.3, height, s * 3.3);
                draw(
                    &cube,
                    Mat4::translation(pos)
                        * Mat4::rotation_y(a + time * 0.1)
                        * Mat4::scale(Vec3::new(0.32, height, 0.32)),
                    Material::matte(Color::new(0.24, 0.5, 0.65, 1.)),
                    Blend::Replace,
                )?;
                draw(
                    &cube,
                    Mat4::translation(pos + Vec3::new(0., height + 0.12, 0.))
                        * Mat4::rotation_y(a)
                        * Mat4::scale(Vec3::new(0.35, 0.1, 0.35)),
                    Material {
                        color: Color::new(0.1, 0.8, 1., 1.),
                        textured: false,
                        metallic: 0.4,
                        roughness: 0.32,
                        emission: 1.5,
                        normal_map_strength: 0.,
                    },
                    Blend::Replace,
                )?;
            }
            static RING: OnceLock<Mesh> = OnceLock::new();
            let ring =
                RING.get_or_init(|| Mesh::torus(64, 12, 2.4, 0.06).expect("valid built-in torus"));
            draw(
                ring,
                Mat4::translation(Vec3::new(0., 0.15, 0.)),
                Material {
                    color: Color::new(0.1, 0.85, 1., 1.),
                    textured: false,
                    metallic: 0.1,
                    roughness: 0.42,
                    emission: 1.,
                    normal_map_strength: 0.,
                },
                Blend::Replace,
            )?;
            static SATELLITE: OnceLock<Mesh> = OnceLock::new();
            let satellite = SATELLITE
                .get_or_init(|| Mesh::torus(48, 16, 0.5, 0.17).expect("valid built-in torus"));
            for i in 0..3 {
                let a = i as f32 * 2.094 + time * 0.2;
                draw(
                    satellite,
                    Mat4::translation(Vec3::new(
                        a.cos() * 2.1,
                        2.5 + (a * 1.2).sin() * 0.4,
                        a.sin() * 2.1,
                    )) * Mat4::rotation_x(a),
                    Material {
                        color: Color::new(0.75, 0.83, 0.9, 1.),
                        textured: false,
                        metallic: 1.,
                        roughness: 0.12,
                        emission: 0.,
                        normal_map_strength: 0.,
                    },
                    Blend::Replace,
                )?;
            }
        }
        _ => {
            return Err(format!(
                "unknown scene {name}; choose cube, textured_cube, triangle_3d, showcase, pbr_showcase or anisotropy_showcase"
            )
            .into());
        }
    }
    Ok(())
}
/// A fully recorded SIR draw: geometry, uniforms, textures, pipeline and shader bytecode.
pub fn shader_cube(width: u32, height: u32, time: f32) -> Result<FrameCapture> {
    use shader::{Instruction::*, Program};
    let vertex = Program::new(vec![
        Input { dst: 0, slot: 0 },
        Mat4 {
            dst: 1,
            src: 0,
            uniform: 0,
        },
        Output { slot: 0, src: 1 },
        Input { dst: 2, slot: 1 },
        Output { slot: 1, src: 2 },
        Input { dst: 3, slot: 2 },
        Output { slot: 2, src: 3 },
        Input { dst: 4, slot: 3 },
        Mat4 {
            dst: 5,
            src: 4,
            uniform: 4,
        },
        Output { slot: 3, src: 5 },
    ])?;
    let fragment = Program::new(vec![
        Input { dst: 0, slot: 1 },
        Sample {
            dst: 1,
            uv: 0,
            texture: 0,
        },
        Input { dst: 2, slot: 2 },
        Normalize3 { dst: 2, src: 2 },
        Const {
            dst: 3,
            value: Vec3::new(-0.4, 0.85, 0.6).normalize().extend(0.),
        },
        Dot3 { dst: 4, a: 2, b: 3 },
        Saturate { dst: 4, src: 4 },
        Const {
            dst: 5,
            value: Vec4::new(0.2, 0.2, 0.2, 0.2),
        },
        Add { dst: 4, a: 4, b: 5 },
        Mul { dst: 6, a: 1, b: 4 },
        Const {
            dst: 7,
            value: Vec4::new(1., 1., 1., 0.),
        },
        Mul { dst: 6, a: 6, b: 7 },
        Const {
            dst: 8,
            value: Vec4::new(0., 0., 0., 1.),
        },
        Add { dst: 6, a: 6, b: 8 },
        Output { slot: 0, src: 6 },
    ])?;
    shader_cube_with_programs(width, height, time, vertex, fragment)
}
/// The same cube/resources, using caller-supplied vertex and fragment programs.
pub fn shader_cube_with_programs(
    width: u32,
    height: u32,
    time: f32,
    vertex: shader::Program,
    fragment: shader::Program,
) -> Result<FrameCapture> {
    use std::sync::Arc;
    shader_cube_with_pipeline(
        width,
        height,
        time,
        Arc::new(ShaderPipeline {
            state: Pipeline {
                cull: Cull::Back,
                ..Default::default()
            },
            vertex,
            fragment,
        }),
    )
}
/// The same cube/resources, using a prebuilt pipeline object.
pub fn shader_cube_with_pipeline(
    width: u32,
    height: u32,
    time: f32,
    pipeline: std::sync::Arc<ShaderPipeline>,
) -> Result<FrameCapture> {
    use std::sync::Arc;
    if !time.is_finite() {
        return Err("scene time must be finite".into());
    }
    Framebuffer::new(width, height)?;
    let device = Device;
    let mesh = Mesh::cube();
    let model = crate::Mat4::rotation_y(time + 0.55) * crate::Mat4::rotation_x(0.2);
    let mvp = crate::Mat4::perspective(0.78, width as f32 / height as f32, 0.1, 60.)
        * crate::Mat4::look_at(Vec3::new(4., 3., 5.), Vec3::ZERO, Vec3::new(0., 1., 0.))
        * model;
    let uniforms = lighting_uniforms(
        mvp,
        model,
        Material {
            color: Color::WHITE,
            textured: true,
            metallic: 0.2,
            roughness: 0.62,
            emission: 0.,
            normal_map_strength: 0.,
        },
        Vec3::new(4., 3., 5.),
    )?;
    let mut commands = device.commands();
    commands.begin_render_pass(Color::new(0.022, 0.032, 0.05, 1.));
    commands.bind_pipeline(pipeline);
    commands.bind_vertex_buffer(device.create_vertex_buffer(mesh.vertices)?);
    commands.bind_index_buffer(device.create_index_buffer(mesh.indices.clone())?);
    commands.bind_uniform_buffer(device.create_uniform_buffer(uniforms)?);
    commands.bind_texture(0, Arc::new(Texture::checker(128)?), Sampler::default());
    commands.draw_indexed(0, mesh.indices.len() as u32);
    commands.end_render_pass();
    Ok(FrameCapture {
        version: 1,
        width,
        height,
        commands,
    })
}

/// GLSL compiled externally by glslang, then translated and executed by SILICON.
pub fn spirv_cube(width: u32, height: u32, time: f32) -> Result<FrameCapture> {
    use std::sync::Arc;
    static PIPELINE: OnceLock<shader::Result<Arc<ShaderPipeline>>> = OnceLock::new();
    let pipeline = PIPELINE
        .get_or_init(|| {
            ShaderPipeline::from_spirv(
                include_bytes!("../../../assets/shaders/textured.vert.spv"),
                include_bytes!("../../../assets/shaders/textured.frag.spv"),
                Pipeline {
                    cull: Cull::Back,
                    ..Default::default()
                },
            )
        })
        .as_ref()
        .map_err(|e| e.clone())?;
    shader_cube_with_pipeline(width, height, time, Arc::clone(pipeline))
}

/// Nested GLSL discard, conditional sampling, Phi merge and early return on a textured cube.
pub fn spirv_cutout(width: u32, height: u32, time: f32) -> Result<FrameCapture> {
    use std::sync::Arc;
    static PIPELINE: OnceLock<shader::Result<Arc<ShaderPipeline>>> = OnceLock::new();
    let pipeline = PIPELINE
        .get_or_init(|| {
            ShaderPipeline::from_spirv(
                include_bytes!("../../../assets/shaders/textured.vert.spv"),
                include_bytes!("../../../assets/shaders/control.ssa.frag.spv"),
                Pipeline {
                    cull: Cull::Back,
                    ..Default::default()
                },
            )
        })
        .as_ref()
        .map_err(|e| e.clone())?;
    shader_cube_with_pipeline(width, height, time, Arc::clone(pipeline))
}
fn compile_graphics(
    vertex: &[u8],
    fragment: &[u8],
) -> shader::Result<(shader::Program, shader::Program)> {
    use shader::spirv::{Module, link};
    let vertex = Module::parse(vertex)?.translate()?;
    let fragment = Module::parse(fragment)?.translate()?;
    link(&vertex, &fragment)?;
    Ok((vertex.program, fragment.program))
}

fn lighting_uniforms(mvp: Mat4, model: Mat4, material: Material, eye: Vec3) -> Result<Vec<Vec4>> {
    let normal = Mat3::normal_matrix(model).ok_or("singular model transform")?;
    let mut uniforms = vec![Vec4::ZERO; 24];
    for i in 0..4 {
        uniforms[i] = Vec4::from_array(mvp.0[i]);
        uniforms[i + 4] = Vec4::from_array(model.0[i]);
    }
    for i in 0..3 {
        uniforms[i + 8] = Vec3::new(normal.0[i][0], normal.0[i][1], normal.0[i][2]).extend(0.);
    }
    uniforms[11] = Vec4::new(0., 0., 0., 1.);
    uniforms[12] = material.color.0;
    uniforms[16] = Vec4::new(
        if material.textured { 1. } else { 0. },
        material.metallic,
        material.emission,
        material.roughness,
    );
    uniforms[20] = eye.extend(1.);
    Ok(uniforms)
}

fn tangent_vertices(mesh: &Mesh) -> Result<Vec<Vertex>> {
    if mesh.indices.is_empty() || !mesh.indices.len().is_multiple_of(3) {
        return Err("tangent generation requires indexed triangles".into());
    }
    let mut tangents = vec![Vec3::ZERO; mesh.vertices.len()];
    let mut bitangents = tangents.clone();
    for triangle in mesh.indices.chunks_exact(3) {
        let [i0, i1, i2] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let [Some(a), Some(b), Some(c)] = [
            mesh.vertices.get(i0),
            mesh.vertices.get(i1),
            mesh.vertices.get(i2),
        ] else {
            return Err("mesh index exceeds vertex buffer".into());
        };
        let edge1 = b.position - a.position;
        let edge2 = c.position - a.position;
        let uv1 = b.uv - a.uv;
        let uv2 = c.uv - a.uv;
        let determinant = uv1.x * uv2.y - uv1.y * uv2.x;
        if !determinant.is_finite() || determinant.abs() <= 1e-10 {
            continue;
        }
        let tangent = (edge1 * uv2.y - edge2 * uv1.y) / determinant;
        let bitangent = (edge2 * uv1.x - edge1 * uv2.x) / determinant;
        if !tangent.is_finite() || !bitangent.is_finite() {
            return Err("non-finite mesh tangent".into());
        }
        for index in [i0, i1, i2] {
            tangents[index] = tangents[index] + tangent;
            bitangents[index] = bitangents[index] + bitangent;
        }
    }
    let mut vertices = mesh.vertices.clone();
    for (i, vertex) in vertices.iter_mut().enumerate() {
        let normal = vertex.normal.normalize();
        let normal = if normal == Vec3::ZERO {
            Vec3::new(0., 0., 1.)
        } else {
            normal
        };
        let tangent = (tangents[i] - normal * normal.dot(tangents[i])).normalize();
        let tangent = if tangent == Vec3::ZERO {
            if normal.z.abs() < 0.999 {
                Vec3::new(0., 0., 1.).cross(normal).normalize()
            } else {
                Vec3::new(0., 1., 0.).cross(normal).normalize()
            }
        } else {
            tangent
        };
        let sign = if normal.cross(tangent).dot(bitangents[i]) < 0. {
            -1.
        } else {
            1.
        };
        vertex.color = Vec4::new(tangent.x, tangent.y, tangent.z, sign);
    }
    Ok(vertices)
}

fn pbr_normal_map() -> std::sync::Arc<Texture> {
    static TEXTURE: OnceLock<std::sync::Arc<Texture>> = OnceLock::new();
    TEXTURE
        .get_or_init(|| {
            let mut bytes = Vec::with_capacity(128 * 128 * 4);
            for y in 0..128 {
                for x in 0..128 {
                    let u = x as f32 / 128.;
                    let v = y as f32 / 128.;
                    let (u_sin, u_cos) = (u * std::f32::consts::TAU * 3.).sin_cos();
                    let (v_sin, v_cos) = (v * std::f32::consts::TAU * 2.).sin_cos();
                    let normal =
                        Vec3::new(0.22 * u_cos * v_sin, 0.22 * u_sin * v_cos, 1.).normalize();
                    bytes.extend([
                        ((normal.x * 0.5 + 0.5) * 255.).round() as u8,
                        ((normal.y * 0.5 + 0.5) * 255.).round() as u8,
                        ((normal.z * 0.5 + 0.5) * 255.).round() as u8,
                        255,
                    ]);
                }
            }
            let mut texture = Texture::new(128, 128, TextureFormat::Rgba8, &bytes)
                .expect("valid built-in tangent-space normal map");
            texture.generate_mips();
            std::sync::Arc::new(texture)
        })
        .clone()
}
fn material_showcase(
    width: u32,
    height: u32,
    time: f32,
    programs: &(shader::Program, shader::Program),
    normal_map: Option<std::sync::Arc<Texture>>,
) -> Result<FrameCapture> {
    use std::sync::Arc;
    if !time.is_finite() {
        return Err("scene time must be finite".into());
    }
    Framebuffer::new(width, height)?;
    let device = Device;
    let mut commands = device.commands();
    commands.begin_render_pass(Color::new(0.022, 0.032, 0.05, 1.));
    commands.bind_pipeline(Arc::new(ShaderPipeline {
        state: Pipeline {
            cull: Cull::Back,
            ..Default::default()
        },
        vertex: programs.0.clone(),
        fragment: programs.1.clone(),
    }));
    commands.bind_texture(0, Arc::new(Texture::checker(128)?), Sampler::default());
    let eye = Vec3::new(7.5, 5.8, 10.);
    let vp = Mat4::perspective(0.78, width as f32 / height as f32, 0.1, 60.)
        * Mat4::look_at(eye, Vec3::new(0., 1.2, 0.), Vec3::new(0., 1., 0.));
    visit_scene("showcase", time, |mesh, model, material, blend| {
        if blend != Blend::Replace {
            return Err("GLSL showcase requires opaque draws".into());
        }
        if let Some(texture) = &normal_map {
            commands.bind_texture(
                1,
                texture.clone(),
                Sampler {
                    mip: if material.normal_map_strength > 0. {
                        MipFilter::Trilinear
                    } else {
                        MipFilter::None
                    },
                    ..Default::default()
                },
            );
        }
        let vertices = if normal_map.is_some() && material.normal_map_strength > 0. {
            tangent_vertices(mesh)?
        } else {
            mesh.vertices.clone()
        };
        commands.bind_vertex_buffer(device.create_vertex_buffer(vertices)?);
        commands.bind_index_buffer(device.create_index_buffer(mesh.indices.clone())?);
        let mut uniforms = lighting_uniforms(vp * model, model, material, eye)?;
        if normal_map.is_some() {
            uniforms.resize(36, Vec4::ZERO);
            uniforms[32] = Vec4::new(material.normal_map_strength, 0., 0., 0.);
        }
        commands.bind_uniform_buffer(device.create_uniform_buffer(uniforms)?);
        commands.draw_indexed(0, mesh.indices.len() as u32);
        Ok(())
    })?;
    commands.end_render_pass();
    Ok(FrameCapture {
        version: 1,
        width,
        height,
        commands,
    })
}
/// The lit OBJ showcase executes ordinary GLSL through SPIR-V, SIR and recorded draws.
pub fn spirv_showcase(width: u32, height: u32, time: f32) -> Result<FrameCapture> {
    static PROGRAMS: OnceLock<shader::Result<(shader::Program, shader::Program)>> = OnceLock::new();
    let programs = PROGRAMS
        .get_or_init(|| {
            compile_graphics(
                include_bytes!("../../../assets/shaders/lit.vert.spv"),
                include_bytes!("../../../assets/shaders/lit.frag.spv"),
            )
        })
        .as_ref()
        .map_err(|e| e.clone())?;
    material_showcase(width, height, time, programs, None)
}
/// A direct-light Cook-Torrance GGX metallic/roughness scene running from GLSL SPIR-V.
pub fn pbr_showcase(width: u32, height: u32, time: f32) -> Result<FrameCapture> {
    static PROGRAMS: OnceLock<shader::Result<(shader::Program, shader::Program)>> = OnceLock::new();
    let programs = PROGRAMS
        .get_or_init(|| {
            compile_graphics(
                include_bytes!("../../../assets/shaders/pbr.vert.spv"),
                include_bytes!("../../../assets/shaders/pbr.frag.spv"),
            )
        })
        .as_ref()
        .map_err(|e| e.clone())?;
    material_showcase(width, height, time, programs, Some(pbr_normal_map()))
}

/// Render a CPU depth map first, then sample it from ordinary GLSL/SPIR-V fragment shaders.
pub fn shadow_showcase(width: u32, height: u32, time: f32) -> Result<(FrameCapture, Statistics)> {
    use std::sync::Arc;
    if !time.is_finite() {
        return Err("scene time must be finite".into());
    }
    Framebuffer::new(width, height)?;
    const SHADOW_SIZE: u32 = 512;
    let target = Vec3::new(0., 1.2, 0.);
    let direction = Vec3::new(-0.4, 0.85, 0.6).normalize();
    let light_view = Mat4::look_at(target + direction * 18., target, Vec3::new(0., 1., 0.));
    let light_matrix = Mat4::orthographic(-7., 7., -7., 7., 0.1, 30.) * light_view;
    let mut depth_pass = Renderer::new(SHADOW_SIZE, SHADOW_SIZE)?;
    visit_scene("showcase", time, |mesh, model, _, blend| {
        if blend != Blend::Replace {
            return Err("shadow maps require opaque geometry".into());
        }
        let light_mvp = light_matrix * model;
        depth_pass.draw(
            &mesh.vertices,
            Some(&mesh.indices),
            Pipeline {
                color_write: false,
                cull: Cull::Back,
                ..Default::default()
            },
            |v| VertexOutput {
                position: light_mvp.transform(v.position.extend(1.)),
                varyings: [Vec4::ZERO; 4],
            },
            |_| Some(Color::BLACK),
        )
    })?;
    let shadow_stats = depth_pass.stats.clone();
    let shadow_texture = Arc::new(Texture::depth32(
        SHADOW_SIZE,
        SHADOW_SIZE,
        &depth_pass.framebuffer.depth,
    )?);

    static PROGRAMS: OnceLock<shader::Result<(shader::Program, shader::Program)>> = OnceLock::new();
    let programs = PROGRAMS
        .get_or_init(|| {
            compile_graphics(
                include_bytes!("../../../assets/shaders/lit.vert.spv"),
                include_bytes!("../../../assets/shaders/shadow.frag.spv"),
            )
        })
        .as_ref()
        .map_err(|e| e.clone())?;
    let device = Device;
    let mut commands = device.commands();
    commands.begin_render_pass(Color::new(0.022, 0.032, 0.05, 1.));
    commands.bind_pipeline(Arc::new(ShaderPipeline {
        state: Pipeline {
            cull: Cull::Back,
            ..Default::default()
        },
        vertex: programs.0.clone(),
        fragment: programs.1.clone(),
    }));
    commands.bind_texture(0, Arc::new(Texture::checker(128)?), Sampler::default());
    commands.bind_texture(
        1,
        shadow_texture,
        Sampler {
            filter: Filter::Nearest,
            address: Address::Clamp,
            mip: MipFilter::None,
        },
    );
    let eye = Vec3::new(7.5, 5.8, 10.);
    let vp = Mat4::perspective(0.78, width as f32 / height as f32, 0.1, 60.)
        * Mat4::look_at(eye, target, Vec3::new(0., 1., 0.));
    visit_scene("showcase", time, |mesh, model, material, blend| {
        if blend != Blend::Replace {
            return Err("shadow showcase requires opaque draws".into());
        }
        let mut uniforms = lighting_uniforms(vp * model, model, material, eye)?;
        uniforms.extend(light_matrix.0.map(Vec4::from_array));
        uniforms.push(Vec4::new(
            0.002,
            0.3,
            SHADOW_SIZE as f32,
            SHADOW_SIZE as f32,
        ));
        commands.bind_vertex_buffer(device.create_vertex_buffer(mesh.vertices.clone())?);
        commands.bind_index_buffer(device.create_index_buffer(mesh.indices.clone())?);
        commands.bind_uniform_buffer(device.create_uniform_buffer(uniforms)?);
        commands.draw_indexed(0, mesh.indices.len() as u32);
        Ok(())
    })?;
    commands.end_render_pass();
    Ok((
        FrameCapture {
            version: 1,
            width,
            height,
            commands,
        },
        shadow_stats,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tangent_generation_uses_uv_orientation_and_checks_indices() {
        let mesh = Mesh {
            vertices: [
                (Vec3::ZERO, Vec2::ZERO),
                (Vec3::new(1., 0., 0.), Vec2::new(1., 0.)),
                (Vec3::new(0., 1., 0.), Vec2::new(0., 1.)),
            ]
            .map(|(position, uv)| Vertex {
                position,
                normal: Vec3::new(0., 0., 1.),
                uv,
                color: Color::WHITE.0,
            })
            .to_vec(),
            indices: vec![0, 1, 2],
        };
        let vertices = tangent_vertices(&mesh).unwrap();
        assert!((vertices[0].color.x - 1.).abs() < 1e-6);
        assert!(vertices[0].color.y.abs() < 1e-6 && vertices[0].color.z.abs() < 1e-6);
        assert_eq!(vertices[0].color.w, 1.);
        assert!(
            tangent_vertices(&Mesh {
                indices: vec![0, 1, 3],
                ..mesh
            })
            .is_err()
        );
    }
}
