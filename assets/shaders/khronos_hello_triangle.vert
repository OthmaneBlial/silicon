#version 450
/* Copyright (c) 2025, Sascha Willems
 *
 * SPDX-License-Identifier: Apache-2.0
 *
 * Derived from KhronosGroup/Vulkan-Samples, hello_triangle, at commit
 * 177edebf0cd7d4f669667e49f052cfb56b17e004.
 * Modified for SILICON: the fixed vertex contract stores RGBA color, so this
 * input accepts vec4 and forwards its RGB components to the original varying.
 * See docs/third-party-demo.md for source and adaptation details.
 */

layout(location = 0) in vec3 inPosition;
layout(location = 1) in vec4 inColor;

layout(location = 0) out vec3 outColor;
out gl_PerVertex { vec4 gl_Position; };

void main()
{
    outColor = inColor.rgb;
    gl_Position = vec4(inPosition, 1.0);
}
