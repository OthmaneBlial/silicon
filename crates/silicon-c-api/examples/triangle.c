#include "silicon.h"

#include <stdio.h>
#include <stdlib.h>

static int report_error(const char *operation, int32_t status) {
    char message[512];
    silicon_last_error(message, sizeof(message));
    fprintf(stderr, "%s failed (%d): %s\n", operation, status, message);
    return 1;
}

static uint8_t *read_file(const char *path, size_t *length) {
    FILE *file = fopen(path, "rb");
    if (!file || fseek(file, 0, SEEK_END) != 0) return NULL;
    long size = ftell(file);
    if (size <= 0 || fseek(file, 0, SEEK_SET) != 0) {
        fclose(file);
        return NULL;
    }
    uint8_t *bytes = malloc((size_t)size);
    if (!bytes) {
        fclose(file);
        return NULL;
    }
    if (fread(bytes, 1, (size_t)size, file) != (size_t)size) {
        free(bytes);
        fclose(file);
        return NULL;
    }
    fclose(file);
    *length = (size_t)size;
    return bytes;
}

int main(void) {
    if (silicon_c_api_version() != SILICON_C_API_VERSION) {
        fprintf(stderr, "unexpected SILICON C API version\n");
        return 1;
    }
    if (silicon_device_create(0, 240) != NULL) {
        fprintf(stderr, "zero-width device was accepted\n");
        return 1;
    }
    char error_text[128];
    if (silicon_last_error(error_text, sizeof(error_text)) <= 1) {
        fprintf(stderr, "failed device creation did not report a diagnostic\n");
        return 1;
    }

    size_t vertex_len = 0, fragment_len = 0;
    uint8_t *vertex_spirv = read_file("assets/shaders/textured.vert.spv", &vertex_len);
    uint8_t *fragment_spirv = read_file("assets/shaders/textured.frag.spv", &fragment_len);
    if (!vertex_spirv || !fragment_spirv) {
        fprintf(stderr, "could not read SPIR-V fixtures\n");
        free(vertex_spirv);
        free(fragment_spirv);
        return 1;
    }

    SiliconDevice *device = silicon_device_create(320, 240);
    if (!device) return report_error("silicon_device_create", SILICON_ERROR);
    SiliconPipeline *pipeline = silicon_pipeline_create(
        device, vertex_spirv, vertex_len, fragment_spirv, fragment_len, SILICON_CULL_NONE);
    free(vertex_spirv);
    free(fragment_spirv);
    if (!pipeline) return report_error("silicon_pipeline_create", SILICON_ERROR);

    const SiliconVertex vertices[] = {
        {{-0.8f, -0.7f, 0.f}, {0.f, 0.f, 1.f}, {0.f, 0.f}, {1.f, 1.f, 1.f, 1.f}},
        {{0.8f, -0.7f, 0.f}, {0.f, 0.f, 1.f}, {1.f, 0.f}, {1.f, 1.f, 1.f, 1.f}},
        {{0.f, 0.8f, 0.f}, {0.f, 0.f, 1.f}, {0.5f, 1.f}, {1.f, 1.f, 1.f, 1.f}},
    };
    const float identity[] = {
        1.f, 0.f, 0.f, 0.f,
        0.f, 1.f, 0.f, 0.f,
        0.f, 0.f, 1.f, 0.f,
        0.f, 0.f, 0.f, 1.f,
    };
    const uint32_t indices[] = {0, 1, 2};
    const uint8_t white[] = {255, 255, 255, 255};
    SiliconVertexBuffer *vertex_buffer = silicon_vertex_buffer_create(device, vertices, 3);
    SiliconIndexBuffer *index_buffer = silicon_index_buffer_create(device, indices, 3);
    SiliconUniformBuffer *uniform_buffer = silicon_uniform_buffer_create(device, identity, 4);
    SiliconTexture *texture = silicon_texture_create_rgba8(device, 1, 1, white, sizeof(white));
    SiliconCommandBuffer *commands = silicon_command_buffer_create();
    if (!vertex_buffer || !index_buffer || !uniform_buffer || !texture || !commands) {
        return report_error("resource creation", SILICON_ERROR);
    }

    int32_t status;
    if ((status = silicon_command_buffer_begin_render_pass(commands, 0.02f, 0.03f, 0.05f, 1.f)) != SILICON_OK)
        return report_error("begin render pass", status);
    if ((status = silicon_command_buffer_bind_pipeline(commands, pipeline)) != SILICON_OK)
        return report_error("bind pipeline", status);
    if ((status = silicon_command_buffer_bind_vertex_buffer(commands, vertex_buffer)) != SILICON_OK)
        return report_error("bind vertex buffer", status);
    if ((status = silicon_command_buffer_bind_index_buffer(commands, index_buffer)) != SILICON_OK)
        return report_error("bind index buffer", status);
    if ((status = silicon_command_buffer_bind_uniform_buffer(commands, uniform_buffer)) != SILICON_OK)
        return report_error("bind uniform buffer", status);
    if ((status = silicon_command_buffer_bind_texture(commands, 0, texture)) != SILICON_OK)
        return report_error("bind texture", status);

    /* Recorded commands own references; resource handles can now be released. */
    silicon_pipeline_destroy(pipeline);
    silicon_vertex_buffer_destroy(vertex_buffer);
    silicon_index_buffer_destroy(index_buffer);
    silicon_uniform_buffer_destroy(uniform_buffer);
    silicon_texture_destroy(texture);

    if ((status = silicon_command_buffer_draw_indexed(commands, 0, 3)) != SILICON_OK)
        return report_error("indexed draw", status);
    if ((status = silicon_command_buffer_end_render_pass(commands)) != SILICON_OK)
        return report_error("end render pass", status);

    SiliconSubmissionStats stats = {0};
    if ((status = silicon_device_submit(device, commands, &stats)) != SILICON_OK)
        return report_error("submit", status);
    size_t required = 0;
    status = silicon_device_copy_framebuffer_rgba8(device, NULL, 0, &required);
    if (status != SILICON_BUFFER_TOO_SMALL) return report_error("framebuffer size query", status);
    uint8_t *pixels = malloc(required);
    if (!pixels) return report_error("framebuffer allocation", SILICON_ERROR);
    if ((status = silicon_device_copy_framebuffer_rgba8(device, pixels, required, &required)) != SILICON_OK)
        return report_error("framebuffer copy", status);

    size_t changed_pixels = 0;
    for (size_t i = 0; i < required; i += 4) {
        if (pixels[i] > 32 || pixels[i + 1] > 32 || pixels[i + 2] > 32) changed_pixels++;
    }
    if (changed_pixels == 0 || stats.draws != 1 || stats.shader_instructions == 0 || stats.texture_samples == 0) {
        fprintf(stderr, "C API submitted %llu draw(s), ran %llu shader instructions, sampled %llu texels, changed %zu pixels\n",
                (unsigned long long)stats.draws,
                (unsigned long long)stats.shader_instructions,
                (unsigned long long)stats.texture_samples,
                changed_pixels);
        free(pixels);
        return 1;
    }

    FILE *image = fopen("output/c_api_triangle.ppm", "wb");
    if (!image) return report_error("open output/c_api_triangle.ppm", SILICON_ERROR);
    fprintf(image, "P6\n%u %u\n255\n", silicon_device_width(device), silicon_device_height(device));
    for (size_t i = 0; i < required; i += 4) fwrite(&pixels[i], 1, 3, image);
    fclose(image);
    free(pixels);

    silicon_command_buffer_destroy(commands);
    silicon_device_destroy(device);
    printf("C ABI v%u rendered one draw across %zu pixels; wrote output/c_api_triangle.ppm\n",
           silicon_c_api_version(), changed_pixels);
    return 0;
}
