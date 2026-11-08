// The prism lance's scope lens for the OpenGL headset renderer (src/glvr, src/scope.rs): a disc
// showing the scope's view (drawn just before into u_scope, layer 0) with a reticle in the
// team's colour and a dark rim; dark glass with a faint sheen while not held up to an eye.
// Compiled with VERTEX or FRAGMENT defined; both eyes in one pass (GL_OVR_multiview2).

uniform mat4 u_view_proj[2];
uniform mat4 u_model;
uniform sampler2DArray u_scope;
// 1: the view is there (the scope at an eye), 0: dark glass.
uniform float u_on;
// The reticle's colour (linear rgb).
uniform vec4 u_colour;

#ifdef VERTEX
layout(num_views = NUM_VIEWS) in;
layout(location = 0) in vec3 a_position;
layout(location = 2) in vec2 a_uv;
out vec2 v_uv;

void main() {
    v_uv = a_uv;
    gl_Position = u_view_proj[VIEW] * (u_model * vec4(a_position, 1.0));
}
#endif

#ifdef FRAGMENT
in vec2 v_uv;
layout(location = 0) out vec4 o_colour;

void main() {
    vec2 p = v_uv * 2.0 - 1.0;
    float r = length(p);
    float aa = fwidth(r) * 1.5;
    vec3 c;
    if (u_on > 0.5) {
        c = texture(u_scope, vec3(v_uv, 0.0)).rgb;
        // The reticle: a fine cross with a gap in the middle and a dot, fading out towards the
        // rim.
        float w = 0.012;
        float line = max(
            1.0 - smoothstep(w - aa, w + aa, abs(p.x)),
            1.0 - smoothstep(w - aa, w + aa, abs(p.y))
        );
        line *= smoothstep(0.1, 0.12, r) * (1.0 - smoothstep(0.6, 0.7, r));
        float dot_ = 1.0 - smoothstep(0.025 - aa, 0.025 + aa, r);
        c = mix(c, u_colour.rgb, max(line, dot_) * 0.9);
        // Darker towards the rim, as through a tube.
        c *= 1.0 - smoothstep(0.75, 1.0, r) * 0.85;
    } else {
        // Dark glass, a soft highlight up and to the left.
        float sheen = 1.0 - smoothstep(0.0, 0.5, length(p - vec2(-0.35, 0.4)));
        c = vec3(0.006, 0.008, 0.014) + vec3(0.05, 0.06, 0.08) * sheen;
    }
    // A thin rim, antialiased at the edge.
    c *= 1.0 - smoothstep(1.0 - 2.0 * aa, 1.0, r);
    o_colour = vec4(c, 1.0);
}
#endif
