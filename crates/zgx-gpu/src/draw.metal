// zgx entry points: fs=fs fs_basic=fs_basic fs_basic_both=fs_basic_both fs_both=fs_both vs=vs vs_basic=vs_basic
// language: metal2.4
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size2;
};

struct Vertex {
    metal::float4 rect;
    metal::float4 uv;
    metal::float4 color;
    metal::float4 fade;
    metal::float4 options;
    metal::float4 shape;
    metal::float4 border;
    metal::float4 mask;
};
struct Out {
    metal::float4 position;
    metal::float2 uv;
    char _pad2[8];
    metal::float4 color;
    metal::float2 point;
    char _pad4[8];
    metal::float4 fade;
    metal::float4 options;
    metal::float4 shape;
    metal::float4 border;
    metal::float4 mask;
};
struct Region {
    metal::float4 round0_;
    metal::float4 radii0_;
    metal::float4 round1_;
    metal::float4 radii1_;
    metal::float4 fade_box;
    metal::float4 fade;
    metal::float4 color0_;
    metal::float4 color1_;
    metal::float4 color2_;
    metal::float4 tint;
};
typedef Region type_6[1];
struct Both {
    metal::float4 retained;
    metal::float4 drawable;
};
struct Basic {
    metal::float4 position;
    metal::float2 uv;
    char _pad2[8];
    metal::float4 color;
    float texture;
    float firmness;
    char _pad5[8];
};

float firmness(
    float options_y,
    metal::float3 text
) {
    bool local = {};
    if (!((options_y < 1.1))) {
        local = options_y > 1.5;
    } else {
        local = true;
    }
    bool _e10 = local;
    if (_e10) {
        return 0.0;
    }
    return metal::clamp(metal::dot(text, metal::float3(0.2126, 0.7152, 0.0722)) * 2.0, 0.0, 1.0);
}

float firm(
    float a,
    float k
) {
    return a + (((k * 0.67) * a) * (1.0 - a));
}

float rounded(
    metal::float4 b,
    metal::float4 r,
    float feather,
    metal::float2 p,
    constant metal::float4& viewport
) {
    if (b.z < 0.0) {
        return 1.0;
    }
    metal::float2 half_ = b.zw * 0.5;
    metal::float2 q_1 = (p - b.xy) - half_;
    float radius_1 = (q_1.y < 0.0) ? ((q_1.x > 0.0) ? r.y : r.x) : ((q_1.x > 0.0) ? r.z : r.w);
    metal::float2 d = (metal::abs(q_1) - half_) + metal::float2(radius_1);
    float sdf = (metal::length(metal::max(d, metal::float2(0.0))) + metal::min(metal::max(d.x, d.y), 0.0)) - radius_1;
    if (feather > 0.0) {
        float t = metal::clamp(-(sdf) / feather, 0.0, 1.0);
        return (t * t) * (3.0 - (2.0 * t));
    }
    float _e60 = viewport.z;
    return metal::clamp(0.5 - (sdf * _e60), 0.0, 1.0);
}

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

metal::float4 tinted(
    float index,
    metal::float4 premultiplied,
    device type_6 const& regions,
    constant _mslBufferSizes& _buffer_sizes
) {
    bool local_1 = {};
    if (index < 0.5) {
        return premultiplied;
    }
    Region g = regions[naga_f2u32(index) - 1u];
    if (!((g.tint.x < 0.5))) {
        local_1 = premultiplied.w <= 0.0;
    } else {
        local_1 = true;
    }
    bool _e21 = local_1;
    if (_e21) {
        return premultiplied;
    }
    metal::float4 c = metal::float4(premultiplied.xyz / metal::float3(premultiplied.w), 1.0);
    metal::float3 rgb_1 = metal::clamp(metal::float3(metal::dot(g.color0_, c), metal::dot(g.color1_, c), metal::dot(g.color2_, c)), metal::float3(0.0), metal::float3(1.0));
    return metal::float4(rgb_1 * premultiplied.w, premultiplied.w);
}

