struct Vertex {
    float4 rect : LOC0;
    float4 uv : LOC1;
    float4 color : LOC2;
    float4 fade : LOC3;
    float4 options : LOC4;
    float4 shape : LOC5;
    float4 border : LOC6;
    float4 mask : LOC7;
};

struct Out {
    float4 position : SV_Position;
    float2 uv : LOC0;
    nointerpolation float4 color : LOC1;
    float2 point_ : LOC2;
    nointerpolation float4 fade : LOC3;
    nointerpolation float4 options : LOC4;
    nointerpolation float4 shape : LOC5;
    nointerpolation float4 border : LOC6;
    nointerpolation float4 mask : LOC7;
};

struct Region {
    float4 round0_;
    float4 radii0_;
    float4 round1_;
    float4 radii1_;
    float4 fade_box;
    float4 fade;
    float4 color0_;
    float4 color1_;
    float4 color2_;
    float4 tint;
};

struct Both {
    float4 retained : SV_Target0;
    float4 drawable : SV_Target1;
};

struct Basic {
    float4 position : SV_Position;
    float2 uv : LOC0;
    nointerpolation float4 color : LOC1;
    nointerpolation float texture_ : LOC2;
    nointerpolation float firmness : LOC3;
};

cbuffer viewport : register(b0) { float4 viewport; }
Texture2D<float4> atlas : register(t0);
ByteAddressBuffer regions : register(t1);
SamplerState tex_sampler : register(s0);
Texture2D<float4> zgx_behind : register(t2);

struct VertexOutput_vs {
    float2 uv_1 : LOC0;
    nointerpolation float4 color_2 : LOC1;
    float2 point_ : LOC2;
    nointerpolation float4 fade : LOC3;
    nointerpolation float4 options_1 : LOC4;
    nointerpolation float4 shape : LOC5;
    nointerpolation float4 border : LOC6;
    nointerpolation float4 mask : LOC7;
    float4 position_3 : SV_Position;
};

struct FragmentInput_fs {
    float2 uv_2 : LOC0;
    nointerpolation float4 color_3 : LOC1;
    float2 point_1 : LOC2;
    nointerpolation float4 fade_1 : LOC3;
    nointerpolation float4 options_2 : LOC4;
    nointerpolation float4 shape_1 : LOC5;
    nointerpolation float4 border_1 : LOC6;
    nointerpolation float4 mask_1 : LOC7;
    float4 position_4 : SV_Position;
};

struct FragmentInput_fs_both {
    float2 uv_3 : LOC0;
    nointerpolation float4 color_4 : LOC1;
    float2 point_2 : LOC2;
    nointerpolation float4 fade_2 : LOC3;
    nointerpolation float4 options_3 : LOC4;
    nointerpolation float4 shape_2 : LOC5;
    nointerpolation float4 border_2 : LOC6;
    nointerpolation float4 mask_2 : LOC7;
    float4 position_5 : SV_Position;
};

struct VertexOutput_vs_basic {
    float2 uv_4 : LOC0;
    nointerpolation float4 color_5 : LOC1;
    nointerpolation float texture_ : LOC2;
    nointerpolation float firmness_1 : LOC3;
    float4 position_6 : SV_Position;
};

struct FragmentInput_fs_basic {
    float2 uv_5 : LOC0;
    nointerpolation float4 color_6 : LOC1;
    nointerpolation float texture_1 : LOC2;
    nointerpolation float firmness_2 : LOC3;
    float4 position_7 : SV_Position;
};

struct FragmentInput_fs_basic_both {
    float2 uv_6 : LOC0;
    nointerpolation float4 color_7 : LOC1;
    nointerpolation float texture_2 : LOC2;
    nointerpolation float firmness_3 : LOC3;
    float4 position_8 : SV_Position;
};

struct FragmentInput_zgx_blended {
    float2 uv_7 : LOC0;
    nointerpolation float4 color_8 : LOC1;
    float2 point_3 : LOC2;
    nointerpolation float4 fade_3 : LOC3;
    nointerpolation float4 options_4 : LOC4;
    nointerpolation float4 shape_3 : LOC5;
    nointerpolation float4 border_3 : LOC6;
    nointerpolation float4 mask_3 : LOC7;
    float4 position_9 : SV_Position;
};

float firmness(float options_y, float3 text)
{
    bool local = (bool)0;

    if (!((options_y < 1.1))) {
        local = (options_y > 1.5);
    } else {
        local = true;
    }
    bool _e10 = local;
    if (_e10) {
        return 0.0;
    }
    return clamp((dot(text, float3(0.2126, 0.7152, 0.0722)) * 2.0), 0.0, 1.0);
}

float firm(float a, float k)
{
    return (a + (((k * 0.67) * a) * (1.0 - a)));
}

