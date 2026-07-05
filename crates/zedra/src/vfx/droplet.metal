#include <metal_stdlib>
using namespace metal;

// Injected by droplet.rs at compile time; the default keeps offline builds working.
#ifndef TRAIL_LEN
#define TRAIL_LEN 5u
#endif

struct DropletUniforms {
    float2 viewport_size;
    float2 bbox_origin;
    float2 bbox_size;
    float2 grab_size;
    float2 center;
    float radius;
    float _pad;
    float2 trail[TRAIL_LEN];
    float2 _pad2;
    float4 base_color;
};

struct DropletVaryings {
    float4 position [[position]];
    float2 px;
};

vertex DropletVaryings droplet_vertex(
    uint vertex_id [[vertex_id]],
    constant DropletUniforms &u [[buffer(0)]]
) {
    float2 corner = float2(float(vertex_id & 1u), float(vertex_id >> 1u));
    float2 px = u.bbox_origin + corner * u.bbox_size;
    float2 ndc = px / u.viewport_size * 2.0 - 1.0;
    DropletVaryings out;
    out.position = float4(ndc.x, -ndc.y, 0.0, 1.0);
    out.px = px;
    return out;
}

// Polynomial smooth min (Inigo Quilez); joins blobs into one liquid surface.
static float smin(float a, float b, float k) {
    float h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

// Head blob unioned with tapered followers; followers sit on past positions.
static float droplet_sdf(float2 px, constant DropletUniforms &u) {
    float k = u.radius * 0.55;
    float d = length(px - u.center) - u.radius;
    float taper = 0.72;
    for (uint i = 0; i < TRAIL_LEN; i++) {
        float follower_radius = u.radius * taper;
        // Shrink caught-up followers so a resting droplet reads as one circle.
        float lag = length(u.trail[i] - u.center);
        follower_radius *= smoothstep(0.0, u.radius * 0.8, lag) * 0.85 + 0.15;
        float di = length(px - u.trail[i]) - follower_radius;
        d = smin(d, di, k);
        taper *= 0.78;
    }
    return d;
}

fragment float4 droplet_fragment(
    DropletVaryings in [[stage_in]],
    constant DropletUniforms &u [[buffer(0)]],
    texture2d<float> grab [[texture(0)]]
) {
    constexpr sampler grab_sampler(address::clamp_to_edge, filter::linear);

    float d = droplet_sdf(in.px, u);

    float aa = max(fwidth(d), 0.5);
    float coverage = 1.0 - smoothstep(-aa, aa, d);
    if (coverage <= 0.001) {
        discard_fragment();
    }

    // Fake a spherical surface: flat in the middle, steep at the rim.
    float depth = clamp(-d / u.radius, 0.0, 1.0);
    float rim = 1.0 - depth;
    float2 gradient = float2(dfdx(d), dfdy(d));
    float gradient_len = length(gradient);
    float2 outward = gradient_len > 0.0001 ? gradient / gradient_len : float2(0.0, 0.0);
    float3 normal = normalize(float3(outward * rim, mix(0.35, 1.0, depth)));

    // Rim lensing. Grab texture is bucket-sized: clamp to the blitted bbox,
    // normalize by texture size.
    float2 refract_offset = -outward * rim * rim * u.radius * 0.32;
    float2 base_px = in.px + refract_offset - u.bbox_origin;
    float2 sample_max = u.bbox_size - 1.0;
    float2 uv_r = clamp(base_px + refract_offset * -0.06, float2(0.0), sample_max) / u.grab_size;
    float2 uv_g = clamp(base_px, float2(0.0), sample_max) / u.grab_size;
    float2 uv_b = clamp(base_px + refract_offset * 0.06, float2(0.0), sample_max) / u.grab_size;
    float3 color = float3(
        grab.sample(grab_sampler, uv_r).r,
        grab.sample(grab_sampler, uv_g).g,
        grab.sample(grab_sampler, uv_b).b
    );

    // Near-clear water tinted toward the theme-inverse base, strongest at the rim.
    float3 base = u.base_color.rgb;
    color = mix(color, base, 0.03 + rim * 0.08);
    color *= 1.0 - rim * 0.05;

    float3 light = normalize(float3(-0.4, -0.55, 0.75));
    float specular = pow(max(dot(normal, light), 0.0), 64.0);
    float sheen = pow(max(dot(normal, light), 0.0), 8.0);
    color += (specular * 0.28 + sheen * 0.04) * mix(base, float3(1.0), 0.4);

    return float4(color, coverage);
}
