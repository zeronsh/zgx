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
    metal::float2 q = (p - b.xy) - half_;
    float radius_1 = (q.y < 0.0) ? ((q.x > 0.0) ? r.y : r.x) : ((q.x > 0.0) ? r.z : r.w);
    metal::float2 d = (metal::abs(q) - half_) + metal::float2(radius_1);
    float sdf = (metal::length(metal::max(d, metal::float2(0.0))) + metal::min(metal::max(d.x, d.y), 0.0)) - radius_1;
    if (feather > 0.0) {
        float t_1 = metal::clamp(-(sdf) / feather, 0.0, 1.0);
        return (t_1 * t_1) * (3.0 - (2.0 * t_1));
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
        float t_2 = metal::clamp((p_1.y - b_1.y) / f.x, 0.0, 1.0);
        float _e16 = k_1;
        k_1 = _e16 * ((t_2 * t_2) * (3.0 - (2.0 * t_2)));
    }
    if (f.y > 0.0) {
        float t_3 = metal::clamp((b_1.z - p_1.x) / f.y, 0.0, 1.0);
        float _e35 = k_1;
        k_1 = _e35 * ((t_3 * t_3) * (3.0 - (2.0 * t_3)));
    }
    if (f.z > 0.0) {
        float t_4 = metal::clamp((b_1.w - p_1.y) / f.z, 0.0, 1.0);
        float _e54 = k_1;
        k_1 = _e54 * ((t_4 * t_4) * (3.0 - (2.0 * t_4)));
    }
    if (f.w > 0.0) {
        float t_5 = metal::clamp((p_1.x - b_1.x) / f.w, 0.0, 1.0);
        float _e73 = k_1;
        k_1 = _e73 * ((t_5 * t_5) * (3.0 - (2.0 * t_5)));
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

metal::float3 supercircle(
    metal::float2 a_1,
    metal::float2 t
) {
    float high = metal::max(a_1.x, a_1.y);
    float low = metal::min(a_1.x, a_1.y);
    float s = metal::clamp(low / metal::max(high, 0.00000000000000000001), 0.0, 1.0);
    float p_3 = ((((((3.15601 - (0.926054 * s)) * s) - 3.64122) * s) + 1.26803) * s) + 0.268531;
    float dp = ((((9.46803 - (3.704216 * s)) * s) - 7.28244) * s) + 1.26803;
    float bulge = 1.0 / (1.0 - ((s * s) * p_3));
    float along = ((-(bulge) * bulge) * s) * ((2.0 * p_3) + (s * dp));
    metal::float2 ds = (a_1.y > a_1.x) ? metal::float2(1.0 / metal::max(high, 0.00000000000000000001), -(low) / metal::max(high * high, 0.00000000000000000001)) : metal::float2(-(low) / metal::max(high * high, 0.00000000000000000001), 1.0 / metal::max(high, 0.00000000000000000001));
    float reach = metal::length(a_1);
    metal::float2 round = (a_1 / metal::float2(metal::max(reach, 0.00000000000000000001))) + (along * ds);
    metal::float2 v_7 = metal::max((a_1 * 1.528665) - metal::float2(0.528665), metal::float2(0.0));
    float vs_1 = metal::length(v_7);
    metal::float2 circle = v_7 / metal::float2(metal::max(vs_1, 0.00000000000000000001));
    float k_3 = (a_1.y > a_1.x) ? t.x : t.y;
    return metal::float3(metal::mix((reach + 1.0) - bulge, (vs_1 * 0.65416557) + 0.34583443, k_3) - 1.0, metal::mix(round, circle, k_3));
}

metal::float3 corner_edge(
    metal::float2 at_1,
    float radius,
    metal::float2 size
) {
    bool local_2 = {};
    float reach_1 = radius * 1.528665;
    if (!((at_1.x >= reach_1))) {
        local_2 = at_1.y >= reach_1;
    } else {
        local_2 = true;
    }
    bool _e13 = local_2;
    if (_e13) {
        return (at_1.x < at_1.y) ? metal::float3(-(at_1.x), 1.0, 0.0) : metal::float3(-(at_1.y), 0.0, 1.0);
    }
    metal::float2 t_6 = metal::clamp(metal::float2(2.891557) - ((1.4457785 * size) / metal::float2(reach_1)), metal::float2(0.0), metal::float2(1.0));
    metal::float3 _e47 = supercircle(metal::max((metal::float2(reach_1) - at_1) / metal::float2(reach_1), metal::float2(0.0)), t_6);
    float slope = metal::length(_e47.yz);
    if (slope < 0.00001) {
        return metal::float3(_e47.x * reach_1, 1.0, 0.0);
    }
    return metal::float3((_e47.x * reach_1) / metal::min(slope, 1.5), metal::abs(_e47.yz) / metal::float2(slope));
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
    bool local_3 = {};
    bool local_4 = {};
    float corner = {};
    bool local_5 = {};
    metal::float3 edge = metal::float3(0.0);
    bool local_6 = {};
    metal::float3 inner = metal::float3(0.0);
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
            local_4 = _e38 > 0.0;
        } else {
            local_4 = true;
        }
        bool _e42 = local_4;
        local_3 = _e42;
    } else {
        local_3 = false;
    }
    bool _e44 = local_3;
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
        metal::float4 _e108 = v_5.position;
        metal::float4 _e111 = v_5.fade;
        metal::float2 _e112 = centred(_e108.xy, _e111, viewport);
        metal::float4 _e114 = v_5.fade;
        metal::float2 q_1 = _e112 - ((_e114.zw * 0.5) - metal::float2(radius_2));
        float _e123 = v_5.shape.w;
        if (_e123 > 1.5) {
            local_5 = radius_2 > 0.0;
        } else {
            local_5 = false;
        }
        bool continuous = local_5;
        if (continuous) {
            metal::float4 _e136 = v_5.fade;
            metal::float4 _e141 = v_5.position;
            metal::float4 _e144 = v_5.fade;
            metal::float2 _e145 = centred(_e141.xy, _e144, viewport);
            metal::float4 _e148 = v_5.fade;
            metal::float3 _e150 = corner_edge((_e136.zw * 0.5) - _e145, radius_2, _e148.zw);
            edge = _e150;
        }
        float _e163 = edge.x;
        float distance = continuous ? _e163 : ((metal::length(metal::max(q_1, metal::float2(0.0))) + metal::min(metal::max(q_1.x, q_1.y), 0.0)) - radius_2);
        metal::float2 outside = metal::max(q_1, metal::float2(0.0));
        float reach_2 = metal::length(outside);
        metal::float3 _e186 = edge;
        metal::float2 gradient = continuous ? _e186.yz : ((reach_2 > 0.0) ? (outside / metal::float2(metal::max(reach_2, 0.00000000000000000001))) : ((q_1.x > q_1.y) ? metal::float2(1.0, 0.0) : metal::float2(0.0, 1.0)));
        float _e191 = viewport.z;
        metal::float2 slope_1 = gradient / metal::float2(_e191);
        float _e196 = v_5.shape.y;
        if (_e196 < 0.0) {
            float _e201 = v_5.shape.x;
            uint a_2 = naga_f2u32(_e201);
            float _e205 = v_5.shape.z;
            uint b_2 = naga_f2u32(_e205);
            metal::float4 sides = metal::float4(static_cast<float>(a_2 & 4095u), static_cast<float>(a_2 >> 12u), static_cast<float>(b_2 & 4095u), static_cast<float>(b_2 >> 12u)) / metal::float4(16.0);
            float _e225 = v_5.point.x;
            float _e228 = v_5.fade.x;
            float _e231 = v_5.fade.z;
            bool right_1 = _e225 > (_e228 + (_e231 * 0.5));
            float _e238 = v_5.point.y;
            float _e241 = v_5.fade.y;
            float _e244 = v_5.fade.w;
            bool bottom_1 = _e238 > (_e241 + (_e244 * 0.5));
            float across = right_1 ? sides.y : sides.w;
            float down = bottom_1 ? sides.z : sides.x;
            metal::float4 _e256 = v_5.fade;
            metal::float4 _e261 = v_5.fade;
            metal::float4 box_1 = metal::float4(_e256.xy + sides.wx, metal::max((_e261.zw - sides.wx) - sides.yz, metal::float2(0.0)));
            float inner_radius = metal::clamp(radius_2 - ((across + down) * 0.5), 0.0, metal::min(box_1.z, box_1.w) * 0.5);
            metal::float4 _e283 = v_5.position;
            metal::float2 _e285 = centred(_e283.xy, box_1, viewport);
            metal::float2 iq = _e285 - ((box_1.zw * 0.5) - metal::float2(inner_radius));
            if (continuous) {
                local_6 = inner_radius > 0.0;
            } else {
                local_6 = false;
            }
            bool curved = local_6;
            if (curved) {
                metal::float4 _e305 = v_5.position;
                metal::float2 _e307 = centred(_e305.xy, box_1, viewport);
                metal::float3 _e310 = corner_edge((box_1.zw * 0.5) - _e307, inner_radius, box_1.zw);
                inner = _e310;
            }
            float _e323 = inner.x;
            float inside = curved ? _e323 : ((metal::length(metal::max(iq, metal::float2(0.0))) + metal::min(metal::max(iq.x, iq.y), 0.0)) - inner_radius);
            float aa = continuous ? (slope_1.x + slope_1.y) : metal::max(slope_1.x + slope_1.y, 0.5);
            float _e335 = inner.y;
            float _e337 = inner.z;
            float _e341 = viewport.z;
            float inner_aa = curved ? ((_e335 + _e337) / _e341) : aa;
            if (box_1.z > 0.0) {
                local_7 = box_1.w > 0.0;
            } else {
                local_7 = false;
            }
            bool _e359 = local_7;
            float hollow = _e359 ? metal::clamp(0.5 - (inside / inner_aa), 0.0, 1.0) : 0.0;
            float _e363 = color_1.w;
            color_1.w = _e363 * metal::max(metal::clamp(0.5 - (distance / aa), 0.0, 1.0) - hollow, 0.0);
        } else {
            float _e376 = v_5.shape.z;
            if (_e376 > 0.0) {
                float _e381 = v_5.shape.z;
                float x = distance / (_e381 * 1.4142135);
                float x2_ = x * x;
                float e = metal::sign(x) * metal::sqrt(1.0 - metal::exp((-(x2_) * (1.2732395 + (0.147 * x2_))) / (1.0 + (0.147 * x2_))));
                float _e404 = color_1.w;
                color_1.w = _e404 * (0.5 - (0.5 * e));
            } else {
                float dx = slope_1.x;
                float dy = slope_1.y;
                float aa_1 = continuous ? (dx + dy) : metal::max((radius_2 <= 0.0) ? metal::max(dx, dy) : (dx + dy), 0.5);
                float outer = metal::clamp(0.5 - (distance / aa_1), 0.0, 1.0);
                float _e429 = v_5.shape.y;
                float inner_1 = metal::clamp(0.5 - ((distance + _e429) / aa_1), 0.0, 1.0);
                float _e438 = color_1.w;
                float _e442 = v_5.border.w;
                float alpha = (_e438 * inner_1) + (_e442 * (outer - inner_1));
                metal::float4 _e446 = color_1;
                float _e449 = color_1.w;
                metal::float4 _e453 = v_5.border;
                float _e457 = v_5.border.w;
                metal::float3 rgb_2 = (alpha > 0.0) ? ((((_e446.xyz * _e449) * inner_1) + ((_e453.xyz * _e457) * (outer - inner_1))) / metal::float3(metal::max(alpha, 0.00001))) : metal::float3(0.0);
                color_1 = metal::float4(rgb_2, alpha);
            }
        }
    }
    float _e481 = v_5.options.y;
    if (_e481 > 0.5) {
        tex = given;
        if (!(has_texel)) {
            metal::float2 _e489 = v_5.uv;
            metal::float4 _e490 = atlas.sample(tex_sampler, _e489);
            tex = _e490;
        }
        float _e493 = v_5.options.y;
        if (_e493 > 1.5) {
            premultiplied_1 = true;
            metal::float4 _e497 = color_1;
            metal::float4 _e499 = tex;
            float _e503 = color_1.w;
            rgb = (_e497.xyz * _e499.xyz) * _e503;
            float _e507 = color_1.w;
            float _e509 = tex.w;
            color_1.w = _e507 * _e509;
        } else {
            metal::float4 _e511 = color_1;
            metal::float4 _e513 = tex;
            float _e517 = color_1.w;
            float _e519 = tex.w;
            float _e522 = v_5.options.y;
            metal::float4 _e523 = color_1;
            float _e525 = firmness(_e522, _e523.xyz);
            float _e526 = firm(_e519, _e525);
            color_1 = metal::float4(_e511.xyz * _e513.xyz, _e517 * _e526);
        }
    }
    float _e531 = v_5.options.x;
    if (_e531 > 0.0) {
        metal::float2 _e535 = v_5.point;
        metal::float4 _e537 = v_5.fade;
        metal::float4 _e541 = v_5.fade;
        metal::float4 _e544 = v_5.fade;
        metal::float2 _e548 = v_5.point;
        metal::float2 distance_1 = metal::min(_e535 - _e537.xy, (_e541.xy + _e544.zw) - _e548);
        float _e556 = v_5.options.x;
        float f_1 = metal::clamp(metal::min(distance_1.x, distance_1.y) / _e556, 0.0, 1.0);
        float _e562 = color_1.w;
        color_1.w = _e562 * f_1;
        float _e564 = scale;
        scale = _e564 * f_1;
    }
    float _e568 = v_5.mask.x;
    if (_e568 > 0.5) {
        float _e573 = v_5.mask.x;
        metal::float4 _e575 = v_5.position;
        float _e577 = region(_e573, _e575.xy, viewport, regions, _buffer_sizes);
        float _e579 = color_1.w;
        color_1.w = _e579 * _e577;
        float _e581 = scale;
        scale = _e581 * _e577;
    }
    float _e585 = v_5.mask.x;
    metal::float4 _e586 = color_1;
    float _e589 = color_1.w;
    float _e592 = color_1.w;
    metal::float3 _e594 = rgb;
    float _e595 = scale;
    float _e598 = color_1.w;
    bool _e600 = premultiplied_1;
    metal::float4 _e602 = tinted(_e585, _e600 ? metal::float4(_e594 * _e595, _e598) : metal::float4(_e586.xyz * _e589, _e592), regions, _buffer_sizes);
    return _e602;
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
        float a_3 = v_6.color.w * _e12;
        return metal::float4((v_6.color.xyz * tex_1.xyz) * ((v_6.texture > 1.5) ? v_6.color.w : a_3), a_3);
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
    metal::float2 p_4 = v.rect.xy + (corner_1 * v.rect.zw);
    position = p_4;
    if (v.options.z > 0.5) {
        position = metal::float2(metal::dot(v.shape.xy, p_4) + v.border.x, metal::dot(v.shape.zw, p_4) + v.border.y);
    }
    if (v.options.z > 1.5) {
        w = metal::max(metal::dot(v.border.zw, p_4) + 1.0, 0.0001);
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
    o.point = p_4;
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
    metal::float2 p_5 = rect.xy + (corner_2 * rect.zw);
    float _e21 = viewport.x;
    float _e30 = viewport.y;
    o_1.position = metal::float4(((p_5.x / _e21) * 2.0) - 1.0, 1.0 - ((p_5.y / _e30) * 2.0), 0.0, 1.0);
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
