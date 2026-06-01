#version 120
uniform sampler2D u_tex;  // Texture sampler
uniform vec4 u_color;
uniform vec3 u_light_dir;
uniform float u_ambient;
varying vec3 v_normal;
varying vec2 v_uv;
void main() {
    vec3 n = normalize(v_normal);
    float d = max(dot(n, u_light_dir), max(dot(-n, u_light_dir), 0.0));
    float light = u_ambient + (1.0 - u_ambient) * d;

    vec4 tex_color = texture2D(u_tex, v_uv);  // Sample texture
    gl_FragColor = vec4(tex_color.rgb * u_color.rgb * light, tex_color.a * u_color.a);
}
