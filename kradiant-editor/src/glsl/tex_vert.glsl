#version 120
attribute vec3 a_pos;
attribute vec3 a_normal;
attribute vec2 a_uv;        // New: texture coordinates
uniform mat4 u_mvp;
varying vec3 v_normal;
varying vec2 v_uv;          // Pass to fragment shader
void main() {
    gl_Position = u_mvp * vec4(a_pos, 1.0);
    v_normal = a_normal;
    v_uv = a_uv;
}
