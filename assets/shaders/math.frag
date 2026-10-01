#version 450
layout(location = 0) in vec4 value;
layout(location = 0) out vec4 outColor;

void main() {
    outColor = vec4(
        floor(value.x) + ceil(value.x) + trunc(value.x) + round(value.x) + roundEven(value.x),
        fract(value.y), sin(value.z), cos(value.w));
}
