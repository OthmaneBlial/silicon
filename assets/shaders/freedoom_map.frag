#version 450
layout(location = 0) in vec4 color;
layout(location = 1) in vec2 uv;
layout(location = 0) out vec4 outColor;
layout(set = 1, binding = 0) uniform sampler2D tex;
void main() {
    vec4 sampleColor = texture(tex, uv);
    outColor = vec4(sampleColor.rgb * color.rgb, 1.0);
}
