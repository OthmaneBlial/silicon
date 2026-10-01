#version 450
layout(location = 0) in vec4 value;
layout(location = 0) out vec4 outColor;

void main() {
    float positive = value.x * value.x + 1.0;
    outColor = vec4(
        floor(value.x) + ceil(value.x) + trunc(value.x) + round(value.x) + roundEven(value.x),
        fract(value.y) + sqrt(positive) + inversesqrt(positive),
        sin(value.z) + cos(value.w),
        exp(value.z * 0.1) + exp2(value.z * 0.1) + log(positive) + log2(positive));
}
