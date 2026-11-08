// Plain lit surfaces for the OpenGL headset renderer (src/glvr): the StandardMaterial meshes
// (the arena's dark body, the suits). The base colour lit by the ambient light and the one
// directional light, with a faint highlight, tone mapped as `--vr`'s; not Bevy's PBR.
// Compiled with VERTEX or FRAGMENT defined; both eyes in one pass (GL_OVR_multiview2).

uniform mat4 u_view_proj[2];
uniform vec3 u_eye[2];
uniform mat4 u_model;
uniform mat3 u_normal;
// Linear RGBA.
uniform vec4 u_base;
// Towards the light (world), its colour times its strength, the ambient light.
uniform vec3 u_light_dir;
uniform vec3 u_light_colour;
uniform vec3 u_ambient;
// Point lights (the bolts'): xyz position, w range; colour times strength at 1 m; how many.
uniform vec4 u_point_pos[16];
uniform vec3 u_point_colour[16];
uniform int u_point_count;

#ifdef VERTEX
layout(num_views = NUM_VIEWS) in;
layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
out vec3 v_world;
out vec3 v_normal;

void main() {
    vec4 world = u_model * vec4(a_position, 1.0);
    v_world = world.xyz;
    v_normal = u_normal * a_normal;
    gl_Position = u_view_proj[VIEW] * world;
}
#endif

#ifdef FRAGMENT
in vec3 v_world;
in vec3 v_normal;
layout(location = 0) out vec4 o_colour;

// The eye cameras' tone mapping in `--vr` (ACES fitted, Narkowicz): its toe keeps the dark
// surfaces near black.
vec3 aces(vec3 x) {
    return clamp(x * (2.51 * x + 0.03) / (x * (2.43 * x + 0.59) + 0.14), 0.0, 1.0);
}

void main() {
    vec3 n = v_normal;
    if (dot(n, n) < 1e-8) {
        n = cross(dFdx(v_world), dFdy(v_world));
    }
    n = normalize(n);
    if (!gl_FrontFacing) {
        n = -n;
    }
    vec3 to_eye = normalize(u_eye[VIEW] - v_world);
    float diffuse = max(dot(n, u_light_dir), 0.0);
    // A dull Blinn highlight, so the black surfaces show their shape.
    vec3 h = normalize(u_light_dir + to_eye);
    float spec = pow(max(dot(n, h), 0.0), 24.0) * 0.04;
    vec3 rgb = u_base.rgb * (u_ambient + u_light_colour * diffuse) + u_light_colour * spec;
    // Point lights: inverse square, windowed to their range as Bevy's; a sharper highlight
    // so the glossy black walls show the bolts passing.
    for (int i = 0; i < u_point_count; i++) {
        vec3 d = u_point_pos[i].xyz - v_world;
        float dist2 = max(dot(d, d), 1e-4);
        float r = u_point_pos[i].w;
        float k = dist2 / (r * r);
        float window = clamp(1.0 - k * k, 0.0, 1.0);
        vec3 l = d * inversesqrt(dist2);
        vec3 c = u_point_colour[i] * window * window / dist2;
        vec3 hp = normalize(l + to_eye);
        rgb += c * (u_base.rgb * max(dot(n, l), 0.0) + pow(max(dot(n, hp), 0.0), 48.0) * 0.35);
    }
    o_colour = vec4(aces(rgb), 1.0);
}
#endif
