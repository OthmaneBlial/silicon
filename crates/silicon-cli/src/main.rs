use minifb::{Key, KeyRepeat, Window, WindowOptions};
use serde::{Deserialize, Serialize};
use silicon_core::*;
use std::{path::Path, time::Instant};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Scene {
    scene: String,
    width: u32,
    height: u32,
    time: f32,
}
impl Default for Scene {
    fn default() -> Self {
        Self {
            scene: "showcase".into(),
            width: 960,
            height: 640,
            time: 0.,
        }
    }
}
struct Options {
    scene: Scene,
    output: String,
    frames: usize,
    capture: Option<String>,
    pixel: Option<(u32, u32)>,
    backend: Backend,
    samples: SampleCount,
    threads: usize,
    benchmark_report: Option<String>,
}
fn options(args: &[String]) -> Result<Options> {
    let name = args
        .first()
        .filter(|s| !s.starts_with('-'))
        .map_or("showcase", String::as_str);
    let scene = if Path::new(name).is_file() {
        use std::io::Read;
        let file = std::fs::File::open(name)?;
        if file.metadata()?.len() > 1024 * 1024 {
            return Err("scene JSON exceeds 1 MiB".into());
        }
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            return Err("scene JSON exceeds 1 MiB".into());
        }
        serde_json::from_slice(&bytes)?
    } else {
        Scene {
            scene: name.into(),
            ..Default::default()
        }
    };
    let mut o = Options {
        scene,
        output: "output/frame.png".into(),
        frames: 0,
        capture: None,
        pixel: None,
        backend: Backend::Scalar,
        samples: SampleCount::One,
        threads: 1,
        benchmark_report: None,
    };
    let mut i = usize::from(args.first().is_some_and(|s| !s.starts_with('-')));
    while i < args.len() {
        let flag = &args[i];
        let value = args
            .get(i + 1)
            .ok_or_else(|| format!("{flag} requires a value"))?;
        match flag.as_str() {
            "--threads" => o.threads = value.parse()?,
            "--backend" => {
                o.backend = match value.as_str() {
                    "scalar" => Backend::Scalar,
                    "simd" => Backend::Simd,
                    _ => return Err("backend must be scalar or simd".into()),
                }
            }
            "--samples" => {
                o.samples = match value.as_str() {
                    "1" => SampleCount::One,
                    "2" => SampleCount::Two,
                    "4" => SampleCount::Four,
                    _ => return Err("samples must be 1, 2 or 4".into()),
                }
            }
            "--width" => o.scene.width = value.parse()?,
            "--height" => o.scene.height = value.parse()?,
            "--time" => o.scene.time = value.parse()?,
            "--output" => o.output = value.clone(),
            "--frames" => o.frames = value.parse()?,
            "--report" => o.benchmark_report = Some(value.clone()),
            "--capture" => o.capture = Some(value.clone()),
            "--pixel" => {
                let (x, y) = value.split_once(',').ok_or("--pixel requires x,y")?;
                o.pixel = Some((x.parse()?, y.parse()?));
            }
            _ => return Err(format!("unknown option {flag}").into()),
        }
        i += 2;
    }
    Framebuffer::new(o.scene.width, o.scene.height)?;
    if !(1..=64).contains(&o.threads) {
        return Err("threads must be 1..64".into());
    }
    if !o.scene.time.is_finite() {
        return Err("time must be finite".into());
    }
    if o.pixel
        .is_some_and(|(x, y)| x >= o.scene.width || y >= o.scene.height)
    {
        return Err("debug pixel outside framebuffer".into());
    }
    Ok(o)
}
fn frame(r: &mut Renderer, scene: &Scene, threads: usize) -> Result<Option<Submission>> {
    if scene.scene == "shadow_showcase" {
        let (capture, depth_stats) = demo::shadow_showcase(scene.width, scene.height, scene.time)?;
        let submission = if threads == 1 {
            Some(Device.submit(&capture.commands, r)?)
        } else {
            r.render_bands(threads, |band| {
                Device.submit(&capture.commands, band).map(|_| ())
            })?;
            None
        };
        r.stats += &depth_stats;
        return Ok(submission);
    }
    if threads > 1 {
        r.render_bands(threads, |band| frame(band, scene, 1).map(|_| ()))?;
        return Ok(None);
    }
    if matches!(
        scene.scene.as_str(),
        "shader_cube"
            | "spirv_cube"
            | "spirv_showcase"
            | "spirv_cutout"
            | "shadow_showcase"
            | "pbr_showcase"
    ) {
        let c = if scene.scene == "spirv_showcase" {
            demo::spirv_showcase(scene.width, scene.height, scene.time)?
        } else if scene.scene == "pbr_showcase" {
            demo::pbr_showcase(scene.width, scene.height, scene.time)?
        } else if scene.scene == "spirv_cutout" {
            demo::spirv_cutout(scene.width, scene.height, scene.time)?
        } else if scene.scene == "spirv_cube" {
            demo::spirv_cube(scene.width, scene.height, scene.time)?
        } else {
            demo::shader_cube(scene.width, scene.height, scene.time)?
        };
        Ok(Some(Device.submit(&c.commands, r)?))
    } else {
        demo::render_into(r, &scene.scene, scene.time)?;
        Ok(None)
    }
}
fn report(r: &Renderer, elapsed: f64, submission: Option<&Submission>) {
    println!(
        "{}x{} CPU framebuffer | {:.3} ms | {:.2} render FPS",
        r.framebuffer.width,
        r.framebuffer.height,
        elapsed * 1000.,
        1. / elapsed
    );
    println!("Multisample coverage: {}x", r.sample_count().get());
    println!(
        "Vertices: {} | triangles: {} | clipped: {} | culled: {}",
        r.stats.vertices, r.stats.triangles, r.stats.clipped, r.stats.culled
    );
    println!(
        "Tile visits: {} | fragments: {} | early-Z: {} | shaded: {} | discarded: {}",
        r.stats.tiles,
        r.stats.fragments,
        r.stats.early_z_rejected,
        r.stats.shaded,
        r.stats.discarded
    );
    println!(
        "Accumulated worker stage times: vertex {:.3} ms | clipping + raster + shading + ROP {:.3} ms",
        r.stats.vertex_time.as_secs_f64() * 1000.,
        r.stats.raster_time.as_secs_f64() * 1000.
    );
    if r.stats.shader_packets > 0 {
        println!("Shader SIMD: {}", shader::packet_backend_name());
        println!(
            "Fragment shader packets: {} | active lanes: {} | occupancy: {:.1}%",
            r.stats.shader_packets,
            r.stats.shader_packet_lanes,
            r.stats.shader_packet_lanes as f64 / (r.stats.shader_packets * 4) as f64 * 100.
        );
    }
    if r.profile_shaders {
        println!(
            "Fragment shader accumulated time: {:.3} ms (instrumented)",
            r.stats.shader_time.as_secs_f64() * 1000.
        );
    }
    if let Some(s) = submission {
        println!("Draws: {}", s.draws);
    }
    if r.stats.shader_instructions > 0 {
        println!(
            "Executed SIR instructions: {} | texture samples: {}",
            r.stats.shader_instructions, r.stats.texture_samples
        );
    }
}
fn load_shader(path: &str) -> Result<(shader::spirv::Module, shader::spirv::Compiled)> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > 1024 * 1024 {
        return Err(format!("{path}: SPIR-V exceeds 1 MiB").into());
    }
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    let module = shader::spirv::Module::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
    let compiled = module.translate().map_err(|e| format!("{path}: {e}"))?;
    Ok((module, compiled))
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let command = args.first().map_or("help", String::as_str);
    if command == "help" || command == "--help" {
        println!(
            "SILICON Software GPU\n\n  silicon info\n  silicon render [scene|scene.json] [--width W --height H --time T --output frame.png]\n  silicon run [scene] [--frames N]\n  silicon benchmark [scene] [--frames N --report timings.json]\n  silicon profile [scene]\n  silicon debug-pixel [scene] --pixel X,Y\n  silicon render shader_cube --capture frame.silicon\n  silicon replay frame.silicon [--output frame.png]\n  silicon inspect frame.silicon\n  silicon inspect-shader shader.spv\n  silicon render-shaders vertex.spv fragment.spv [render options]\n\nExecution: --backend scalar|simd --threads 1..64 --samples 1|2|4\nScenes: showcase, cubemap_showcase, cube, textured_cube, triangle_3d, shader_cube, spirv_cube, spirv_showcase, spirv_cutout, shadow_showcase, pbr_showcase, stencil\nWindow: Escape exits, Space pauses, arrows adjust rotation. PNG and capture modes need no display."
        );
        return Ok(());
    }
    if command == "info" {
        println!(
            "SILICON {}\nHost: {} / {}\nThreads available: {}\nRenderer: CPU / scalar / 16x16 tiles\nShader engines: Rust closures, validated SIR interpreter, strict SPIR-V 1.0 subset\nCompatibility: no Vulkan/OpenGL driver",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::ARCH,
            std::env::consts::OS,
            std::thread::available_parallelism().map_or(1, usize::from)
        );
        println!(
            "Available SIR packet backend: {}",
            shader::packet_backend_name()
        );
        return Ok(());
    }
    if command == "inspect-shader" || command == "render-shaders" {
        let path = args.get(1).ok_or("SPIR-V path required")?;
        let (module, vertex) = load_shader(path)?;
        if command == "inspect-shader" {
            println!(
                "SPIR-V 1.0 | {:?} main | ID bound {} | {} binary / {} SIR instructions",
                vertex.stage,
                module.bound(),
                module.instructions().len(),
                vertex.program.instructions().len()
            );
            println!(
                "Inputs: {:?} | outputs: {:?}",
                vertex.inputs, vertex.outputs
            );
            for op in module.instructions() {
                println!(
                    "word {}: {} {:?}",
                    op.word,
                    shader::spirv::name(op.opcode),
                    op.operands
                );
            }
            return Ok(());
        }
        let fragment_path = args.get(2).ok_or("fragment SPIR-V path required")?;
        let (_, fragment) = load_shader(fragment_path)?;
        shader::spirv::link(&vertex, &fragment)?;
        let o = options(&args[3..])?;
        if o.capture.is_some() && o.samples != SampleCount::One {
            return Err("frame captures do not store multisample state".into());
        }
        let c = demo::shader_cube_with_programs(
            o.scene.width,
            o.scene.height,
            o.scene.time,
            vertex.program,
            fragment.program,
        )?;
        let mut r = Renderer::new(o.scene.width, o.scene.height)?;
        r.set_sample_count(o.samples)?;
        r.backend = o.backend;
        r.debug_pixel = o.pixel;
        let start = Instant::now();
        let submission = if o.threads == 1 {
            Some(Device.submit(&c.commands, &mut r)?)
        } else {
            r.render_bands(o.threads, |band| {
                Device.submit(&c.commands, band).map(|_| ())
            })?;
            None
        };
        report(&r, start.elapsed().as_secs_f64(), submission.as_ref());
        r.framebuffer.save_png(&o.output)?;
        if let Some(path) = o.capture {
            c.save(path)?;
        }
        println!(
            "Saved {} from externally compiled GLSL/SPIR-V through SIR",
            o.output
        );
        return Ok(());
    }
    if command == "replay" || command == "inspect" {
        let path = args.get(1).ok_or("capture path required")?;
        let c = FrameCapture::load(path)?;
        if command == "inspect" {
            println!(
                "SILICON capture v{} | {}x{} | {} commands",
                c.version,
                c.width,
                c.height,
                c.commands.stream().len()
            );
            for (i, cmd) in c.commands.stream().iter().enumerate() {
                match cmd {
                    Command::BindVertices(b) => println!(
                        "{i}: vertex buffer, {} bytes, {} vertices",
                        b.size(),
                        b.mapped().len()
                    ),
                    Command::BindIndices(b) => println!("{i}: index buffer, {} bytes", b.size()),
                    Command::BindUniforms(b) => println!("{i}: uniform buffer, {} bytes", b.size()),
                    Command::BindTexture { slot, texture, .. } => println!(
                        "{i}: texture {slot}, {}x{}, {} mips",
                        texture.levels[0].width,
                        texture.levels[0].height,
                        texture.levels.len()
                    ),
                    Command::BindPipeline(p) => println!(
                        "{i}: pipeline, {} vertex / {} fragment instructions",
                        p.vertex.instructions().len(),
                        p.fragment.instructions().len()
                    ),
                    _ => println!("{i}: {cmd:?}"),
                }
            }
            return Ok(());
        }
        let output = match args.get(2).map(String::as_str) {
            None => "output/replay.png",
            Some("--output") => args.get(3).ok_or("--output requires path")?,
            Some(s) => return Err(format!("unknown replay option {s}").into()),
        };
        let start = Instant::now();
        let r = c.replay()?;
        report(&r, start.elapsed().as_secs_f64(), None);
        r.framebuffer.save_png(output)?;
        println!("Saved {output}");
        return Ok(());
    }
    if !["render", "run", "benchmark", "profile", "debug-pixel"].contains(&command) {
        return Err(format!("unknown command {command}; run silicon --help").into());
    }
    let mut o = options(&args[1..])?;
    if o.capture.is_some() && o.samples != SampleCount::One {
        return Err("frame captures do not store multisample state".into());
    }
    let mut r = Renderer::new(o.scene.width, o.scene.height)?;
    r.set_sample_count(o.samples)?;
    if o.benchmark_report.is_some() && command != "benchmark" {
        return Err("--report is a benchmark option".into());
    }
    r.profile_shaders = command == "profile";
    r.debug_pixel = o.pixel;
    r.backend = o.backend;
    if command == "benchmark" {
        let count = if o.frames == 0 { 30 } else { o.frames };
        if !(1..=10000).contains(&count) {
            return Err("benchmark frames must be 1..10000".into());
        }
        for _ in 0..3 {
            frame(&mut r, &o.scene, o.threads)?;
        }
        let mut times = Vec::with_capacity(count);
        let mut total_shaded = 0u64;
        let mut total_triangles = 0u64;
        let mut total_packets = 0u64;
        let mut packet_lanes = 0u64;
        let mut instructions = 0u64;
        let mut samples = 0u64;
        let mut discarded = 0u64;
        for _ in 0..count {
            let start = Instant::now();
            frame(&mut r, &o.scene, o.threads)?;
            times.push(start.elapsed().as_secs_f64());
            total_shaded += r.stats.shaded;
            total_triangles += r.stats.triangles;
            total_packets += r.stats.shader_packets;
            packet_lanes += r.stats.shader_packet_lanes;
            instructions += r.stats.shader_instructions;
            samples += r.stats.texture_samples;
            discarded += r.stats.discarded;
        }
        if let Some(path) = &o.benchmark_report {
            let path = Path::new(path);
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            let data = serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "scene": o.scene,
                "coverage_backend": simd::name(o.backend),
                "sample_count": o.samples.get(),
                "shader_packet_backend": if total_packets > 0 {
                    Some(shader::packet_backend_name())
                } else { None },
                "workers": o.threads,
                "warmups": 3,
                "frame_ms": times.iter().map(|t| t * 1000.).collect::<Vec<_>>(),
                "logical_shaded_fragments": total_shaded,
                "submitted_triangles": total_triangles,
                "shader_packets": total_packets,
                "packet_active_lanes": packet_lanes,
                "executed_shader_instructions": instructions,
                "texture_samples": samples,
                "discarded_fragments": discarded,
            });
            std::fs::write(path, serde_json::to_vec_pretty(&data)?)?;
        }
        times.sort_by(f64::total_cmp);
        let total: f64 = times.iter().sum();
        println!(
            "SILICON benchmark | scene {} | {}x{} | {count} frames | 3 warmups",
            o.scene.scene, o.scene.width, o.scene.height
        );
        println!(
            "median_ms,p95_ms,render_fps,shaded_pixels_per_s,triangles_per_s\n{:.4},{:.4},{:.2},{:.0},{:.0}",
            times[count / 2] * 1000.,
            times[((count as f64 * 0.95).ceil() as usize - 1).min(count - 1)] * 1000.,
            count as f64 / total,
            total_shaded as f64 / total,
            total_triangles as f64 / total
        );
        return Ok(());
    }
    if command == "run" {
        let mut window = Window::new(
            "SILICON — CPU framebuffer",
            o.scene.width as usize,
            o.scene.height as usize,
            WindowOptions {
                resize: true,
                ..Default::default()
            },
        )?;
        window.set_target_fps(60);
        let mut pixels = vec![0u32; (o.scene.width * o.scene.height) as usize];
        let mut number = 0;
        let mut paused = false;
        let mut last = Instant::now();
        let start = last;
        while window.is_open()
            && !window.is_key_down(Key::Escape)
            && (o.frames == 0 || number < o.frames)
        {
            let now = Instant::now();
            let delta = now.duration_since(last).as_secs_f32();
            last = now;
            if window.is_key_pressed(Key::Space, KeyRepeat::No) {
                paused = !paused;
            }
            if !paused {
                o.scene.time += delta;
            }
            if window.is_key_down(Key::Left) {
                o.scene.time -= delta * 2.;
            }
            if window.is_key_down(Key::Right) {
                o.scene.time += delta * 2.;
            }
            let render = Instant::now();
            frame(&mut r, &o.scene, o.threads)?;
            let render_ms = render.elapsed().as_secs_f64() * 1000.;
            r.framebuffer.present_into(&mut pixels)?;
            window.set_title(&format!(
                "SILICON | CPU | {}x{} | {} triangles | {:.2} ms render",
                o.scene.width, o.scene.height, r.stats.triangles, render_ms
            ));
            window.update_with_buffer(&pixels, o.scene.width as usize, o.scene.height as usize)?;
            number += 1;
        }
        r.framebuffer.save_png(&o.output)?;
        println!(
            "Presented {number} CPU-rendered frames in {:.3} s; final framebuffer: {}",
            start.elapsed().as_secs_f64(),
            o.output
        );
        return Ok(());
    }
    let start = Instant::now();
    let submission = frame(&mut r, &o.scene, o.threads)?;
    report(&r, start.elapsed().as_secs_f64(), submission.as_ref());
    if command == "debug-pixel" {
        if o.pixel.is_none() {
            return Err("debug-pixel requires --pixel X,Y".into());
        }
        if let Some(s) = &submission {
            for (primitive, trace) in &s.shader_traces {
                println!("Primitive {primitive} SIR fragment execution:");
                for t in trace {
                    println!("  #{} {:?} => {:?}", t.instruction, t.operation, t.value);
                }
            }
        }
        for trace in &r.traces {
            println!("{trace:#?}");
        }
        if r.traces.is_empty() {
            println!("No primitive covers this pixel; framebuffer clear color.");
        }
    }
    if let Some(path) = o.capture {
        let c = match o.scene.scene.as_str() {
            "shader_cube" => demo::shader_cube(o.scene.width, o.scene.height, o.scene.time)?,
            "spirv_cube" => demo::spirv_cube(o.scene.width, o.scene.height, o.scene.time)?,
            "spirv_showcase" => demo::spirv_showcase(o.scene.width, o.scene.height, o.scene.time)?,
            "pbr_showcase" => demo::pbr_showcase(o.scene.width, o.scene.height, o.scene.time)?,
            "spirv_cutout" => demo::spirv_cutout(o.scene.width, o.scene.height, o.scene.time)?,
            "shadow_showcase" => {
                demo::shadow_showcase(o.scene.width, o.scene.height, o.scene.time)?.0
            }
            _ => {
                return Err(
                    "serialized capture requires shader_cube, spirv_cube, spirv_showcase, spirv_cutout, shadow_showcase or pbr_showcase".into(),
                );
            }
        };
        c.save(&path)?;
        println!("Captured {path}");
    }
    r.framebuffer.save_png(&o.output)?;
    println!("Saved {}", o.output);
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("SILICON: {e}");
        std::process::exit(1);
    }
}
