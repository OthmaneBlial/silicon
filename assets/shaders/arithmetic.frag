#version 450
layout(location = 0) in vec4 color;
layout(location = 1) in vec2 uv;
layout(location = 0) out vec4 outColor;
layout(set = 1, binding = 0) uniform sampler2D tex;
void main() {
    outColor = texture(tex, uv) * color
        * dot((uv.yx + vec2(2.0, 1.0) - vec2(0.5, 0.25))
            / vec2(4.0, 2.0), vec2(0.5, 0.25));
}
