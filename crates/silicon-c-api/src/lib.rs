//! Versioned C ABI over the supported synchronous `silicon::api` surface.
//!
//! # Safety
//! Raw inputs must satisfy each function's pointer contract. Handles must be live values from the
//! matching constructors, and mutable access or destruction must be serialized per handle.
use silicon::api as gpu;
use std::{
    cell::RefCell,
    ffi::{CString, c_char},
    mem::size_of,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr, slice,
    sync::Arc,
};

pub const SILICON_OK: i32 = 0;
pub const SILICON_ERROR: i32 = 1;
pub const SILICON_BUFFER_TOO_SMALL: i32 = 2;
pub const SILICON_PANIC: i32 = 3;
pub const SILICON_CULL_NONE: i32 = 0;
pub const SILICON_CULL_FRONT: i32 = 1;
pub const SILICON_CULL_BACK: i32 = 2;
pub const SILICON_C_API_VERSION: u32 = 1;

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

type CResult<T> = std::result::Result<T, (i32, String)>;

#[repr(C)]
pub struct SiliconDevice {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct SiliconPipeline {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct SiliconVertexBuffer {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct SiliconIndexBuffer {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct SiliconUniformBuffer {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct SiliconTexture {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct SiliconCommandBuffer {
    _opaque: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SiliconVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SiliconSubmissionStats {
    pub draws: u64,
    pub shader_instructions: u64,
    pub texture_samples: u64,
}

struct DeviceInner {
    device: gpu::Device,
    renderer: gpu::Renderer,
}
struct PipelineInner(Arc<gpu::ShaderPipeline>);
struct VertexBufferInner(gpu::Buffer<gpu::Vertex>);
struct IndexBufferInner(gpu::Buffer<u32>);
struct UniformBufferInner(gpu::Buffer<gpu::Vec4>);
struct TextureInner(Arc<gpu::Texture>);
struct CommandBufferInner(gpu::CommandBuffer);

fn error<T>(message: impl Into<String>) -> CResult<T> {
    Err((SILICON_ERROR, message.into()))
}

fn core<T>(result: gpu::Result<T>) -> CResult<T> {
    result.map_err(|e| (SILICON_ERROR, e.to_string()))
}

fn set_last_error(message: &str) {
    let message = CString::new(message.replace('\0', " ")).unwrap_or_default();
    LAST_ERROR.with(|slot| *slot.borrow_mut() = message);
}

fn boundary<T>(f: impl FnOnce() -> CResult<T>) -> std::result::Result<T, i32> {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = CString::default());
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err((status, message))) => {
            set_last_error(&message);
            Err(status)
        }
        Err(_) => {
            set_last_error("unexpected Rust panic inside the SILICON C API");
            Err(SILICON_PANIC)
        }
    }
}

fn status(f: impl FnOnce() -> CResult<()>) -> i32 {
    boundary(f).map_or_else(|status| status, |_| SILICON_OK)
}

fn into_handle<T, H>(value: T) -> *mut H {
    Box::into_raw(Box::new(value)).cast()
}

unsafe fn handle_ref<'a, T, H>(handle: *const H, name: &str) -> CResult<&'a T> {
    if handle.is_null() {
        return error(format!("{name} handle is null"));
    }
    Ok(unsafe { &*handle.cast::<T>() })
}

unsafe fn handle_mut<'a, T, H>(handle: *mut H, name: &str) -> CResult<&'a mut T> {
    if handle.is_null() {
        return error(format!("{name} handle is null"));
    }
    Ok(unsafe { &mut *handle.cast::<T>() })
}

unsafe fn drop_handle<T, H>(handle: *mut H) {
    if !handle.is_null() {
        unsafe { drop(Box::from_raw(handle.cast::<T>())) };
    }
}

/// # Safety
/// For nonzero `len`, `data` must be aligned and readable for `len` consecutive `T` values.
unsafe fn input_slice<'a, T>(
    data: *const T,
    len: usize,
    max: usize,
    name: &str,
) -> CResult<&'a [T]> {
    if len > max {
        return error(format!("{name} exceeds the limit of {max} elements"));
    }
    if len == 0 {
        return Ok(&[]);
    }
    if data.is_null() {
        return error(format!("{name} pointer is null"));
    }
    if len
        .checked_mul(size_of::<T>())
        .is_none_or(|n| n > isize::MAX as usize)
    {
        return error(format!("{name} byte length overflows"));
    }
    Ok(unsafe { slice::from_raw_parts(data, len) })
}

/// Returns this shared library's C ABI version.
#[unsafe(no_mangle)]
pub extern "C" fn silicon_c_api_version() -> u32 {
    SILICON_C_API_VERSION
}

/// Copies the calling thread's last error into `output` and returns its required size,
/// including the trailing NUL. A null output or zero capacity performs a size query.
///
/// # Safety
/// When `output` is non-null and `capacity` is nonzero, it must be writable for `capacity` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_last_error(output: *mut c_char, capacity: usize) -> usize {
    LAST_ERROR.with(|slot| {
        let message = slot.borrow();
        let bytes = message.as_bytes_with_nul();
        if !output.is_null() && capacity > 0 {
            let copied = bytes.len().min(capacity.saturating_sub(1));
            unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), output.cast::<u8>(), copied) };
            unsafe { *output.add(copied) = 0 };
        }
        bytes.len()
    })
}

