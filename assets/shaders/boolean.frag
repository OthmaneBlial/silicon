#version 450
layout(location = 0) in vec4 color;
layout(location = 1) in vec2 uv;
layout(location = 0) out vec4 outColor;
void main() {
    bool a = uv.x <= 0.5;
    bool b = uv.y >= 0.5;
    bool same = a == b;
    bool opposite = a != b;
    float weight = ((same && !a) || opposite) ? 0.8 : 0.2;
    outColor = color * weight;
}
