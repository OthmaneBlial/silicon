#version 450
layout(location = 0) in vec3 position;
layout(location = 1) in vec4 vertexColor;
layout(location = 2) in vec2 inUv;
layout(location = 0) out vec4 color;
layout(location = 1) out vec2 uv;
layout(set = 0, binding = 0) uniform Transform { mat4 mvp; } transform;
out gl_PerVertex { vec4 gl_Position; };
void main() {
    gl_Position = transform.mvp * vec4(position, 1.0);
    color = vertexColor;
    uv = inUv;
}