/// Creates a device with one RGBA8 framebuffer.
#[unsafe(no_mangle)]
pub extern "C" fn silicon_device_create(width: u32, height: u32) -> *mut SiliconDevice {
    match boundary(|| {
        let renderer = core(gpu::Renderer::new(width, height))?;
        Ok(into_handle::<_, SiliconDevice>(DeviceInner {
            device: gpu::Device::new(),
            renderer,
        }))
    }) {
        Ok(handle) => handle,
        Err(_) => ptr::null_mut(),
    }
}

/// Destroys a device. Null is ignored.
///
/// # Safety
/// A non-null handle must be live and returned by `silicon_device_create`, exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_device_destroy(handle: *mut SiliconDevice) {
    unsafe { drop_handle::<DeviceInner, _>(handle) };
}

/// Returns the device framebuffer width, or zero for a null handle.
///
/// # Safety
/// A non-null handle must be a live `SiliconDevice` returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_device_width(handle: *const SiliconDevice) -> u32 {
    boundary(|| {
        let device = unsafe { handle_ref::<DeviceInner, _>(handle, "device")? };
        Ok(device.renderer.framebuffer.width)
    })
    .unwrap_or_default()
}

/// Returns the device framebuffer height, or zero for a null handle.
///
/// # Safety
/// A non-null handle must be a live `SiliconDevice` returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_device_height(handle: *const SiliconDevice) -> u32 {
    boundary(|| {
        let device = unsafe { handle_ref::<DeviceInner, _>(handle, "device")? };
        Ok(device.renderer.framebuffer.height)
    })
    .unwrap_or_default()
}

/// Creates a linked SPIR-V pipeline. `cull_mode` is one of `SILICON_CULL_*`.
///
/// # Safety
/// Shader pointers must address their stated readable byte ranges. The device must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_pipeline_create(
    device: *const SiliconDevice,
    vertex_spirv: *const u8,
    vertex_len: usize,
    fragment_spirv: *const u8,
    fragment_len: usize,
    cull_mode: i32,
) -> *mut SiliconPipeline {
    match boundary(|| {
        let device = unsafe { handle_ref::<DeviceInner, _>(device, "device")? };
        let vertex_bytes =
            unsafe { input_slice(vertex_spirv, vertex_len, 1024 * 1024, "vertex SPIR-V")? };
        let fragment_bytes =
            unsafe { input_slice(fragment_spirv, fragment_len, 1024 * 1024, "fragment SPIR-V")? };
        let cull = match cull_mode {
            SILICON_CULL_NONE => gpu::Cull::None,
            SILICON_CULL_FRONT => gpu::Cull::Front,
            SILICON_CULL_BACK => gpu::Cull::Back,
            _ => return error("invalid cull mode"),
        };
        let vertex = device
            .device
            .create_shader(vertex_bytes)
            .map_err(|e| (SILICON_ERROR, e))?;
        let fragment = device
            .device
            .create_shader(fragment_bytes)
            .map_err(|e| (SILICON_ERROR, e))?;
        let pipeline = device
            .device
            .create_pipeline(
                &vertex,
                &fragment,
                gpu::Pipeline {
                    cull,
                    ..Default::default()
                },
            )
            .map_err(|e| (SILICON_ERROR, e))?;
        Ok(into_handle::<_, SiliconPipeline>(PipelineInner(pipeline)))
    }) {
        Ok(handle) => handle,
        Err(_) => ptr::null_mut(),
    }
}

