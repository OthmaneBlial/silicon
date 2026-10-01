#version 450
layout(location = 0) in vec4 value;
layout(location = 0) out vec4 outColor;

void main() {
    outColor = vec4(floor(value.x), fract(value.y), sin(value.z), cos(value.w));
}
