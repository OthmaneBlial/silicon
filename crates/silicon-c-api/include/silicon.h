#ifndef SILICON_H
#define SILICON_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define SILICON_C_API_VERSION 1u
#define SILICON_OK 0
#define SILICON_ERROR 1
#define SILICON_BUFFER_TOO_SMALL 2
#define SILICON_PANIC 3

#define SILICON_CULL_NONE 0
#define SILICON_CULL_FRONT 1
#define SILICON_CULL_BACK 2

typedef struct SiliconDevice SiliconDevice;
typedef struct SiliconPipeline SiliconPipeline;
typedef struct SiliconVertexBuffer SiliconVertexBuffer;
typedef struct SiliconIndexBuffer SiliconIndexBuffer;
typedef struct SiliconUniformBuffer SiliconUniformBuffer;
typedef struct SiliconTexture SiliconTexture;
typedef struct SiliconCommandBuffer SiliconCommandBuffer;

typedef struct SiliconVertex {
    float position[3];
    float normal[3];
    float uv[2];
    float color[4];
} SiliconVertex;

typedef struct SiliconSubmissionStats {
    uint64_t draws;
    uint64_t shader_instructions;
    uint64_t texture_samples;
} SiliconSubmissionStats;

/* Input/output pointers must be valid and aligned for their declared lengths.
 * Serialize mutations/destruction of handles against any other access to them. */
uint32_t silicon_c_api_version(void);
/* Returns required bytes including NUL; output may be NULL to query the size. */
size_t silicon_last_error(char *output, size_t capacity);

SiliconDevice *silicon_device_create(uint32_t width, uint32_t height);
void silicon_device_destroy(SiliconDevice *device);
uint32_t silicon_device_width(const SiliconDevice *device);
uint32_t silicon_device_height(const SiliconDevice *device);

SiliconPipeline *silicon_pipeline_create(
    const SiliconDevice *device,
    const uint8_t *vertex_spirv,
    size_t vertex_len,
    const uint8_t *fragment_spirv,
    size_t fragment_len,
    int32_t cull_mode);
void silicon_pipeline_destroy(SiliconPipeline *pipeline);

SiliconVertexBuffer *silicon_vertex_buffer_create(
    const SiliconDevice *device,
    const SiliconVertex *vertices,
    size_t count);
void silicon_vertex_buffer_destroy(SiliconVertexBuffer *buffer);

SiliconIndexBuffer *silicon_index_buffer_create(
    const SiliconDevice *device,
    const uint32_t *indices,
    size_t count);
void silicon_index_buffer_destroy(SiliconIndexBuffer *buffer);

/* values contains vec4_count consecutive groups of four floats. */
SiliconUniformBuffer *silicon_uniform_buffer_create(
    const SiliconDevice *device,
    const float *values,
    size_t vec4_count);
void silicon_uniform_buffer_destroy(SiliconUniformBuffer *buffer);

SiliconTexture *silicon_texture_create_rgba8(
    const SiliconDevice *device,
    uint32_t width,
    uint32_t height,
    const uint8_t *pixels,
    size_t byte_len);
void silicon_texture_destroy(SiliconTexture *texture);

SiliconCommandBuffer *silicon_command_buffer_create(void);
void silicon_command_buffer_destroy(SiliconCommandBuffer *commands);
int32_t silicon_command_buffer_begin_render_pass(
    SiliconCommandBuffer *commands,
    float red,
    float green,
    float blue,
    float alpha);
int32_t silicon_command_buffer_bind_pipeline(
    SiliconCommandBuffer *commands,
    const SiliconPipeline *pipeline);
int32_t silicon_command_buffer_bind_vertex_buffer(
    SiliconCommandBuffer *commands,
    const SiliconVertexBuffer *buffer);
int32_t silicon_command_buffer_bind_index_buffer(
    SiliconCommandBuffer *commands,
    const SiliconIndexBuffer *buffer);
int32_t silicon_command_buffer_bind_uniform_buffer(
    SiliconCommandBuffer *commands,
    const SiliconUniformBuffer *buffer);
int32_t silicon_command_buffer_bind_texture(
    SiliconCommandBuffer *commands,
    uint8_t slot,
    const SiliconTexture *texture);
int32_t silicon_command_buffer_draw(
    SiliconCommandBuffer *commands,
    uint32_t first,
    uint32_t count);
int32_t silicon_command_buffer_draw_indexed(
    SiliconCommandBuffer *commands,
    uint32_t first,
    uint32_t count);
int32_t silicon_command_buffer_end_render_pass(SiliconCommandBuffer *commands);

int32_t silicon_device_submit(
    SiliconDevice *device,
    const SiliconCommandBuffer *commands,
    SiliconSubmissionStats *stats);
/* On short capacity, returns SILICON_BUFFER_TOO_SMALL and writes required_bytes. */
int32_t silicon_device_copy_framebuffer_rgba8(
    const SiliconDevice *device,
    uint8_t *destination,
    size_t capacity,
    size_t *required_bytes);

#ifdef __cplusplus
}
#endif

#endif
