#version 450
layout(location = 1) in vec3 direction;
layout(location = 0) out vec4 outColor;
layout(set = 1, binding = 0) uniform samplerCube environmentMap;
void main() {
    outColor = texture(environmentMap, direction);
}