float faded(
    metal::float4 b_1,
    metal::float4 f,
    metal::float2 p_1
) {
    float k_1 = 1.0;
    if (f.x > 0.0) {
        float t_1 = metal::clamp((p_1.y - b_1.y) / f.x, 0.0, 1.0);
        float _e16 = k_1;
        k_1 = _e16 * ((t_1 * t_1) * (3.0 - (2.0 * t_1)));
    }
    if (f.y > 0.0) {
        float t_2 = metal::clamp((b_1.z - p_1.x) / f.y, 0.0, 1.0);
        float _e35 = k_1;
        k_1 = _e35 * ((t_2 * t_2) * (3.0 - (2.0 * t_2)));
    }
    if (f.z > 0.0) {
        float t_3 = metal::clamp((b_1.w - p_1.y) / f.z, 0.0, 1.0);
        float _e54 = k_1;
        k_1 = _e54 * ((t_3 * t_3) * (3.0 - (2.0 * t_3)));
    }
    if (f.w > 0.0) {
        float t_4 = metal::clamp((p_1.x - b_1.x) / f.w, 0.0, 1.0);
        float _e73 = k_1;
        k_1 = _e73 * ((t_4 * t_4) * (3.0 - (2.0 * t_4)));
    }
    float _e81 = k_1;
    return _e81;
}

float region(
    float index_1,
    metal::float2 position_1,
    constant metal::float4& viewport,
    device type_6 const& regions,
    constant _mslBufferSizes& _buffer_sizes
) {
    float k_2 = {};
    Region g_1 = regions[naga_f2u32(index_1) - 1u];
    float _e10 = viewport.z;
    metal::float2 p_2 = position_1 / metal::float2(_e10);
    float _e17 = rounded(g_1.round0_, g_1.radii0_, g_1.tint.y, p_2, viewport);
    float _e22 = rounded(g_1.round1_, g_1.radii1_, g_1.tint.z, p_2, viewport);
    k_2 = _e17 * _e22;
    float _e25 = k_2;
    float _e28 = faded(g_1.fade_box, g_1.fade, p_2);
    k_2 = _e25 * _e28;
    if (g_1.tint.w > 0.5) {
        Region o_2 = regions[naga_f2u32(g_1.tint.w) - 1u];
        float _e42 = k_2;
        float _e45 = faded(o_2.fade_box, o_2.fade, p_2);
        k_2 = _e42 * _e45;
    }
    float _e47 = k_2;
    return _e47;
}

metal::float3 corner_sdf(
    metal::float2 q,
    float radius,
    float k_3
) {
    bool local_2 = {};
    bool local_3 = {};
    bool local_4 = {};
    metal::float2 o_3 = metal::max(q, metal::float2(0.0));
    if (k_3 > 1.01) {
        local_2 = o_3.x > 0.0;
    } else {
        local_2 = false;
    }
    bool _e14 = local_2;
    if (_e14) {
        local_3 = o_3.y > 0.0;
    } else {
        local_3 = false;
    }
    bool _e21 = local_3;
    if (_e21) {
        local_4 = radius > 0.0;
    } else {
        local_4 = false;
    }
    bool _e27 = local_4;
    if (_e27) {
        float n = metal::exp2(k_3);
        metal::float2 u = o_3 / metal::float2(radius);
        float s = metal::pow(metal::pow(u.x, n) + metal::pow(u.y, n), 1.0 / n);
        metal::float2 g_2 = metal::pow(u / metal::float2(metal::max(s, 0.000001)), metal::float2(n - 1.0));
        float gl = metal::max(metal::length(g_2), 0.000001);
        return metal::float3(((s - 1.0) * radius) / gl, g_2 / metal::float2(gl));
    }
    float reach = metal::length(o_3);
    metal::float2 gradient = (reach > 0.0) ? (o_3 / metal::float2(metal::max(reach, 0.00000000000000000001))) : ((q.x > q.y) ? metal::float2(1.0, 0.0) : metal::float2(0.0, 1.0));
    return metal::float3((reach + metal::min(metal::max(q.x, q.y), 0.0)) - radius, gradient);
}

metal::float2 centred(
    metal::float2 at,
    metal::float4 box,
    constant metal::float4& viewport
) {
    float _e9 = viewport.z;
    metal::float2 middle = metal::rint(((box.xy + (box.zw * 0.5)) * _e9) * 64.0) / metal::float2(64.0);
    float _e21 = viewport.z;
    return metal::abs(at - middle) / metal::float2(_e21);
}

