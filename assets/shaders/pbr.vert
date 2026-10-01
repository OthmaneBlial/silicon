#version 450
layout(location = 0) in vec3 position;
layout(location = 1) in vec4 tangentAndSign;
layout(location = 2) in vec2 inUv;
layout(location = 3) in vec3 inNormal;
layout(location = 0) out vec4 tangent;
layout(location = 1) out vec2 uv;
layout(location = 2) out vec3 normal;
layout(location = 3) out vec3 world;
layout(set = 0, binding = 0) uniform Projection { mat4 matrix; } mvp;
layout(set = 0, binding = 1) uniform Model { mat4 matrix; } model;
layout(set = 0, binding = 2) uniform NormalTransform { mat4 matrix; } normalTransform;
out gl_PerVertex { vec4 gl_Position; };
void main() {
    vec4 localPosition = vec4(position, 1.0);
    gl_Position = mvp.matrix * localPosition;
    world = (model.matrix * localPosition).xyz;
    normal = (normalTransform.matrix * vec4(inNormal, 0.0)).xyz;
    tangent = vec4((model.matrix * vec4(tangentAndSign.xyz, 0.0)).xyz, tangentAndSign.w);
    uv = inUv;
}