/// Destroys a pipeline. Null is ignored.
///
/// # Safety
/// A non-null handle must be live and returned by `silicon_pipeline_create`, exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_pipeline_destroy(handle: *mut SiliconPipeline) {
    unsafe { drop_handle::<PipelineInner, _>(handle) };
}

/// Creates an owned vertex buffer from C-compatible interleaved vertices.
///
/// # Safety
/// A non-empty `vertices` range must be readable; `device` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_vertex_buffer_create(
    device: *const SiliconDevice,
    vertices: *const SiliconVertex,
    count: usize,
) -> *mut SiliconVertexBuffer {
    match boundary(|| {
        let device = unsafe { handle_ref::<DeviceInner, _>(device, "device")? };
        let input = unsafe { input_slice(vertices, count, 1_000_000, "vertices")? };
        let data = input
            .iter()
            .map(|v| gpu::Vertex {
                position: gpu::Vec3::new(v.position[0], v.position[1], v.position[2]),
                normal: gpu::Vec3::new(v.normal[0], v.normal[1], v.normal[2]),
                uv: gpu::Vec2::new(v.uv[0], v.uv[1]),
                color: gpu::Vec4::new(v.color[0], v.color[1], v.color[2], v.color[3]),
            })
            .collect();
        let buffer = core(device.device.create_vertex_buffer(data))?;
        Ok(into_handle::<_, SiliconVertexBuffer>(VertexBufferInner(
            buffer,
        )))
    }) {
        Ok(handle) => handle,
        Err(_) => ptr::null_mut(),
    }
}

/// Destroys a vertex buffer. Null is ignored.
///
/// # Safety
/// A non-null handle must be live and returned by `silicon_vertex_buffer_create`, exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_vertex_buffer_destroy(handle: *mut SiliconVertexBuffer) {
    unsafe { drop_handle::<VertexBufferInner, _>(handle) };
}

/// Creates an owned 32-bit index buffer.
///
/// # Safety
/// A non-empty `indices` range must be readable; `device` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_index_buffer_create(
    device: *const SiliconDevice,
    indices: *const u32,
    count: usize,
) -> *mut SiliconIndexBuffer {
    match boundary(|| {
        let device = unsafe { handle_ref::<DeviceInner, _>(device, "device")? };
        let data = unsafe { input_slice(indices, count, 3_000_000, "indices")? }.to_vec();
        let buffer = core(device.device.create_index_buffer(data))?;
        Ok(into_handle::<_, SiliconIndexBuffer>(IndexBufferInner(
            buffer,
        )))
    }) {
        Ok(handle) => handle,
        Err(_) => ptr::null_mut(),
    }
}

/// Destroys an index buffer. Null is ignored.
///
/// # Safety
/// A non-null handle must be live and returned by `silicon_index_buffer_create`, exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_index_buffer_destroy(handle: *mut SiliconIndexBuffer) {
    unsafe { drop_handle::<IndexBufferInner, _>(handle) };
}

/// Creates an owned uniform buffer from `vec4_count` consecutive four-float values.
///
/// # Safety
/// A non-empty `values` range of `vec4_count * 4` floats must be readable; `device` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_uniform_buffer_create(
    device: *const SiliconDevice,
    values: *const f32,
    vec4_count: usize,
) -> *mut SiliconUniformBuffer {
    match boundary(|| {
        let device = unsafe { handle_ref::<DeviceInner, _>(device, "device")? };
        if vec4_count > 64 {
            return error("uniform buffer exceeds 64 vec4 values");
        }
        let scalar_count = vec4_count * 4;
        let data = unsafe { input_slice(values, scalar_count, 256, "uniform values")? }
            .chunks_exact(4)
            .map(|v| gpu::Vec4::new(v[0], v[1], v[2], v[3]))
            .collect();
        let buffer = core(device.device.create_uniform_buffer(data))?;
        Ok(into_handle::<_, SiliconUniformBuffer>(UniformBufferInner(
            buffer,
        )))
    }) {
        Ok(handle) => handle,
        Err(_) => ptr::null_mut(),
    }
}

/// Destroys a uniform buffer. Null is ignored.
///
/// # Safety
/// A non-null handle must be live and returned by `silicon_uniform_buffer_create`, exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_uniform_buffer_destroy(handle: *mut SiliconUniformBuffer) {
    unsafe { drop_handle::<UniformBufferInner, _>(handle) };
}