metal::float4 shade_texel(
    Out input,
    metal::float4 given,
    bool has_texel,
    constant metal::float4& viewport,
    metal::texture2d<float, metal::access::sample> atlas,
    device type_6 const& regions,
    metal::sampler tex_sampler,
    constant _mslBufferSizes& _buffer_sizes
) {
    Out v_5 = {};
    metal::float4 color_1 = {};
    bool local_5 = {};
    bool local_6 = {};
    float corner = {};
    bool local_7 = {};
    bool premultiplied_1 = false;
    metal::float3 rgb = metal::float3(0.0);
    float scale = 1.0;
    metal::float4 tex = {};
    v_5 = input;
    float _e6 = v_5.options.z;
    if (_e6 < 0.5) {
        metal::float4 _e11 = v_5.position;
        float _e15 = viewport.z;
        v_5.point = _e11.xy / metal::float2(_e15);
    }
    metal::float4 _e19 = v_5.color;
    color_1 = _e19;
    float _e23 = v_5.options.z;
    if (_e23 < 0.5) {
        float _e30 = v_5.shape.w;
        if (!((_e30 > 0.5))) {
            float _e38 = v_5.shape.z;
            local_6 = _e38 > 0.0;
        } else {
            local_6 = true;
        }
        bool _e42 = local_6;
        local_5 = _e42;
    } else {
        local_5 = false;
    }
    bool _e44 = local_5;
    if (_e44) {
        float _e47 = v_5.shape.x;
        corner = _e47;
        float _e51 = v_5.options.w;
        if (_e51 > 0.5) {
            float _e56 = v_5.point.x;
            float _e59 = v_5.fade.x;
            float _e62 = v_5.fade.z;
            bool right = _e56 > (_e59 + (_e62 * 0.5));
            float _e69 = v_5.point.y;
            float _e72 = v_5.fade.y;
            float _e75 = v_5.fade.w;
            bool bottom = _e69 > (_e72 + (_e75 * 0.5));
            float _e82 = v_5.border.x;
            float _e85 = v_5.border.y;
            float _e89 = v_5.border.w;
            float _e92 = v_5.border.z;
            corner = bottom ? (right ? _e92 : _e89) : (right ? _e85 : _e82);
        }
        float _e95 = corner;
        float _e98 = v_5.fade.z;
        float _e101 = v_5.fade.w;
        float radius_2 = metal::clamp(_e95, 0.0, metal::min(_e98, _e101) * 0.5);
        float _e109 = v_5.shape.w;
        float _e112 = v_5.fade.z;
        float _e115 = v_5.fade.w;
        float k_4 = (radius_2 < ((metal::min(_e112, _e115) * 0.5) - 0.25)) ? _e109 : 1.0;
        metal::float4 _e125 = v_5.position;
        metal::float4 _e128 = v_5.fade;
        metal::float2 _e129 = centred(_e125.xy, _e128, viewport);
        metal::float4 _e131 = v_5.fade;
        metal::float2 q_2 = _e129 - ((_e131.zw * 0.5) - metal::float2(radius_2));
        metal::float3 _e138 = corner_sdf(q_2, radius_2, k_4);
        float distance = _e138.x;
        float _e144 = viewport.z;
        metal::float2 slope = metal::abs(_e138.yz) / metal::float2(_e144);
        float _e149 = v_5.shape.y;
        if (_e149 < 0.0) {
            float _e154 = v_5.shape.x;
            uint a_1 = naga_f2u32(_e154);
            float _e158 = v_5.shape.z;
            uint b_2 = naga_f2u32(_e158);
            metal::float4 sides = metal::float4(static_cast<float>(a_1 & 4095u), static_cast<float>(a_1 >> 12u), static_cast<float>(b_2 & 4095u), static_cast<float>(b_2 >> 12u)) / metal::float4(16.0);
            float _e178 = v_5.point.x;
            float _e181 = v_5.fade.x;
            float _e184 = v_5.fade.z;
            bool right_1 = _e178 > (_e181 + (_e184 * 0.5));
            float _e191 = v_5.point.y;
            float _e194 = v_5.fade.y;
            float _e197 = v_5.fade.w;
            bool bottom_1 = _e191 > (_e194 + (_e197 * 0.5));
            float across = right_1 ? sides.y : sides.w;
            float down = bottom_1 ? sides.z : sides.x;
            metal::float4 _e209 = v_5.fade;
            metal::float4 _e214 = v_5.fade;
            metal::float4 box_1 = metal::float4(_e209.xy + sides.wx, metal::max((_e214.zw - sides.wx) - sides.yz, metal::float2(0.0)));
            float inner_radius = metal::clamp(radius_2 - ((across + down) * 0.5), 0.0, metal::min(box_1.z, box_1.w) * 0.5);
            metal::float4 _e236 = v_5.position;
            metal::float2 _e238 = centred(_e236.xy, box_1, viewport);
            metal::float2 iq = _e238 - ((box_1.zw * 0.5) - metal::float2(inner_radius));
            metal::float3 _e245 = corner_sdf(iq, inner_radius, k_4);
            float inside = _e245.x;
            float aa = metal::max(slope.x + slope.y, 0.5);
            if (box_1.z > 0.0) {
                local_7 = box_1.w > 0.0;
            } else {
                local_7 = false;
            }
            bool _e267 = local_7;
            float hollow = _e267 ? metal::clamp(0.5 - (inside / aa), 0.0, 1.0) : 0.0;
            float _e271 = color_1.w;
            color_1.w = _e271 * metal::max(metal::clamp(0.5 - (distance / aa), 0.0, 1.0) - hollow, 0.0);
        } else {
            float _e284 = v_5.shape.z;
            if (_e284 > 0.0) {
                float _e289 = v_5.shape.z;
                float x = distance / (_e289 * 1.4142135);
                float x2_ = x * x;
                float e = metal::sign(x) * metal::sqrt(1.0 - metal::exp((-(x2_) * (1.2732395 + (0.147 * x2_))) / (1.0 + (0.147 * x2_))));
                float _e312 = color_1.w;
                color_1.w = _e312 * (0.5 - (0.5 * e));
                float _e320 = v_5.shape.y;
                if (_e320 > 500.0) {
                    float _e325 = v_5.shape.y;
                    float spread = _e325 - 1000.0;
                    metal::float4 _e329 = v_5.fade;
                    metal::float2 _e332 = v_5.uv;
                    metal::float4 _e337 = v_5.fade;
                    metal::float4 box_2 = metal::float4((_e329.xy - _e332) + metal::float2(spread), metal::max(_e337.zw - metal::float2(2.0 * spread), metal::float2(0.0)));
                    float r0_ = metal::clamp(radius_2 - spread, 0.0, metal::min(box_2.z, box_2.w) * 0.5);
                    metal::float4 _e356 = v_5.position;
                    metal::float2 _e358 = centred(_e356.xy, box_2, viewport);
                    metal::float2 q0_ = _e358 - ((box_2.zw * 0.5) - metal::float2(r0_));
                    float _e366 = color_1.w;
                    metal::float3 _e367 = corner_sdf(q0_, r0_, k_4);
                    float _e371 = viewport.z;
                    color_1.w = _e366 * metal::clamp(0.5 + (_e367.x * _e371), 0.0, 1.0);
                }
            } else {
                float dx = slope.x;
                float dy = slope.y;
                float aa_1 = metal::max((radius_2 <= 0.0) ? metal::max(dx, dy) : (dx + dy), 0.5);
                float outer = metal::clamp(0.5 - (distance / aa_1), 0.0, 1.0);
                float _e396 = v_5.shape.y;
                float inner = metal::clamp(0.5 - ((distance + _e396) / aa_1), 0.0, 1.0);
                float _e405 = color_1.w;
                float _e409 = v_5.border.w;
                float alpha = (_e405 * inner) + (_e409 * (outer - inner));
                metal::float4 _e413 = color_1;
                float _e416 = color_1.w;
                metal::float4 _e420 = v_5.border;
                float _e424 = v_5.border.w;
                metal::float3 rgb_2 = (alpha > 0.0) ? ((((_e413.xyz * _e416) * inner) + ((_e420.xyz * _e424) * (outer - inner))) / metal::float3(metal::max(alpha, 0.00001))) : metal::float3(0.0);
                color_1 = metal::float4(rgb_2, alpha);
            }
        }
    }
    float _e448 = v_5.options.y;
    if (_e448 > 0.5) {
        tex = given;
        if (!(has_texel)) {
            metal::float2 _e456 = v_5.uv;
            metal::float4 _e457 = atlas.sample(tex_sampler, _e456);
            tex = _e457;
        }
        float _e460 = v_5.options.y;
        if (_e460 > 1.5) {
            premultiplied_1 = true;
            metal::float4 _e464 = color_1;
            metal::float4 _e466 = tex;
            float _e470 = color_1.w;
            rgb = (_e464.xyz * _e466.xyz) * _e470;
            float _e474 = color_1.w;
            float _e476 = tex.w;
            color_1.w = _e474 * _e476;
        } else {
            metal::float4 _e478 = color_1;
            metal::float4 _e480 = tex;
            float _e484 = color_1.w;
            float _e486 = tex.w;
            float _e489 = v_5.options.y;
            metal::float4 _e490 = color_1;
            float _e492 = firmness(_e489, _e490.xyz);
            float _e493 = firm(_e486, _e492);
            color_1 = metal::float4(_e478.xyz * _e480.xyz, _e484 * _e493);
        }
    }
    float _e498 = v_5.options.x;
    if (_e498 > 0.0) {
        metal::float2 _e502 = v_5.point;
        metal::float4 _e504 = v_5.fade;
        metal::float4 _e508 = v_5.fade;
        metal::float4 _e511 = v_5.fade;
        metal::float2 _e515 = v_5.point;
        metal::float2 distance_1 = metal::min(_e502 - _e504.xy, (_e508.xy + _e511.zw) - _e515);
        float _e523 = v_5.options.x;
        float f_1 = metal::clamp(metal::min(distance_1.x, distance_1.y) / _e523, 0.0, 1.0);
        float _e529 = color_1.w;
        color_1.w = _e529 * f_1;
        float _e531 = scale;
        scale = _e531 * f_1;
    }
    float _e535 = v_5.mask.x;
    if (_e535 > 0.5) {
        float _e540 = v_5.mask.x;
        metal::float4 _e542 = v_5.position;
        float _e544 = region(_e540, _e542.xy, viewport, regions, _buffer_sizes);
        float _e546 = color_1.w;
        color_1.w = _e546 * _e544;
        float _e548 = scale;
        scale = _e548 * _e544;
    }
    float _e552 = v_5.mask.x;
    metal::float4 _e553 = color_1;
    float _e556 = color_1.w;
    float _e559 = color_1.w;
    metal::float3 _e561 = rgb;
    float _e562 = scale;
    float _e565 = color_1.w;
    bool _e567 = premultiplied_1;
    metal::float4 _e569 = tinted(_e552, _e567 ? metal::float4(_e561 * _e562, _e565) : metal::float4(_e553.xyz * _e556, _e559), regions, _buffer_sizes);
    return _e569;
}

