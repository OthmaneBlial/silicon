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
    vec3 albedo = clamp(base.rgb, vec3(0.0), vec3(1.0));
    float metallic = clamp(params.value.y, 0.0, 1.0);
    float roughness = clamp(params.value.w, 0.045, 1.0);
    vec3 n = normalize(normal);
    vec3 v = normalize(camera.position.xyz - world);
    vec3 l = normalize(vec3(-0.4, 0.85, 0.6));
    vec3 h = normalize(v + l);
    float nDotV = max(dot(n, v), 0.001);
    float nDotL = max(dot(n, l), 0.0);
    float nDotH = max(dot(n, h), 0.0);
    float vDotH = max(dot(v, h), 0.0);
    float alpha = roughness * roughness;
    float alpha2 = alpha * alpha;
    float d = nDotH * nDotH * (alpha2 - 1.0) + 1.0;
    float distribution = alpha2 / max(3.14159265 * d * d, 0.0001);
    float k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    float geometryV = nDotV / (nDotV * (1.0 - k) + k);
    float geometryL = nDotL / (nDotL * (1.0 - k) + k);
    vec3 f0 = mix(vec3(0.04), albedo, metallic);
    vec3 fresnel = f0 + (vec3(1.0) - f0) * pow(1.0 - vDotH, 5.0);
    vec3 specular = distribution * geometryV * geometryL * fresnel
        / max(4.0 * nDotV * nDotL, 0.001);
    vec3 diffuse = (vec3(1.0) - fresnel) * albedo * (1.0 - metallic) / 3.14159265;
    vec3 ambient = albedo * (0.035 * (1.0 - metallic));
    vec3 radiance = vec3(4.2, 3.8, 3.3);
    vec3 rgb = (diffuse + specular) * radiance * nDotL + ambient
        + albedo * params.value.z;
    float fog = clamp(length(world - camera.position.xyz) / 45.0, 0.0, 0.7);
    rgb = mix(rgb, vec3(0.022, 0.032, 0.05), fog);
    rgb = max(rgb, vec3(0.0));
    outColor = vec4(pow(rgb / (vec3(1.0) + rgb), vec3(1.0 / 2.2)), base.a);
}
