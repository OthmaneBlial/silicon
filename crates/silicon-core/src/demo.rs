//! Reproducible CPU-only scenes. Native Rust closures are the first shader backend.
use crate::*;
use std::sync::OnceLock;
#[derive(Clone, Copy, Debug)]
pub struct Material {
    pub color: Color,
    pub textured: bool,
    pub metallic: f32,
    pub emission: f32,
}
impl Material {
    pub fn matte(color: Color) -> Self {
        Self {
            color,
            textured: true,
            metallic: 0.1,
            emission: 0.,
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
    if matches!(name, "shader_cube" | "spirv_cube" | "spirv_showcase") {
        let (width, height) = r.surface_size();
        let capture = match name {
            "spirv_showcase" => spirv_showcase(width, height, time)?,
            "spirv_cube" => spirv_cube(width, height, time)?,
            _ => shader_cube(width, height, time)?,
        };
        Device.submit(&capture.commands, r)?;
        return Ok(());
    }
    r.clear(Color::new(0.022, 0.032, 0.05, 1.));
    static TEXTURE: OnceLock<Texture> = OnceLock::new();
    let texture = TEXTURE.get_or_init(|| Texture::checker(128).expect("valid built-in checker"));
    let eye = if name == "showcase" {
        Vec3::new(7.5, 5.8, 10.)
    } else {
        Vec3::new(4., 3., 5.)
    };
    let view = Mat4::look_at(
        eye,
        if name == "showcase" {
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
                let tex = if mat.textured {
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
    visit_scene(name, time, draw)
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
                    emission: 0.,
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
                    emission: 0.,
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
                        emission: 1.5,
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
                    emission: 1.,
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
                        emission: 0.,
                    },
                    Blend::Replace,
                )?;
            }
        }
        _ => {
            return Err(format!(
                "unknown scene {name}; choose cube, textured_cube, triangle_3d, showcase"
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
            emission: 0.,
        },
        Vec3::new(4., 3., 5.),
    )?;
    let mut commands = device.commands();
    commands.begin_render_pass(Color::new(0.022, 0.032, 0.05, 1.));
    commands.bind_pipeline(Arc::new(ShaderPipeline {
        state: Pipeline {
            cull: Cull::Back,
            ..Default::default()
        },
        vertex,
        fragment,
    }));
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
    use shader::{
        Program,
        spirv::{Module, link},
    };
    static PROGRAMS: OnceLock<std::result::Result<(Program, Program), String>> = OnceLock::new();
    let programs = PROGRAMS
        .get_or_init(|| {
            let vertex =
                Module::parse(include_bytes!("../../../assets/shaders/textured.vert.spv"))?
                    .translate()?;
            let fragment =
                Module::parse(include_bytes!("../../../assets/shaders/textured.frag.spv"))?
                    .translate()?;
            link(&vertex, &fragment)?;
            Ok((vertex.program, fragment.program))
        })
        .as_ref()
        .map_err(|e| e.clone())?;
    shader_cube_with_programs(width, height, time, programs.0.clone(), programs.1.clone())
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
        0.,
    );
    uniforms[20] = eye.extend(1.);
    Ok(uniforms)
}
/// The lit OBJ showcase executes ordinary GLSL through SPIR-V, SIR and recorded draws.
pub fn spirv_showcase(width: u32, height: u32, time: f32) -> Result<FrameCapture> {
    use shader::{
        Program,
        spirv::{Module, link},
    };
    use std::sync::Arc;
    if !time.is_finite() {
        return Err("scene time must be finite".into());
    }
    Framebuffer::new(width, height)?;
    static PROGRAMS: OnceLock<std::result::Result<(Program, Program), String>> = OnceLock::new();
    let programs = PROGRAMS
        .get_or_init(|| {
            let vertex = Module::parse(include_bytes!("../../../assets/shaders/lit.vert.spv"))?
                .translate()?;
            let fragment = Module::parse(include_bytes!("../../../assets/shaders/lit.frag.spv"))?
                .translate()?;
            link(&vertex, &fragment)?;
            Ok((vertex.program, fragment.program))
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
    let eye = Vec3::new(7.5, 5.8, 10.);
    let vp = Mat4::perspective(0.78, width as f32 / height as f32, 0.1, 60.)
        * Mat4::look_at(eye, Vec3::new(0., 1.2, 0.), Vec3::new(0., 1., 0.));
    visit_scene("showcase", time, |mesh, model, material, blend| {
        if blend != Blend::Replace {
            return Err("lit showcase requires opaque draws".into());
        }
        commands.bind_vertex_buffer(device.create_vertex_buffer(mesh.vertices.clone())?);
        commands.bind_index_buffer(device.create_index_buffer(mesh.indices.clone())?);
        commands.bind_uniform_buffer(device.create_uniform_buffer(lighting_uniforms(
            vp * model,
            model,
            material,
            eye,
        )?)?);
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