metal::float4 shade(
    Out input_1,
    constant metal::float4& viewport,
    metal::texture2d<float, metal::access::sample> atlas,
    device type_6 const& regions,
    metal::sampler tex_sampler,
    constant _mslBufferSizes& _buffer_sizes
) {
    metal::float4 _e4 = shade_texel(input_1, metal::float4(0.0), false, viewport, atlas, regions, tex_sampler, _buffer_sizes);
    return _e4;
}

metal::float4 basic(
    Basic v_6,
    metal::texture2d<float, metal::access::sample> atlas,
    metal::sampler tex_sampler
) {
    if (v_6.texture > 0.5) {
        metal::float4 tex_1 = atlas.sample(tex_sampler, v_6.uv);
        float _e12 = firm(tex_1.w, v_6.firmness);
        float a_2 = v_6.color.w * _e12;
        return metal::float4((v_6.color.xyz * tex_1.xyz) * ((v_6.texture > 1.5) ? v_6.color.w : a_2), a_2);
    }
    return metal::float4(v_6.color.xyz * v_6.color.w, v_6.color.w);
}

struct vsInput {
    metal::float4 rect [[attribute(0)]];
    metal::float4 uv [[attribute(1)]];
    metal::float4 color [[attribute(2)]];
    metal::float4 fade [[attribute(3)]];
    metal::float4 options [[attribute(4)]];
    metal::float4 shape [[attribute(5)]];
    metal::float4 border [[attribute(6)]];
    metal::float4 mask [[attribute(7)]];
};
struct vsOutput {
    metal::float4 position [[position]];
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), flat]];
    metal::float2 point [[user(loc2), center_perspective]];
    metal::float4 fade [[user(loc3), flat]];
    metal::float4 options [[user(loc4), flat]];
    metal::float4 shape [[user(loc5), flat]];
    metal::float4 border [[user(loc6), flat]];
    metal::float4 mask [[user(loc7), flat]];
};
vertex vsOutput vs(
  vsInput varyings [[stage_in]]
, uint i [[vertex_id]]
, constant metal::float4& viewport [[buffer(0)]]
) {
    const Vertex v = { varyings.rect, varyings.uv, varyings.color, varyings.fade, varyings.options, varyings.shape, varyings.border, varyings.mask };
    metal::float2 position = {};
    float w = 1.0;
    Out o = {};
    metal::float2 corner_1 = metal::float2(static_cast<float>(i & 1u), static_cast<float>(i >> 1u));
    metal::float2 p_3 = v.rect.xy + (corner_1 * v.rect.zw);
    position = p_3;
    if (v.options.z > 0.5) {
        position = metal::float2(metal::dot(v.shape.xy, p_3) + v.border.x, metal::dot(v.shape.zw, p_3) + v.border.y);
    }
    if (v.options.z > 1.5) {
        w = metal::max(metal::dot(v.border.zw, p_3) + 1.0, 0.0001);
    }
    float _e49 = position.x;
    float _e52 = viewport.x;
    float _e56 = w;
    float _e58 = w;
    float _e60 = position.y;
    float _e63 = viewport.y;
    float _e68 = w;
    o.position = metal::float4(((_e49 / _e52) * 2.0) - _e56, _e58 - ((_e60 / _e63) * 2.0), 0.0, _e68);
    o.uv = v.uv.xy + (corner_1 * v.uv.zw);
    o.color = v.color;
    o.point = p_3;
    o.fade = v.fade;
    o.options = v.options;
    o.shape = v.shape;
    o.border = v.border;
    o.mask = v.mask;
    Out _e91 = o;
    const auto _tmp = _e91;
    return vsOutput { _tmp.position, _tmp.uv, _tmp.color, _tmp.point, _tmp.fade, _tmp.options, _tmp.shape, _tmp.border, _tmp.mask };
}


