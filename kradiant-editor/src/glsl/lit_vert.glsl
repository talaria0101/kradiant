#version 120
attribute vec3 a_pos;
attribute vec3 a_normal;
uniform mat4 u_mvp;
varying vec3 v_normal;
void main() {
    gl_Position = u_mvp * vec4(a_pos, 1.0);
    v_normal = a_normal;
}
