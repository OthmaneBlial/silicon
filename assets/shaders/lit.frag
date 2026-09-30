#version 450
layout(location = 0) in vec4 color;
layout(location = 1) in vec2 uv;
layout(location = 2) in vec3 normal;
layout(location = 3) in vec3 world;
layout(location = 0) out vec4 outColor;
layout(set = 1, binding = 0) uniform sampler2D tex;
layout(set = 0, binding = 3) uniform MaterialColor { vec4 value; } material;
layout(set = 0, binding = 4) uniform MaterialParams { vec4 value; } params;
layout(set = 0, binding = 5) uniform Camera { vec4 position; } camera;
void main() {
    vec4 texel = mix(vec4(1.0), texture(tex, uv), params.value.x);
    vec4 base = material.value * texel * color;
    vec3 n = normalize(normal);
    vec3 light = normalize(vec3(-0.4, 0.85, 0.6));
    float diffuse = max(dot(n, light), 0.0);
    vec3 view = normalize(camera.position.xyz - world);
    float specular = pow(max(dot(n, normalize(view + light)), 0.0), 64.0)
        * (0.35 + params.value.y * 1.5);
    vec3 delta = vec3(2.0, 3.0, -2.0) - world;
    float point = max(dot(n, normalize(delta)), 0.0) * 5.0 / (1.0 + dot(delta, delta));
    float ambient = 0.16 + 0.12 * max(n.y, 0.0);
    vec3 rgb = base.rgb * (ambient + diffuse * 0.85 + params.value.z)
        + vec3(1.0, 0.86, 0.68) * specular + vec3(0.08, 0.65, 0.95) * point;
    float fog = clamp(length(world - camera.position.xyz) / 45.0, 0.0, 0.7);
    rgb = mix(rgb, vec3(0.022, 0.032, 0.05), fog);
    rgb = max(rgb, vec3(0.0));
    outColor = vec4(pow(rgb / (vec3(1.0) + rgb), vec3(1.0 / 2.2)), base.a);
}
