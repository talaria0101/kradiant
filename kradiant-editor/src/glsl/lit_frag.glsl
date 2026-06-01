#version 120
uniform vec4 u_color;
uniform vec3 u_light_dir;   // normalized, world space
uniform float u_ambient;
varying vec3 v_normal;
void main() {
    vec3 n = normalize(v_normal);
    // Sample both sides so back-faces aren't black
    float d = max(dot(n, u_light_dir), max(dot(-n, u_light_dir), 0.0));
    float light = u_ambient + (1.0 - u_ambient) * d;
    gl_FragColor = vec4(u_color.rgb * light, u_color.a);
}
