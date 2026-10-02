#version 450
layout(location = 0) in vec3 color;
layout(location = 0) out vec4 outColor;

void main() {
    float value = 0.0;
    float i = 0.0;
    while (i < 10.0) {
        if (i >= 3.0) {
            break;
        }
        value += color.x;
        i += 1.0;
    }
    outColor = vec4(value, color.y, color.z, 1.0);
}