float rounded(float4 b, float4 r, float feather, float2 p)
{
    if ((b.z < 0.0)) {
        return 1.0;
    }
    float2 half_ = (b.zw * 0.5);
    float2 q_1 = ((p - b.xy) - half_);
    float radius_1 = ((q_1.y < 0.0) ? ((q_1.x > 0.0) ? r.y : r.x) : ((q_1.x > 0.0) ? r.z : r.w));
    float2 d_1 = ((abs(q_1) - half_) + (radius_1).xx);
    float sdf = ((length(max(d_1, (0.0).xx)) + min(max(d_1.x, d_1.y), 0.0)) - radius_1);
    if ((feather > 0.0)) {
        float t = clamp((-(sdf) / feather), 0.0, 1.0);
        return ((t * t) * (3.0 - (2.0 * t)));
    }
    float _e60 = viewport.z;
    return clamp((0.5 - (sdf * _e60)), 0.0, 1.0);
}

uint naga_f2u32(float value) {
    return uint(clamp(value, 0.0, 4294967000.0));
}

Region ConstructRegion(float4 arg0, float4 arg1, float4 arg2, float4 arg3, float4 arg4, float4 arg5, float4 arg6, float4 arg7, float4 arg8, float4 arg9) {
    Region ret = (Region)0;
    ret.round0_ = arg0;
    ret.radii0_ = arg1;
    ret.round1_ = arg2;
    ret.radii1_ = arg3;
    ret.fade_box = arg4;
    ret.fade = arg5;
    ret.color0_ = arg6;
    ret.color1_ = arg7;
    ret.color2_ = arg8;
    ret.tint = arg9;
    return ret;
}

float4 tinted(float index, float4 premultiplied)
{
    bool local_1 = (bool)0;

    if ((index < 0.5)) {
        return premultiplied;
    }
    Region g = ConstructRegion(asfloat(regions.Load4((naga_f2u32(index) - 1u)*160+0)), asfloat(regions.Load4((naga_f2u32(index) - 1u)*160+16)), asfloat(regions.Load4((naga_f2u32(index) - 1u)*160+32)), asfloat(regions.Load4((naga_f2u32(index) - 1u)*160+48)), asfloat(regions.Load4((naga_f2u32(index) - 1u)*160+64)), asfloat(regions.Load4((naga_f2u32(index) - 1u)*160+80)), asfloat(regions.Load4((naga_f2u32(index) - 1u)*160+96)), asfloat(regions.Load4((naga_f2u32(index) - 1u)*160+112)), asfloat(regions.Load4((naga_f2u32(index) - 1u)*160+128)), asfloat(regions.Load4((naga_f2u32(index) - 1u)*160+144)));
    if (!((g.tint.x < 0.5))) {
        local_1 = (premultiplied.w <= 0.0);
    } else {
        local_1 = true;
    }
    bool _e21 = local_1;
    if (_e21) {
        return premultiplied;
    }
    float4 c_5 = float4((premultiplied.xyz / (premultiplied.w).xxx), 1.0);
    float3 rgb_1 = clamp(float3(dot(g.color0_, c_5), dot(g.color1_, c_5), dot(g.color2_, c_5)), (0.0).xxx, (1.0).xxx);
    return float4((rgb_1 * premultiplied.w), premultiplied.w);
}

float faded(float4 b_1, float4 f, float2 p_1)
{
    float k_1 = 1.0;

    if ((f.x > 0.0)) {
        float t_1 = clamp(((p_1.y - b_1.y) / f.x), 0.0, 1.0);
        float _e16 = k_1;
        k_1 = (_e16 * ((t_1 * t_1) * (3.0 - (2.0 * t_1))));
    }
    if ((f.y > 0.0)) {
        float t_2 = clamp(((b_1.z - p_1.x) / f.y), 0.0, 1.0);
        float _e35 = k_1;
        k_1 = (_e35 * ((t_2 * t_2) * (3.0 - (2.0 * t_2))));
    }
    if ((f.z > 0.0)) {
        float t_3 = clamp(((b_1.w - p_1.y) / f.z), 0.0, 1.0);
        float _e54 = k_1;
        k_1 = (_e54 * ((t_3 * t_3) * (3.0 - (2.0 * t_3))));
    }
    if ((f.w > 0.0)) {
        float t_4 = clamp(((p_1.x - b_1.x) / f.w), 0.0, 1.0);
        float _e73 = k_1;
        k_1 = (_e73 * ((t_4 * t_4) * (3.0 - (2.0 * t_4))));
    }
    float _e81 = k_1;
    return _e81;
}

