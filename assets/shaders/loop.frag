#version 450
layout(location = 0) in vec3 color;
layout(location = 0) out vec4 outColor;

void main() {
    float value = 0.0;
    for (float i = 0.0; i < 3.0; i += 1.0) {
        value += color.x;
    }
    outColor = vec4(value, color.y, color.z, 1.0);
}
