#version 450
layout(location = 0) in vec4 color;
layout(location = 0) out vec4 outColor;
layout(set = 0, binding = 6) uniform Offset { vec2 value; } offset;
layout(set = 0, binding = 7) uniform Gain { float value; } gain;
layout(set = 0, binding = 8) uniform Bias { vec3 value; } bias;
void main() {
    vec2 v = color.xy;
    vec2 before = v;
    v.y = gain.value;
    vec2 n = normalize(v);
    vec4 combined = vec4(before, length(v), normalize(color).w);
    outColor = combined + vec4(offset.value + n, min(gain.value, 3.0), bias.value.z);
}