struct fsInput {
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), flat]];
    metal::float2 point [[user(loc2), center_perspective]];
    metal::float4 fade [[user(loc3), flat]];
    metal::float4 options [[user(loc4), flat]];
    metal::float4 shape [[user(loc5), flat]];
    metal::float4 border [[user(loc6), flat]];
    metal::float4 mask [[user(loc7), flat]];
};
struct fsOutput {
    metal::float4 member_1 [[color(0)]];
};
fragment fsOutput fs(
  fsInput varyings_1 [[stage_in]]
, metal::float4 position_2 [[position]]
, constant metal::float4& viewport [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> atlas [[texture(0)]]
, device type_6 const& regions [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    constexpr metal::sampler tex_sampler(
        metal::s_address::clamp_to_edge,
        metal::t_address::clamp_to_edge,
        metal::r_address::clamp_to_edge,
        metal::mag_filter::linear,
        metal::min_filter::linear,
        metal::coord::normalized
    );
    const Out v_1 = { position_2, varyings_1.uv, {}, varyings_1.color, varyings_1.point, {}, varyings_1.fade, varyings_1.options, varyings_1.shape, varyings_1.border, varyings_1.mask };
    metal::float4 _e1 = shade(v_1, viewport, atlas, regions, tex_sampler, _buffer_sizes);
    return fsOutput { _e1 };
}


struct fs_bothInput {
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), flat]];
    metal::float2 point [[user(loc2), center_perspective]];
    metal::float4 fade [[user(loc3), flat]];
    metal::float4 options [[user(loc4), flat]];
    metal::float4 shape [[user(loc5), flat]];
    metal::float4 border [[user(loc6), flat]];
    metal::float4 mask [[user(loc7), flat]];
};
struct fs_bothOutput {
    metal::float4 retained [[color(0)]];
    metal::float4 drawable [[color(1)]];
};
fragment fs_bothOutput fs_both(
  fs_bothInput varyings_2 [[stage_in]]
, metal::float4 position_3 [[position]]
, constant metal::float4& viewport [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> atlas [[texture(0)]]
, device type_6 const& regions [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    constexpr metal::sampler tex_sampler(
        metal::s_address::clamp_to_edge,
        metal::t_address::clamp_to_edge,
        metal::r_address::clamp_to_edge,
        metal::mag_filter::linear,
        metal::min_filter::linear,
        metal::coord::normalized
    );
    const Out v_2 = { position_3, varyings_2.uv, {}, varyings_2.color, varyings_2.point, {}, varyings_2.fade, varyings_2.options, varyings_2.shape, varyings_2.border, varyings_2.mask };
    metal::float4 _e1 = shade(v_2, viewport, atlas, regions, tex_sampler, _buffer_sizes);
    const auto _tmp = Both {_e1, _e1};
    return fs_bothOutput { _tmp.retained, _tmp.drawable };
}


struct vs_basicInput {
    metal::float4 rect [[attribute(0)]];
    metal::float4 uv [[attribute(1)]];
    metal::float4 color [[attribute(2)]];
    metal::float4 options [[attribute(4)]];
};
struct vs_basicOutput {
    metal::float4 position [[position]];
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), flat]];
    float texture [[user(loc2), flat]];
    float firmness [[user(loc3), flat]];
};
vertex vs_basicOutput vs_basic(
  vs_basicInput varyings_3 [[stage_in]]
, uint i_1 [[vertex_id]]
, constant metal::float4& viewport [[buffer(0)]]
) {
    const auto rect = varyings_3.rect;
    const auto uv = varyings_3.uv;
    const auto color = varyings_3.color;
    const auto options = varyings_3.options;
    Basic o_1 = {};
    metal::float2 corner_2 = metal::float2(static_cast<float>(i_1 & 1u), static_cast<float>(i_1 >> 1u));
    metal::float2 p_4 = rect.xy + (corner_2 * rect.zw);
    float _e21 = viewport.x;
    float _e30 = viewport.y;
    o_1.position = metal::float4(((p_4.x / _e21) * 2.0) - 1.0, 1.0 - ((p_4.y / _e30) * 2.0), 0.0, 1.0);
    o_1.uv = uv.xy + (corner_2 * uv.zw);
    o_1.color = color;
    o_1.texture = options.y;
    float _e50 = firmness(options.y, color.xyz);
    o_1.firmness = _e50;
    Basic _e51 = o_1;
    const auto _tmp = _e51;
    return vs_basicOutput { _tmp.position, _tmp.uv, _tmp.color, _tmp.texture, _tmp.firmness };
}


