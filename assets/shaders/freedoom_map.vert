#version 450
layout(location = 0) in vec3 inPosition;
layout(location = 1) in vec4 inColor;
layout(location = 0) out vec4 color;
layout(set = 0, binding = 0) uniform Transform { mat4 matrix; } transform;
out gl_PerVertex { vec4 gl_Position; };
void main() {
    gl_Position = transform.matrix * vec4(inPosition, 1.0);
    color = inColor;
}