float region(float index_1, float2 position_1)
{
    float k_2 = (float)0;

    Region g_1 = ConstructRegion(asfloat(regions.Load4((naga_f2u32(index_1) - 1u)*160+0)), asfloat(regions.Load4((naga_f2u32(index_1) - 1u)*160+16)), asfloat(regions.Load4((naga_f2u32(index_1) - 1u)*160+32)), asfloat(regions.Load4((naga_f2u32(index_1) - 1u)*160+48)), asfloat(regions.Load4((naga_f2u32(index_1) - 1u)*160+64)), asfloat(regions.Load4((naga_f2u32(index_1) - 1u)*160+80)), asfloat(regions.Load4((naga_f2u32(index_1) - 1u)*160+96)), asfloat(regions.Load4((naga_f2u32(index_1) - 1u)*160+112)), asfloat(regions.Load4((naga_f2u32(index_1) - 1u)*160+128)), asfloat(regions.Load4((naga_f2u32(index_1) - 1u)*160+144)));
    float _e10 = viewport.z;
    float2 p_2 = (position_1 / (_e10).xx);
    const float _e17 = rounded(g_1.round0_, g_1.radii0_, g_1.tint.y, p_2);
    const float _e22 = rounded(g_1.round1_, g_1.radii1_, g_1.tint.z, p_2);
    k_2 = (_e17 * _e22);
    float _e25 = k_2;
    const float _e28 = faded(g_1.fade_box, g_1.fade, p_2);
    k_2 = (_e25 * _e28);
    if ((g_1.tint.w > 0.5)) {
        Region o_3 = ConstructRegion(asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+0)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+16)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+32)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+48)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+64)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+80)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+96)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+112)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+128)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+144)));
        float _e42 = k_2;
        const float _e45 = faded(o_3.fade_box, o_3.fade, p_2);
        k_2 = (_e42 * _e45);
    }
    float _e47 = k_2;
    return _e47;
}

float3 corner_sdf(float2 q, float radius, float k_3)
{
    bool local_2 = (bool)0;
    bool local_3 = (bool)0;
    bool local_4 = (bool)0;

    float2 o_4 = max(q, (0.0).xx);
    if ((k_3 > 1.01)) {
        local_2 = (o_4.x > 0.0);
    } else {
        local_2 = false;
    }
    bool _e14 = local_2;
    if (_e14) {
        local_3 = (o_4.y > 0.0);
    } else {
        local_3 = false;
    }
    bool _e21 = local_3;
    if (_e21) {
        local_4 = (radius > 0.0);
    } else {
        local_4 = false;
    }
    bool _e27 = local_4;
    if (_e27) {
        float n = exp2(k_3);
        float2 u = (o_4 / (radius).xx);
        float s_2 = pow((pow(u.x, n) + pow(u.y, n)), (1.0 / n));
        float2 g_2 = pow((u / (max(s_2, 1e-6)).xx), ((n - 1.0)).xx);
        float gl = max(length(g_2), 1e-6);
        return float3((((s_2 - 1.0) * radius) / gl), (g_2 / (gl).xx));
    }
    float reach_1 = length(o_4);
    float2 gradient = ((reach_1 > 0.0) ? (o_4 / (max(reach_1, 1e-20)).xx) : ((q.x > q.y) ? float2(1.0, 0.0) : float2(0.0, 1.0)));
    return float3(((reach_1 + min(max(q.x, q.y), 0.0)) - radius), gradient);
}

float2 centred(float2 at, float4 box)
{
    float _e9 = viewport.z;
    float2 middle = (round((((box.xy + (box.zw * 0.5)) * _e9) * 64.0)) / (64.0).xx);
    float _e21 = viewport.z;
    return (abs((at - middle)) / (_e21).xx);
}