/// Creates a single-level RGBA8 texture.
///
/// # Safety
/// A non-empty `pixels` range must be readable; `device` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_texture_create_rgba8(
    device: *const SiliconDevice,
    width: u32,
    height: u32,
    pixels: *const u8,
    byte_len: usize,
) -> *mut SiliconTexture {
    match boundary(|| {
        let _device = unsafe { handle_ref::<DeviceInner, _>(device, "device")? };
        let bytes = unsafe { input_slice(pixels, byte_len, 16_777_216 * 4, "texture pixels")? };
        let texture = core(gpu::Texture::new(
            width,
            height,
            gpu::TextureFormat::Rgba8,
            bytes,
        ))?;
        Ok(into_handle::<_, SiliconTexture>(TextureInner(Arc::new(
            texture,
        ))))
    }) {
        Ok(handle) => handle,
        Err(_) => ptr::null_mut(),
    }
}

/// Destroys a texture. Null is ignored.
///
/// # Safety
/// A non-null handle must be live and returned by `silicon_texture_create_rgba8`, exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_texture_destroy(handle: *mut SiliconTexture) {
    unsafe { drop_handle::<TextureInner, _>(handle) };
}

/// Creates an empty command buffer.
#[unsafe(no_mangle)]
pub extern "C" fn silicon_command_buffer_create() -> *mut SiliconCommandBuffer {
    match boundary(|| {
        Ok(into_handle::<_, SiliconCommandBuffer>(CommandBufferInner(
            gpu::Device::new().commands(),
        )))
    }) {
        Ok(handle) => handle,
        Err(_) => ptr::null_mut(),
    }
}

/// Destroys a command buffer. Null is ignored.
///
/// # Safety
/// A non-null handle must be live and returned by `silicon_command_buffer_create`, exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_command_buffer_destroy(handle: *mut SiliconCommandBuffer) {
    unsafe { drop_handle::<CommandBufferInner, _>(handle) };
}

/// Begins the command buffer's render pass after a finite RGBA clear color.
///
/// # Safety
/// `commands` must be a live handle returned by `silicon_command_buffer_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_command_buffer_begin_render_pass(
    commands: *mut SiliconCommandBuffer,
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
) -> i32 {
    status(|| {
        if ![red, green, blue, alpha].iter().all(|v| v.is_finite()) {
            return error("clear color components must be finite");
        }
        let commands = unsafe { handle_mut::<CommandBufferInner, _>(commands, "command buffer")? };
        commands
            .0
            .begin_render_pass(gpu::Color::new(red, green, blue, alpha));
        Ok(())
    })
}

/// Binds a pipeline; the command buffer retains its own shared reference.
///
/// # Safety
/// Both handles must be live handles returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_command_buffer_bind_pipeline(
    commands: *mut SiliconCommandBuffer,
    pipeline: *const SiliconPipeline,
) -> i32 {
    status(|| {
        let commands = unsafe { handle_mut::<CommandBufferInner, _>(commands, "command buffer")? };
        let pipeline = unsafe { handle_ref::<PipelineInner, _>(pipeline, "pipeline")? };
        commands.0.bind_pipeline(Arc::clone(&pipeline.0));
        Ok(())
    })
}

/// Binds a vertex buffer; the command buffer retains its resources.
///
/// # Safety
/// Both handles must be live handles returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_command_buffer_bind_vertex_buffer(
    commands: *mut SiliconCommandBuffer,
    buffer: *const SiliconVertexBuffer,
) -> i32 {
    status(|| {
        let commands = unsafe { handle_mut::<CommandBufferInner, _>(commands, "command buffer")? };
        let buffer = unsafe { handle_ref::<VertexBufferInner, _>(buffer, "vertex buffer")? };
        commands.0.bind_vertex_buffer(buffer.0.clone());
        Ok(())
    })
}

/// Binds an index buffer; the command buffer retains its resources.
///
/// # Safety
/// Both handles must be live handles returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_command_buffer_bind_index_buffer(
    commands: *mut SiliconCommandBuffer,
    buffer: *const SiliconIndexBuffer,
) -> i32 {
    status(|| {
        let commands = unsafe { handle_mut::<CommandBufferInner, _>(commands, "command buffer")? };
        let buffer = unsafe { handle_ref::<IndexBufferInner, _>(buffer, "index buffer")? };
        commands.0.bind_index_buffer(buffer.0.clone());
        Ok(())
    })
}

