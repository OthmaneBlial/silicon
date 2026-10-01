#version 450
layout(location = 0) in vec3 position;
layout(location = 0) out vec4 base;
out gl_PerVertex { vec4 gl_Position; };
void main() {
    gl_Position = vec4(position.xy, 0.5, 1.0);
    base = vec4(0.25, 0.5, 0.75, 1.0);
}