float4 shade_texel(Out input_1, float4 given, bool has_texel)
{
    Out v_5 = (Out)0;
    float4 color_1 = (float4)0;
    bool local_5 = (bool)0;
    bool local_6 = (bool)0;
    float corner = (float)0;
    bool local_7 = (bool)0;
    float reach = (float)0;
    bool local_8 = (bool)0;
    bool premultiplied_1 = false;
    float3 rgb = (0.0).xxx;
    float scale = 1.0;
    float4 tex = (float4)0;

    v_5 = input_1;
    float _e6 = v_5.options.z;
    if ((_e6 < 0.5)) {
        float4 _e11 = v_5.position;
        float _e15 = viewport.z;
        v_5.point_ = (_e11.xy / (_e15).xx);
    }
    float4 _e19 = v_5.color;
    color_1 = _e19;
    float _e23 = v_5.options.z;
    if ((_e23 < 0.5)) {
        float _e30 = v_5.shape.w;
        if (!((_e30 > 0.5))) {
            float _e38 = v_5.shape.z;
            local_6 = (_e38 > 0.0);
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
        if ((_e51 > 0.5)) {
            float _e56 = v_5.point_.x;
            float _e59 = v_5.fade.x;
            float _e62 = v_5.fade.z;
            bool right = (_e56 > (_e59 + (_e62 * 0.5)));
            float _e69 = v_5.point_.y;
            float _e72 = v_5.fade.y;
            float _e75 = v_5.fade.w;
            bool bottom = (_e69 > (_e72 + (_e75 * 0.5)));
            float _e82 = v_5.border.x;
            float _e85 = v_5.border.y;
            float _e89 = v_5.border.w;
            float _e92 = v_5.border.z;
            corner = (bottom ? (right ? _e92 : _e89) : (right ? _e85 : _e82));
        }
        float _e95 = corner;
        float _e98 = v_5.fade.z;
        float _e101 = v_5.fade.w;
        float radius_2 = clamp(_e95, 0.0, (min(_e98, _e101) * 0.5));
        float _e109 = v_5.shape.w;
        float _e112 = v_5.fade.z;
        float _e115 = v_5.fade.w;
        float k_4 = ((radius_2 < ((min(_e112, _e115) * 0.5) - 0.25)) ? _e109 : 1.0);
        float4 _e125 = v_5.position;
        float4 _e128 = v_5.fade;
        const float2 _e129 = centred(_e125.xy, _e128);
        float4 _e131 = v_5.fade;
        float2 q_2 = (_e129 - ((_e131.zw * 0.5) - (radius_2).xx));
        const float3 _e138 = corner_sdf(q_2, radius_2, k_4);
        float distance_ = _e138.x;
        float _e144 = viewport.z;
        float2 slope = (abs(_e138.yz) / (_e144).xx);
        float _e149 = v_5.shape.y;
        if ((_e149 < 0.0)) {
            float _e154 = v_5.shape.x;
            uint a_1 = naga_f2u32(_e154);
            float _e158 = v_5.shape.z;
            uint b_2 = naga_f2u32(_e158);
            float4 sides = (float4(float((a_1 & 4095u)), float((a_1 >> 12u)), float((b_2 & 4095u)), float((b_2 >> 12u))) / (16.0).xxxx);
            float _e178 = v_5.point_.x;
            float _e181 = v_5.fade.x;
            float _e184 = v_5.fade.z;
            bool right_1 = (_e178 > (_e181 + (_e184 * 0.5)));
            float _e191 = v_5.point_.y;
            float _e194 = v_5.fade.y;
            float _e197 = v_5.fade.w;
            bool bottom_1 = (_e191 > (_e194 + (_e197 * 0.5)));
            float across = (right_1 ? sides.y : sides.w);
            float down = (bottom_1 ? sides.z : sides.x);
            float4 _e209 = v_5.fade;
            float4 _e214 = v_5.fade;
            float4 box_1 = float4((_e209.xy + sides.wx), max(((_e214.zw - sides.wx) - sides.yz), (0.0).xx));
            float inner_radius = clamp((radius_2 - ((across + down) * 0.5)), 0.0, (min(box_1.z, box_1.w) * 0.5));
            float4 _e236 = v_5.position;
            const float2 _e238 = centred(_e236.xy, box_1);
            float2 iq = (_e238 - ((box_1.zw * 0.5) - (inner_radius).xx));
            const float3 _e245 = corner_sdf(iq, inner_radius, k_4);
            float inside = _e245.x;
            float aa = max((slope.x + slope.y), 0.5);
            if ((box_1.z > 0.0)) {
                local_7 = (box_1.w > 0.0);
            } else {
                local_7 = false;
            }
            bool _e267 = local_7;
            float hollow = (_e267 ? clamp((0.5 - (inside / aa)), 0.0, 1.0) : 0.0);
            float _e271 = color_1.w;
            color_1.w = (_e271 * max((clamp((0.5 - (distance_ / aa)), 0.0, 1.0) - hollow), 0.0));
        } else {
            float _e284 = v_5.shape.z;
            if ((_e284 > 0.0)) {
                reach = distance_;
                float _e290 = v_5.shape.y;
                if ((_e290 > 500.0)) {
                    local_8 = (k_4 > 1.01);
                } else {
                    local_8 = false;
                }
                bool _e298 = local_8;
                if (_e298) {
                    float _e301 = v_5.shape.y;
                    float grown = (_e301 - 1000.0);
                    float4 _e305 = v_5.fade;
                    float4 _e310 = v_5.fade;
                    float4 held = float4((_e305.xy + (grown).xx), max((_e310.zw - ((2.0 * grown)).xx), (0.0).xx));
                    float held_radius = clamp((radius_2 - grown), 0.0, (min(held.z, held.w) * 0.5));
                    float4 _e329 = v_5.position;
                    const float2 _e331 = centred(_e329.xy, held);
                    float2 hq = (_e331 - ((held.zw * 0.5) - (held_radius).xx));
                    const float3 _e338 = corner_sdf(hq, held_radius, k_4);
                    reach = (_e338.x - grown);
                }
                float _e341 = reach;
                float _e344 = v_5.shape.z;
                float x = (_e341 / (_e344 * 1.4142135));
                float x2_ = (x * x);
                float e = (sign(x) * sqrt((1.0 - exp(((-(x2_) * (1.2732395 + (0.147 * x2_))) / (1.0 + (0.147 * x2_)))))));
                float _e367 = color_1.w;
                color_1.w = (_e367 * (0.5 - (0.5 * e)));
                float _e375 = v_5.shape.y;
                if ((_e375 > 500.0)) {
                    float _e380 = v_5.shape.y;
                    float spread = (_e380 - 1000.0);
                    float4 _e384 = v_5.fade;
                    float2 _e387 = v_5.uv;
                    float4 _e392 = v_5.fade;
                    float4 box_2 = float4(((_e384.xy - _e387) + (spread).xx), max((_e392.zw - ((2.0 * spread)).xx), (0.0).xx));
                    float r0_ = clamp((radius_2 - spread), 0.0, (min(box_2.z, box_2.w) * 0.5));
                    float4 _e411 = v_5.position;
                    const float2 _e413 = centred(_e411.xy, box_2);
                    float2 q0_ = (_e413 - ((box_2.zw * 0.5) - (r0_).xx));
                    float _e421 = color_1.w;
                    const float3 _e422 = corner_sdf(q0_, r0_, k_4);
                    float _e426 = viewport.z;
                    color_1.w = (_e421 * clamp((0.5 + (_e422.x * _e426)), 0.0, 1.0));
                }
            } else {
                float dx = slope.x;
                float dy = slope.y;
                float aa_1 = max(((radius_2 <= 0.0) ? max(dx, dy) : (dx + dy)), 0.5);
                float outer = clamp((0.5 - (distance_ / aa_1)), 0.0, 1.0);
                float _e451 = v_5.shape.y;
                float inner = clamp((0.5 - ((distance_ + _e451) / aa_1)), 0.0, 1.0);
                float _e461 = v_5.border.w;
                float ba = (_e461 * (outer - inner));
                float _e465 = color_1.w;
                float fa = (_e465 * (outer - ba));
                float alpha = (ba + fa);
                float4 _e470 = v_5.border;
                float4 _e473 = color_1;
                float3 rgb_2 = ((alpha > 0.0) ? (((_e470.xyz * ba) + (_e473.xyz * fa)) / (max(alpha, 1e-5)).xxx) : (0.0).xxx);
                color_1 = float4(rgb_2, alpha);
            }
        }
    }
    float _e496 = v_5.options.y;
    if ((_e496 > 0.5)) {
        tex = given;
        if (!(has_texel)) {
            float2 _e504 = v_5.uv;
            float4 _e505 = atlas.Sample(tex_sampler, _e504);
            tex = _e505;
        }
        float _e508 = v_5.options.y;
        if ((_e508 > 1.5)) {
            premultiplied_1 = true;
            float4 _e512 = color_1;
            float4 _e514 = tex;
            float _e518 = color_1.w;
            rgb = ((_e512.xyz * _e514.xyz) * _e518);
            float _e522 = color_1.w;
            float _e524 = tex.w;
            color_1.w = (_e522 * _e524);
        } else {
            float4 _e526 = color_1;
            float4 _e528 = tex;
            float _e532 = color_1.w;
            float _e534 = tex.w;
            float _e537 = v_5.options.y;
            float4 _e538 = color_1;
            const float _e540 = firmness(_e537, _e538.xyz);
            const float _e541 = firm(_e534, _e540);
            color_1 = float4((_e526.xyz * _e528.xyz), (_e532 * _e541));
        }
    }
    float _e546 = v_5.options.x;
    if ((_e546 > 0.0)) {
        float2 _e550 = v_5.point_;
        float4 _e552 = v_5.fade;
        float4 _e556 = v_5.fade;
        float4 _e559 = v_5.fade;
        float2 _e563 = v_5.point_;
        float2 distance_1 = min((_e550 - _e552.xy), ((_e556.xy + _e559.zw) - _e563));
        float _e571 = v_5.options.x;
        float f_1 = clamp((min(distance_1.x, distance_1.y) / _e571), 0.0, 1.0);
        float _e577 = color_1.w;
        color_1.w = (_e577 * f_1);
        float _e579 = scale;
        scale = (_e579 * f_1);
    }
    float _e583 = v_5.mask.x;
    if ((_e583 > 0.5)) {
        float _e588 = v_5.mask.x;
        float4 _e590 = v_5.position;
        const float _e592 = region(_e588, _e590.xy);
        float _e594 = color_1.w;
        color_1.w = (_e594 * _e592);
        float _e596 = scale;
        scale = (_e596 * _e592);
    }
    float _e600 = v_5.mask.x;
    float4 _e601 = color_1;
    float _e604 = color_1.w;
    float _e607 = color_1.w;
    float3 _e609 = rgb;
    float _e610 = scale;
    float _e613 = color_1.w;
    bool _e615 = premultiplied_1;
    const float4 _e617 = tinted(_e600, (_e615 ? float4((_e609 * _e610), _e613) : float4((_e601.xyz * _e604), _e607)));
    return _e617;
}

float4 shade(Out input_2)
{
    const float4 _e4 = shade_texel(input_2, (0.0).xxxx, false);
    return _e4;
}

float4 basic(Basic v_6)
{
    if ((v_6.texture_ > 0.5)) {
        float4 tex_1 = atlas.Sample(tex_sampler, v_6.uv);
        const float _e12 = firm(tex_1.w, v_6.firmness);
        float a_2 = (v_6.color.w * _e12);
        return float4(((v_6.color.xyz * tex_1.xyz) * ((v_6.texture_ > 1.5) ? v_6.color.w : a_2)), a_2);
    }
    return float4((v_6.color.xyz * v_6.color.w), v_6.color.w);
}

float zgx_lum(float3 c)
{
    return dot(c, float3(0.3, 0.59, 0.11));
}

float3 zgx_clip_color(float3 c_1)
{
    float3 o_2 = (float3)0;

    const float _e1 = zgx_lum(c_1);
    float n_1 = min(min(c_1.x, c_1.y), c_1.z);
    float x_1 = max(max(c_1.x, c_1.y), c_1.z);
    o_2 = c_1;
    if ((n_1 < 0.0)) {
        float3 _e15 = o_2;
        o_2 = ((_e1).xxx + (((_e15 - (_e1).xxx) * _e1) / (max((_e1 - n_1), 1e-6)).xxx));
    }
    if ((x_1 > 1.0)) {
        float3 _e28 = o_2;
        o_2 = ((_e1).xxx + (((_e28 - (_e1).xxx) * (1.0 - _e1)) / (max((x_1 - _e1), 1e-6)).xxx));
    }
    float3 _e41 = o_2;
    return _e41;
}

float3 zgx_set_lum(float3 c_2, float l)
{
    const float _e2 = zgx_lum(c_2);
    const float3 _e6 = zgx_clip_color((c_2 + ((l - _e2)).xxx));
    return _e6;
}

float zgx_sat(float3 c_3)
{
    return (max(max(c_3.x, c_3.y), c_3.z) - min(min(c_3.x, c_3.y), c_3.z));
}

float3 zgx_set_sat(float3 c_4, float s)
{
    float n_2 = min(min(c_4.x, c_4.y), c_4.z);
    float x_2 = max(max(c_4.x, c_4.y), c_4.z);
    if ((x_2 <= n_2)) {
        return (0.0).xxx;
    }
    return (((c_4 - (n_2).xxx) * s) / ((x_2 - n_2)).xxx);
}

float3 zgx_hard_light(float3 cb, float3 cs)
{
    float3 multiply = ((cb * 2.0) * cs);
    float3 screen = ((cb + ((2.0 * cs) - (1.0).xxx)) - (cb * ((2.0 * cs) - (1.0).xxx)));
    return ((cs <= (0.5).xxx) ? multiply : screen);
}

float3 zgx_soft_light(float3 cb_1, float3 cs_1)
{
    float3 d_2 = ((cb_1 <= (0.25).xxx) ? (((((16.0 * cb_1) - (12.0).xxx) * cb_1) + (4.0).xxx) * cb_1) : sqrt(cb_1));
    return ((cs_1 <= (0.5).xxx) ? (cb_1 - ((((1.0).xxx - (2.0 * cs_1)) * cb_1) * ((1.0).xxx - cb_1))) : (cb_1 + (((2.0 * cs_1) - (1.0).xxx) * (d_2 - cb_1))));
}

float zgx_dodge(float cb_2, float cs_2)
{
    if ((cb_2 <= 0.0)) {
        return 0.0;
    }
    if ((cs_2 >= 1.0)) {
        return 1.0;
    }
    return min(1.0, (cb_2 / (1.0 - cs_2)));
}

float zgx_burn(float cb_3, float cs_3)
{
    if ((cb_3 >= 1.0)) {
        return 1.0;
    }
    if ((cs_3 <= 0.0)) {
        return 0.0;
    }
    return (1.0 - min(1.0, ((1.0 - cb_3) / cs_3)));
}

float3 zgx_mix(float3 cb_4, float3 cs_4, uint mode)
{
    switch(mode) {
        case 1u: {
            return (cb_4 * cs_4);
        }
        case 2u: {
            return ((cb_4 + cs_4) - (cb_4 * cs_4));
        }
        case 3u: {
            const float3 _e7 = zgx_hard_light(cs_4, cb_4);
            return _e7;
        }
        case 4u: {
            return min(cb_4, cs_4);
        }
        case 5u: {
            return max(cb_4, cs_4);
        }
        case 6u: {
            const float _e12 = zgx_dodge(cb_4.x, cs_4.x);
            const float _e15 = zgx_dodge(cb_4.y, cs_4.y);
            const float _e18 = zgx_dodge(cb_4.z, cs_4.z);
            return float3(_e12, _e15, _e18);
        }
        case 7u: {
            const float _e22 = zgx_burn(cb_4.x, cs_4.x);
            const float _e25 = zgx_burn(cb_4.y, cs_4.y);
            const float _e28 = zgx_burn(cb_4.z, cs_4.z);
            return float3(_e22, _e25, _e28);
        }
        case 8u: {
            const float3 _e30 = zgx_hard_light(cb_4, cs_4);
            return _e30;
        }
        case 9u: {
            const float3 _e31 = zgx_soft_light(cb_4, cs_4);
            return _e31;
        }
        case 10u: {
            return abs((cb_4 - cs_4));
        }
        case 11u: {
            return ((cb_4 + cs_4) - ((2.0 * cb_4) * cs_4));
        }
        case 12u: {
            const float _e39 = zgx_sat(cb_4);
            const float3 _e40 = zgx_set_sat(cs_4, _e39);
            const float _e41 = zgx_lum(cb_4);
            const float3 _e42 = zgx_set_lum(_e40, _e41);
            return _e42;
        }
        case 13u: {
            const float _e43 = zgx_sat(cs_4);
            const float3 _e44 = zgx_set_sat(cb_4, _e43);
            const float _e45 = zgx_lum(cb_4);
            const float3 _e46 = zgx_set_lum(_e44, _e45);
            return _e46;
        }
        case 14u: {
            const float _e47 = zgx_lum(cb_4);
            const float3 _e48 = zgx_set_lum(cs_4, _e47);
            return _e48;
        }
        case 15u: {
            const float _e49 = zgx_lum(cs_4);
            const float3 _e50 = zgx_set_lum(cb_4, _e49);
            return _e50;
        }
        default: {
            return cs_4;
        }
    }
}

float4 zgx_blend(float4 s_1, float4 d, uint mode_1)
{
    float3 cs_5 = ((s_1.w > 0.0) ? (s_1.xyz / (s_1.w).xxx) : (0.0).xxx);
    float3 cb_5 = ((d.w > 0.0) ? (d.xyz / (d.w).xxx) : (0.0).xxx);
    const float3 _e23 = zgx_mix(cb_5, cs_5, mode_1);
    float3 mixed = clamp(_e23, (0.0).xxx, (1.0).xxx);
    return float4(((((1.0 - d.w) * s_1.xyz) + ((1.0 - s_1.w) * d.xyz)) + ((s_1.w * d.w) * mixed)), ((s_1.w + d.w) - (s_1.w * d.w)));
}

int2 naga_f2i32(float2 value) {
    return int2(clamp(value, -2147483600.0, 2147483500.0));
}

float4 zgx_beneath(float4 position_2)
{
    float4 _e5 = zgx_behind.Load(int3(naga_f2i32(position_2.xy), int(0)));
    return _e5;
}

VertexOutput_vs vs(Vertex v, uint i : SV_VertexID)
{
    float2 position = (float2)0;
    float w = 1.0;
    Out o = (Out)0;

    float2 corner_1 = float2(float((i & 1u)), float((i >> 1u)));
    float2 p_3 = (v.rect.xy + (corner_1 * v.rect.zw));
    position = p_3;
    if ((v.options.z > 0.5)) {
        position = float2((dot(v.shape.xy, p_3) + v.border.x), (dot(v.shape.zw, p_3) + v.border.y));
    }
    if ((v.options.z > 1.5)) {
        w = max((dot(v.border.zw, p_3) + 1.0), 0.0001);
    }
    float _e49 = position.x;
    float _e52 = viewport.x;
    float _e56 = w;
    float _e58 = w;
    float _e60 = position.y;
    float _e63 = viewport.y;
    float _e68 = w;
    o.position = float4((((_e49 / _e52) * 2.0) - _e56), (_e58 - ((_e60 / _e63) * 2.0)), 0.0, _e68);
    o.uv = (v.uv.xy + (corner_1 * v.uv.zw));
    o.color = v.color;
    o.point_ = p_3;
    o.fade = v.fade;
    o.options = v.options;
    o.shape = v.shape;
    o.border = v.border;
    o.mask = v.mask;
    Out _e91 = o;
    const Out out_ = _e91;
    const VertexOutput_vs out_1 = { out_.uv, out_.color, out_.point_, out_.fade, out_.options, out_.shape, out_.border, out_.mask, out_.position };
    return out_1;
}

float4 fs(FragmentInput_fs fragmentinput_fs) : SV_Target0
{
    Out v_1 = { fragmentinput_fs.position_4, fragmentinput_fs.uv_2, fragmentinput_fs.color_3, fragmentinput_fs.point_1, fragmentinput_fs.fade_1, fragmentinput_fs.options_2, fragmentinput_fs.shape_1, fragmentinput_fs.border_1, fragmentinput_fs.mask_1 };
    const float4 _e1 = shade(v_1);
    return _e1;
}

Both ConstructBoth(float4 arg0, float4 arg1) {
    Both ret = (Both)0;
    ret.retained = arg0;
    ret.drawable = arg1;
    return ret;
}

Both fs_both(FragmentInput_fs_both fragmentinput_fs_both)
{
    Out v_2 = { fragmentinput_fs_both.position_5, fragmentinput_fs_both.uv_3, fragmentinput_fs_both.color_4, fragmentinput_fs_both.point_2, fragmentinput_fs_both.fade_2, fragmentinput_fs_both.options_3, fragmentinput_fs_both.shape_2, fragmentinput_fs_both.border_2, fragmentinput_fs_both.mask_2 };
    const float4 _e1 = shade(v_2);
    const Both both = ConstructBoth(_e1, _e1);
    return both;
}

VertexOutput_vs_basic vs_basic(float4 rect : LOC0, float4 uv : LOC1, float4 color : LOC2, float4 options : LOC4, uint i_1 : SV_VertexID)
{
    Basic o_1 = (Basic)0;

    float2 corner_2 = float2(float((i_1 & 1u)), float((i_1 >> 1u)));
    float2 p_4 = (rect.xy + (corner_2 * rect.zw));
    float _e21 = viewport.x;
    float _e30 = viewport.y;
    o_1.position = float4((((p_4.x / _e21) * 2.0) - 1.0), (1.0 - ((p_4.y / _e30) * 2.0)), 0.0, 1.0);
    o_1.uv = (uv.xy + (corner_2 * uv.zw));
    o_1.color = color;
    o_1.texture_ = options.y;
    const float _e50 = firmness(options.y, color.xyz);
    o_1.firmness = _e50;
    Basic _e51 = o_1;
    const Basic basic_1 = _e51;
    const VertexOutput_vs_basic basic_1_ = { basic_1.uv, basic_1.color, basic_1.texture_, basic_1.firmness, basic_1.position };
    return basic_1_;
}

float4 fs_basic(FragmentInput_fs_basic fragmentinput_fs_basic) : SV_Target0
{
    Basic v_3 = { fragmentinput_fs_basic.position_7, fragmentinput_fs_basic.uv_5, fragmentinput_fs_basic.color_6, fragmentinput_fs_basic.texture_1, fragmentinput_fs_basic.firmness_2 };
    const float4 _e1 = basic(v_3);
    return _e1;
}

Both fs_basic_both(FragmentInput_fs_basic_both fragmentinput_fs_basic_both)
{
    Basic v_4 = { fragmentinput_fs_basic_both.position_8, fragmentinput_fs_basic_both.uv_6, fragmentinput_fs_basic_both.color_7, fragmentinput_fs_basic_both.texture_2, fragmentinput_fs_basic_both.firmness_3 };
    const float4 _e1 = basic(v_4);
    const Both both_1 = ConstructBoth(_e1, _e1);
    return both_1;
}

float4 zgx_blended(FragmentInput_zgx_blended fragmentinput_zgx_blended) : SV_Target0
{
    Out input = { fragmentinput_zgx_blended.position_9, fragmentinput_zgx_blended.uv_7, fragmentinput_zgx_blended.color_8, fragmentinput_zgx_blended.point_3, fragmentinput_zgx_blended.fade_3, fragmentinput_zgx_blended.options_4, fragmentinput_zgx_blended.shape_3, fragmentinput_zgx_blended.border_3, fragmentinput_zgx_blended.mask_3 };
    const float4 _e1 = shade(input);
    const float4 _e3 = zgx_beneath(input.position);
    const float4 _e7 = zgx_blend(_e1, _e3, naga_f2u32(input.mask.z));
    return _e7;
}