/// Binds a uniform buffer; the command buffer retains its resources.
///
/// # Safety
/// Both handles must be live handles returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_command_buffer_bind_uniform_buffer(
    commands: *mut SiliconCommandBuffer,
    buffer: *const SiliconUniformBuffer,
) -> i32 {
    status(|| {
        let commands = unsafe { handle_mut::<CommandBufferInner, _>(commands, "command buffer")? };
        let buffer = unsafe { handle_ref::<UniformBufferInner, _>(buffer, "uniform buffer")? };
        commands.0.bind_uniform_buffer(buffer.0.clone());
        Ok(())
    })
}

/// Binds an RGBA8 texture to the specified shader resource slot.
///
/// # Safety
/// Both handles must be live handles returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_command_buffer_bind_texture(
    commands: *mut SiliconCommandBuffer,
    slot: u8,
    texture: *const SiliconTexture,
) -> i32 {
    status(|| {
        let commands = unsafe { handle_mut::<CommandBufferInner, _>(commands, "command buffer")? };
        let texture = unsafe { handle_ref::<TextureInner, _>(texture, "texture")? };
        commands
            .0
            .bind_texture(slot, Arc::clone(&texture.0), gpu::Sampler::default());
        Ok(())
    })
}

/// Records a non-indexed triangle draw range.
///
/// # Safety
/// `commands` must be a live handle returned by `silicon_command_buffer_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_command_buffer_draw(
    commands: *mut SiliconCommandBuffer,
    first: u32,
    count: u32,
) -> i32 {
    status(|| {
        let commands = unsafe { handle_mut::<CommandBufferInner, _>(commands, "command buffer")? };
        commands.0.draw(first, count);
        Ok(())
    })
}

/// Records an indexed triangle draw range.
///
/// # Safety
/// `commands` must be a live handle returned by `silicon_command_buffer_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_command_buffer_draw_indexed(
    commands: *mut SiliconCommandBuffer,
    first: u32,
    count: u32,
) -> i32 {
    status(|| {
        let commands = unsafe { handle_mut::<CommandBufferInner, _>(commands, "command buffer")? };
        commands.0.draw_indexed(first, count);
        Ok(())
    })
}

/// Ends the current render pass.
///
/// # Safety
/// `commands` must be a live handle returned by `silicon_command_buffer_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_command_buffer_end_render_pass(
    commands: *mut SiliconCommandBuffer,
) -> i32 {
    status(|| {
        let commands = unsafe { handle_mut::<CommandBufferInner, _>(commands, "command buffer")? };
        commands.0.end_render_pass();
        Ok(())
    })
}

/// Submits commands synchronously. `stats` may be null.
///
/// # Safety
/// Device and command handles must be live. A non-null `stats` must be writable for one stats struct.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_device_submit(
    device: *mut SiliconDevice,
    commands: *const SiliconCommandBuffer,
    stats: *mut SiliconSubmissionStats,
) -> i32 {
    status(|| {
        let device = unsafe { handle_mut::<DeviceInner, _>(device, "device")? };
        let commands = unsafe { handle_ref::<CommandBufferInner, _>(commands, "command buffer")? };
        let result = core(device.device.submit(&commands.0, &mut device.renderer))?;
        if !stats.is_null() {
            unsafe {
                ptr::write(
                    stats,
                    SiliconSubmissionStats {
                        draws: result.draws,
                        shader_instructions: result.shader_instructions,
                        texture_samples: result.texture_samples,
                    },
                )
            };
        }
        Ok(())
    })
}

/// Copies the RGBA8 framebuffer. On a short destination, writes the needed byte count and returns
/// `SILICON_BUFFER_TOO_SMALL` without copying pixels.
///
/// # Safety
/// Device must be live. `required_bytes` must be writable. A sufficient non-null destination must
/// be writable for `capacity` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn silicon_device_copy_framebuffer_rgba8(
    device: *const SiliconDevice,
    destination: *mut u8,
    capacity: usize,
    required_bytes: *mut usize,
) -> i32 {
    status(|| {
        if required_bytes.is_null() {
            return error("required_bytes pointer is null");
        }
        let device = unsafe { handle_ref::<DeviceInner, _>(device, "device")? };
        let pixels = device.renderer.framebuffer.bytes();
        unsafe { ptr::write(required_bytes, pixels.len()) };
        if capacity < pixels.len() {
            return Err((
                SILICON_BUFFER_TOO_SMALL,
                format!("framebuffer needs {} bytes", pixels.len()),
            ));
        }
        if destination.is_null() {
            return error("destination pointer is null");
        }
        unsafe { ptr::copy_nonoverlapping(pixels.as_ptr(), destination, pixels.len()) };
        Ok(())
    })
}
