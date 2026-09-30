#version 450
layout(location = 0) in vec4 color;
layout(location = 1) in vec2 uv;
layout(location = 2) in vec3 normal;
layout(location = 3) in vec3 world;
layout(location = 0) out vec4 outColor;
layout(set = 1, binding = 0) uniform sampler2D tex;
layout(set = 1, binding = 1) uniform sampler2D shadowMap;
layout(set = 0, binding = 3) uniform MaterialColor { vec4 value; } material;
layout(set = 0, binding = 4) uniform MaterialParams { vec4 value; } params;
layout(set = 0, binding = 5) uniform Camera { vec4 position; } camera;
layout(set = 0, binding = 6) uniform LightMatrix { mat4 matrix; } lightMatrix;
layout(set = 0, binding = 7) uniform ShadowSettings { vec4 value; } shadowSettings;
void main() {
    vec4 texel = mix(vec4(1.0), texture(tex, uv), params.value.x);
    vec4 base = material.value * texel * color;
    vec3 n = normalize(normal);
    vec3 light = normalize(vec3(-0.4, 0.85, 0.6));
    vec4 projected = lightMatrix.matrix * vec4(world, 1.0);
    vec3 shadowCoordinate = projected.xyz / projected.w;
    shadowCoordinate.xy = vec2(shadowCoordinate.x * 0.5 + 0.5, 0.5 - shadowCoordinate.y * 0.5);
    float visibility = 1.0;
    if (shadowCoordinate.x >= 0.0 && shadowCoordinate.x <= 1.0
        && shadowCoordinate.y >= 0.0 && shadowCoordinate.y <= 1.0
        && shadowCoordinate.z >= 0.0 && shadowCoordinate.z <= 1.0) {
        float storedDepth = textureLod(shadowMap, shadowCoordinate.xy, 0.0).r;
        float bias = shadowSettings.value.x * max(0.25, 1.0 - max(dot(n, light), 0.0));
        visibility = shadowCoordinate.z - bias > storedDepth ? shadowSettings.value.y : 1.0;
    }
    float diffuse = max(dot(n, light), 0.0);
    vec3 view = normalize(camera.position.xyz - world);
    float specular = pow(max(dot(n, normalize(view + light)), 0.0), 64.0)
        * (0.35 + params.value.y * 1.5);
    vec3 delta = vec3(2.0, 3.0, -2.0) - world;
    float point = max(dot(n, normalize(delta)), 0.0) * 5.0 / (1.0 + dot(delta, delta));
    float ambient = 0.16 + 0.12 * max(n.y, 0.0);
    vec3 rgb = base.rgb * (ambient + diffuse * 0.85 * visibility + params.value.z)
        + vec3(1.0, 0.86, 0.68) * specular * visibility + vec3(0.08, 0.65, 0.95) * point;
    float fog = clamp(length(world - camera.position.xyz) / 45.0, 0.0, 0.7);
    rgb = mix(rgb, vec3(0.022, 0.032, 0.05), fog);
    rgb = max(rgb, vec3(0.0));
    outColor = vec4(pow(rgb / (vec3(1.0) + rgb), vec3(1.0 / 2.2)), base.a);
}