struct fs_basicInput {
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), flat]];
    float texture [[user(loc2), flat]];
    float firmness [[user(loc3), flat]];
};
struct fs_basicOutput {
    metal::float4 member_4 [[color(0)]];
};
fragment fs_basicOutput fs_basic(
  fs_basicInput varyings_4 [[stage_in]]
, metal::float4 position_4 [[position]]
, metal::texture2d<float, metal::access::sample> atlas [[texture(0)]]
) {
    constexpr metal::sampler tex_sampler(
        metal::s_address::clamp_to_edge,
        metal::t_address::clamp_to_edge,
        metal::r_address::clamp_to_edge,
        metal::mag_filter::linear,
        metal::min_filter::linear,
        metal::coord::normalized
    );
    const Basic v_3 = { position_4, varyings_4.uv, {}, varyings_4.color, varyings_4.texture, varyings_4.firmness };
    metal::float4 _e1 = basic(v_3, atlas, tex_sampler);
    return fs_basicOutput { _e1 };
}


struct fs_basic_bothInput {
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), flat]];
    float texture [[user(loc2), flat]];
    float firmness [[user(loc3), flat]];
};
struct fs_basic_bothOutput {
    metal::float4 retained [[color(0)]];
    metal::float4 drawable [[color(1)]];
};
fragment fs_basic_bothOutput fs_basic_both(
  fs_basic_bothInput varyings_5 [[stage_in]]
, metal::float4 position_5 [[position]]
, metal::texture2d<float, metal::access::sample> atlas [[texture(0)]]
) {
    constexpr metal::sampler tex_sampler(
        metal::s_address::clamp_to_edge,
        metal::t_address::clamp_to_edge,
        metal::r_address::clamp_to_edge,
        metal::mag_filter::linear,
        metal::min_filter::linear,
        metal::coord::normalized
    );
    const Basic v_4 = { position_5, varyings_5.uv, {}, varyings_5.color, varyings_5.texture, varyings_5.firmness };
    metal::float4 _e1 = basic(v_4, atlas, tex_sampler);
    const auto _tmp = Both {_e1, _e1};
    return fs_basic_bothOutput { _tmp.retained, _tmp.drawable };
}
