#version 450
layout(location = 0) in vec4 color;
layout(location = 1) in vec2 uv;
layout(location = 0) out vec4 outColor;
layout(set = 1, binding = 0) uniform sampler2D tex;
void main() {
    if (uv.x < 0.25) {
        if (uv.y < 0.5) discard;
    }
    vec4 base;
    if (uv.x < 0.65) {
        base = texture(tex, uv) * color;
    } else {
        base = vec4(0.95, 0.28, 0.08, 1.0);
    }
    if (uv.y > 0.8) {
        outColor = base;
        return;
    }
    outColor = base * vec4(0.5, 0.8, 1.0, 1.0);
}
