#version 450
layout(location = 0) in vec4 base;
layout(location = 0) out vec4 out0;
layout(location = 1) out vec4 out1;
layout(location = 2) out vec4 out2;
layout(location = 3) out vec4 out3;
void main() {
    out0 = base;
    out1 = base.zyxw;
    out2 = vec4(0.0, 1.0, 0.0, 1.0);
    out3 = vec4(1.0, 0.0, 1.0, 1.0);
}
