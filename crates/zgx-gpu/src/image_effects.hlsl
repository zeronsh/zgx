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

struct VertexOutput_vs {
    float2 uv_1 : LOC0;
    nointerpolation float4 color_2 : LOC1;
    float2 point_ : LOC2;
    nointerpolation float4 fade : LOC3;
    nointerpolation float4 options_1 : LOC4;
    nointerpolation float4 shape : LOC5;
    nointerpolation float4 border : LOC6;
    nointerpolation float4 mask : LOC7;
    float4 position_2 : SV_Position;
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
    float4 position_3 : SV_Position;
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
    float4 position_4 : SV_Position;
};

struct VertexOutput_vs_basic {
    float2 uv_4 : LOC0;
    nointerpolation float4 color_5 : LOC1;
    nointerpolation float texture_ : LOC2;
    nointerpolation float firmness_1 : LOC3;
    float4 position_5 : SV_Position;
};

struct FragmentInput_fs_basic {
    float2 uv_5 : LOC0;
    nointerpolation float4 color_6 : LOC1;
    nointerpolation float texture_1 : LOC2;
    nointerpolation float firmness_2 : LOC3;
    float4 position_6 : SV_Position;
};

struct FragmentInput_fs_basic_both {
    float2 uv_6 : LOC0;
    nointerpolation float4 color_7 : LOC1;
    nointerpolation float texture_2 : LOC2;
    nointerpolation float firmness_3 : LOC3;
    float4 position_7 : SV_Position;
};

struct FragmentInput_zgx_image_effect {
    float2 uv_7 : LOC0;
    nointerpolation float4 color_8 : LOC1;
    float2 point_3 : LOC2;
    nointerpolation float4 fade_3 : LOC3;
    nointerpolation float4 options_4 : LOC4;
    nointerpolation float4 shape_3 : LOC5;
    nointerpolation float4 border_3 : LOC6;
    nointerpolation float4 mask_3 : LOC7;
    float4 position_8 : SV_Position;
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
    float2 q = ((p - b.xy) - half_);
    float radius = ((q.y < 0.0) ? ((q.x > 0.0) ? r.y : r.x) : ((q.x > 0.0) ? r.z : r.w));
    float2 d = ((abs(q) - half_) + (radius).xx);
    float sdf = ((length(max(d, (0.0).xx)) + min(max(d.x, d.y), 0.0)) - radius);
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
    float4 c_1 = float4((premultiplied.xyz / (premultiplied.w).xxx), 1.0);
    float3 rgb_1 = clamp(float3(dot(g.color0_, c_1), dot(g.color1_, c_1), dot(g.color2_, c_1)), (0.0).xxx, (1.0).xxx);
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
        Region o_2 = ConstructRegion(asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+0)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+16)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+32)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+48)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+64)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+80)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+96)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+112)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+128)), asfloat(regions.Load4((naga_f2u32(g_1.tint.w) - 1u)*160+144)));
        float _e42 = k_2;
        const float _e45 = faded(o_2.fade_box, o_2.fade, p_2);
        k_2 = (_e42 * _e45);
    }
    float _e47 = k_2;
    return _e47;
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
    Out v_6 = (Out)0;
    float4 color_1 = (float4)0;
    bool local_2 = (bool)0;
    bool local_3 = (bool)0;
    float corner = (float)0;
    bool local_4 = (bool)0;
    bool premultiplied_1 = false;
    float3 rgb = (0.0).xxx;
    float scale = 1.0;
    float4 tex_1 = (float4)0;

    v_6 = input_1;
    float _e6 = v_6.options.z;
    if ((_e6 < 0.5)) {
        float4 _e11 = v_6.position;
        float _e15 = viewport.z;
        v_6.point_ = (_e11.xy / (_e15).xx);
    }
    float4 _e19 = v_6.color;
    color_1 = _e19;
    float _e23 = v_6.options.z;
    if ((_e23 < 0.5)) {
        float _e30 = v_6.shape.w;
        if (!((_e30 > 0.5))) {
            float _e38 = v_6.shape.z;
            local_3 = (_e38 > 0.0);
        } else {
            local_3 = true;
        }
        bool _e42 = local_3;
        local_2 = _e42;
    } else {
        local_2 = false;
    }
    bool _e44 = local_2;
    if (_e44) {
        float _e47 = v_6.shape.x;
        corner = _e47;
        float _e51 = v_6.options.w;
        if ((_e51 > 0.5)) {
            float _e56 = v_6.point_.x;
            float _e59 = v_6.fade.x;
            float _e62 = v_6.fade.z;
            bool right = (_e56 > (_e59 + (_e62 * 0.5)));
            float _e69 = v_6.point_.y;
            float _e72 = v_6.fade.y;
            float _e75 = v_6.fade.w;
            bool bottom = (_e69 > (_e72 + (_e75 * 0.5)));
            float _e82 = v_6.border.x;
            float _e85 = v_6.border.y;
            float _e89 = v_6.border.w;
            float _e92 = v_6.border.z;
            corner = (bottom ? (right ? _e92 : _e89) : (right ? _e85 : _e82));
        }
        float _e95 = corner;
        float _e98 = v_6.fade.z;
        float _e101 = v_6.fade.w;
        float radius_1 = clamp(_e95, 0.0, (min(_e98, _e101) * 0.5));
        float4 _e108 = v_6.position;
        float4 _e111 = v_6.fade;
        const float2 _e112 = centred(_e108.xy, _e111);
        float4 _e114 = v_6.fade;
        float2 q_1 = (_e112 - ((_e114.zw * 0.5) - (radius_1).xx));
        float distance_ = ((length(max(q_1, (0.0).xx)) + min(max(q_1.x, q_1.y), 0.0)) - radius_1);
        float2 outside = max(q_1, (0.0).xx);
        float reach = length(outside);
        float2 gradient = ((reach > 0.0) ? (outside / (max(reach, 1e-20)).xx) : ((q_1.x > q_1.y) ? float2(1.0, 0.0) : float2(0.0, 1.0)));
        float _e155 = viewport.z;
        float2 slope = (gradient / (_e155).xx);
        float _e160 = v_6.shape.y;
        if ((_e160 < 0.0)) {
            float _e165 = v_6.shape.x;
            uint a_1 = naga_f2u32(_e165);
            float _e169 = v_6.shape.z;
            uint b_2 = naga_f2u32(_e169);
            float4 sides = (float4(float((a_1 & 4095u)), float((a_1 >> 12u)), float((b_2 & 4095u)), float((b_2 >> 12u))) / (16.0).xxxx);
            float _e189 = v_6.point_.x;
            float _e192 = v_6.fade.x;
            float _e195 = v_6.fade.z;
            bool right_1 = (_e189 > (_e192 + (_e195 * 0.5)));
            float _e202 = v_6.point_.y;
            float _e205 = v_6.fade.y;
            float _e208 = v_6.fade.w;
            bool bottom_1 = (_e202 > (_e205 + (_e208 * 0.5)));
            float across = (right_1 ? sides.y : sides.w);
            float down = (bottom_1 ? sides.z : sides.x);
            float4 _e220 = v_6.fade;
            float4 _e225 = v_6.fade;
            float4 box_1 = float4((_e220.xy + sides.wx), max(((_e225.zw - sides.wx) - sides.yz), (0.0).xx));
            float inner_radius = clamp((radius_1 - ((across + down) * 0.5)), 0.0, (min(box_1.z, box_1.w) * 0.5));
            float4 _e247 = v_6.position;
            const float2 _e249 = centred(_e247.xy, box_1);
            float2 iq = (_e249 - ((box_1.zw * 0.5) - (inner_radius).xx));
            float inside = ((length(max(iq, (0.0).xx)) + min(max(iq.x, iq.y), 0.0)) - inner_radius);
            float aa = max((slope.x + slope.y), 0.5);
            if ((box_1.z > 0.0)) {
                local_4 = (box_1.w > 0.0);
            } else {
                local_4 = false;
            }
            bool _e287 = local_4;
            float hollow = (_e287 ? clamp((0.5 - (inside / aa)), 0.0, 1.0) : 0.0);
            float _e291 = color_1.w;
            color_1.w = (_e291 * max((clamp((0.5 - (distance_ / aa)), 0.0, 1.0) - hollow), 0.0));
        } else {
            float _e304 = v_6.shape.z;
            if ((_e304 > 0.0)) {
                float _e309 = v_6.shape.z;
                float x = (distance_ / (_e309 * 1.4142135));
                float x2_ = (x * x);
                float e = (sign(x) * sqrt((1.0 - exp(((-(x2_) * (1.2732395 + (0.147 * x2_))) / (1.0 + (0.147 * x2_)))))));
                float _e332 = color_1.w;
                color_1.w = (_e332 * (0.5 - (0.5 * e)));
            } else {
                float dx = slope.x;
                float dy = slope.y;
                float aa_1 = max(((radius_1 <= 0.0) ? max(dx, dy) : (dx + dy)), 0.5);
                float outer = clamp((0.5 - (distance_ / aa_1)), 0.0, 1.0);
                float _e355 = v_6.shape.y;
                float inner = clamp((0.5 - ((distance_ + _e355) / aa_1)), 0.0, 1.0);
                float _e364 = color_1.w;
                float _e368 = v_6.border.w;
                float alpha = ((_e364 * inner) + (_e368 * (outer - inner)));
                float4 _e372 = color_1;
                float _e375 = color_1.w;
                float4 _e379 = v_6.border;
                float _e383 = v_6.border.w;
                float3 rgb_2 = ((alpha > 0.0) ? ((((_e372.xyz * _e375) * inner) + ((_e379.xyz * _e383) * (outer - inner))) / (max(alpha, 1e-5)).xxx) : (0.0).xxx);
                color_1 = float4(rgb_2, alpha);
            }
        }
    }
    float _e407 = v_6.options.y;
    if ((_e407 > 0.5)) {
        tex_1 = given;
        if (!(has_texel)) {
            float2 _e415 = v_6.uv;
            float4 _e416 = atlas.Sample(tex_sampler, _e415);
            tex_1 = _e416;
        }
        float _e419 = v_6.options.y;
        if ((_e419 > 1.5)) {
            premultiplied_1 = true;
            float4 _e423 = color_1;
            float4 _e425 = tex_1;
            float _e429 = color_1.w;
            rgb = ((_e423.xyz * _e425.xyz) * _e429);
            float _e433 = color_1.w;
            float _e435 = tex_1.w;
            color_1.w = (_e433 * _e435);
        } else {
            float4 _e437 = color_1;
            float4 _e439 = tex_1;
            float _e443 = color_1.w;
            float _e445 = tex_1.w;
            float _e448 = v_6.options.y;
            float4 _e449 = color_1;
            const float _e451 = firmness(_e448, _e449.xyz);
            const float _e452 = firm(_e445, _e451);
            color_1 = float4((_e437.xyz * _e439.xyz), (_e443 * _e452));
        }
    }
    float _e457 = v_6.options.x;
    if ((_e457 > 0.0)) {
        float2 _e461 = v_6.point_;
        float4 _e463 = v_6.fade;
        float4 _e467 = v_6.fade;
        float4 _e470 = v_6.fade;
        float2 _e474 = v_6.point_;
        float2 distance_1 = min((_e461 - _e463.xy), ((_e467.xy + _e470.zw) - _e474));
        float _e482 = v_6.options.x;
        float f_1 = clamp((min(distance_1.x, distance_1.y) / _e482), 0.0, 1.0);
        float _e488 = color_1.w;
        color_1.w = (_e488 * f_1);
        float _e490 = scale;
        scale = (_e490 * f_1);
    }
    float _e494 = v_6.mask.x;
    if ((_e494 > 0.5)) {
        float _e499 = v_6.mask.x;
        float4 _e501 = v_6.position;
        const float _e503 = region(_e499, _e501.xy);
        float _e505 = color_1.w;
        color_1.w = (_e505 * _e503);
        float _e507 = scale;
        scale = (_e507 * _e503);
    }
    float _e511 = v_6.mask.x;
    float4 _e512 = color_1;
    float _e515 = color_1.w;
    float _e518 = color_1.w;
    float3 _e520 = rgb;
    float _e521 = scale;
    float _e524 = color_1.w;
    bool _e526 = premultiplied_1;
    const float4 _e528 = tinted(_e511, (_e526 ? float4((_e520 * _e521), _e524) : float4((_e512.xyz * _e515), _e518)));
    return _e528;
}

float4 shade(Out input_2)
{
    const float4 _e4 = shade_texel(input_2, (0.0).xxxx, false);
    return _e4;
}

float4 basic(Basic v_7)
{
    if ((v_7.texture_ > 0.5)) {
        float4 tex_2 = atlas.Sample(tex_sampler, v_7.uv);
        const float _e12 = firm(tex_2.w, v_7.firmness);
        float a_2 = (v_7.color.w * _e12);
        return float4(((v_7.color.xyz * tex_2.xyz) * ((v_7.texture_ > 1.5) ? v_7.color.w : a_2)), a_2);
    }
    return float4((v_7.color.xyz * v_7.color.w), v_7.color.w);
}

uint2 glyph(uint i_2)
{
    switch(i_2) {
        case 1u: {
            return uint2(402653184u, 12u);
        }
        case 2u: {
            return uint2(1015808u, 0u);
        }
        case 3u: {
            return uint2(139432064u, 0u);
        }
        case 4u: {
            return uint2(340083712u, 17u);
        }
        case 5u: {
            return uint2(581052977u, 17u);
        }
        case 6u: {
            return uint2(35078671u, 30u);
        }
        case 7u: {
            return uint2(588727854u, 14u);
        }
        case 8u: {
            return uint2(597347886u, 14u);
        }
        case 9u: {
            return uint2(368409930u, 10u);
        }
        case 10u: {
            return uint2(561700398u, 15u);
        }
        default: {
            return (0u).xx;
        }
    }
}

float cell_hash(uint2 c)
{
    uint h = (uint)0;

    h = (((c.x * 1664525u) + (c.y * 1013904223u)) + 2654435769u);
    uint _e11 = h;
    uint _e12 = h;
    h = (_e11 ^ (_e12 >> 16u));
    uint _e16 = h;
    h = (_e16 * 2246822519u);
    uint _e19 = h;
    uint _e20 = h;
    h = (_e19 ^ (_e20 >> 13u));
    uint _e24 = h;
    h = (_e24 * 3266489917u);
    uint _e27 = h;
    uint _e28 = h;
    h = (_e27 ^ (_e28 >> 16u));
    uint _e32 = h;
    return (float((_e32 & 16777215u)) / 16777216.0);
}

int2 naga_f2i32(float2 value) {
    return int2(clamp(value, -2147483600.0, 2147483500.0));
}

uint2 naga_f2u32(float2 value) {
    return uint2(clamp(value, 0.0, 4294967000.0));
}

float4 characters(Out v_8)
{
    float shown = 1.0;
    float glyph_alpha = 1.0;
    bool scrambling = false;
    uint scramble = 0u;
    uint index_2 = (uint)0;
    float lit_ = 0.0;
    bool local_5 = (bool)0;

    float _e7 = viewport.z;
    float size = ((v_8.options.y - 2.0) * _e7);
    float3 reveal = v_8.color.xyz;
    float2 cell_size = (float2(6.0, 8.0) * size);
    float2 cell = floor((v_8.position.xy / cell_size));
    float2 to_centre = (((cell + (0.5).xx) * cell_size) - v_8.position.xy);
    float2 _e30 = ddx_fine(v_8.uv);
    float2 _e35 = ddy_fine(v_8.uv);
    float4 at_1 = atlas.SampleLevel(tex_sampler, ((v_8.uv + (_e30 * to_centre.x)) + (_e35 * to_centre.y)), 0.0);
    float4 own = atlas.Sample(tex_sampler, v_8.uv);
    uint2 c_2 = uint2(naga_f2i32(cell));
    if ((reveal.y > 0.0)) {
        float _e62 = viewport.z;
        float4 _e66 = viewport;
        float2 q_2 = (abs(((((v_8.position.xy / (_e62).xx) / _e66.xy) - (0.5).xx) - v_8.border.zw)) * 2.0);
        float2 q4_ = (((q_2 * q_2) * q_2) * q_2);
        shown = smoothstep((reveal.y * 0.35), (reveal.y * 1.35), (reveal.x - sqrt(sqrt((q4_.x + q4_.y)))));
        float _e101 = viewport.z;
        float4 _e105 = viewport;
        float2 qc = (abs(((((((cell + (0.5).xx) * cell_size) / (_e101).xx) / _e105.xy) - (0.5).xx) - v_8.border.zw)) * 2.0);
        float2 qc4_ = (((qc * qc) * qc) * qc);
        const float _e127 = cell_hash(c_2);
        float front = ((reveal.x - sqrt(sqrt((qc4_.x + qc4_.y)))) + ((_e127 - 0.5) * reveal.z));
        float into = (front / reveal.y);
        const float _e142 = cell_hash((c_2 + uint2(77u, 13u)));
        glyph_alpha = ((_e142 < pow(clamp(into, 0.0, 1.0), 0.7)) ? smoothstep(0.0, 0.6, into) : 0.0);
        scrambling = (into < 1.0);
        scramble = naga_f2u32((max(into, 0.0) * 12.0));
    }
    float3 straight = ((at_1.w > 0.0) ? (at_1.xyz / (at_1.w).xxx) : (0.0).xxx);
    index_2 = naga_f2u32((pow(clamp((((dot(straight, float3(0.299, 0.587, 0.114)) * at_1.w) - 0.3) / 0.65), 0.0, 1.0), 1.6) * 10.999));
    bool _e188 = scrambling;
    if (_e188) {
        uint _e190 = scramble;
        uint _e195 = scramble;
        const float _e198 = cell_hash((c_2 + uint2(((_e190 * 7u) + 1u), _e195)));
        index_2 = (1u + naga_f2u32((_e198 * 9.999)));
    }
    uint _e203 = index_2;
    const uint2 _e204 = glyph(_e203);
    uint2 fp = naga_f2u32(floor(((v_8.position.xy - (cell * cell_size)) / (size).xx)));
    if ((fp.x < 5u)) {
        local_5 = (fp.y < 7u);
    } else {
        local_5 = false;
    }
    bool _e224 = local_5;
    if (_e224) {
        lit_ = float(((((fp.y < 6u) ? (_e204.x >> (fp.y * 5u)) : _e204.y) >> (4u - fp.x)) & 1u));
    }
    float3 ink = (straight / (max(max(straight.x, max(straight.y, straight.z)), 0.25)).xxx);
    float _e251 = lit_;
    float _e252 = glyph_alpha;
    float _e254 = shown;
    float k_3 = ((_e251 * _e252) * lerp(1.0, 0.65, _e254));
    float _e265 = shown;
    float _e274 = shown;
    return float4((((ink * at_1.w) * k_3) + (((own.xyz * 0.88) * _e265) * (1.0 - k_3))), ((at_1.w * k_3) + ((own.w * _e274) * (1.0 - k_3))));
}

float4 dithered(Out v_9)
{
    float _e7 = viewport.z;
    float dot_ = ((v_9.options.y - 2.0) * _e7);
    if ((dot_ <= 0.5)) {
        float4 _e14 = atlas.Sample(tex_sampler, v_9.uv);
        return _e14;
    }
    float2 cell_1 = floor((v_9.position.xy / (dot_).xx));
    float2 centre = (((cell_1 + (0.5).xx) * dot_) - v_9.position.xy);
    float2 _e31 = ddx_fine(v_9.uv);
    float2 _e36 = ddy_fine(v_9.uv);
    float4 tex_3 = atlas.SampleLevel(tex_sampler, ((v_9.uv + (_e31 * centre.x)) + (_e36 * centre.y)), 0.0);
    if ((tex_3.w <= 0.0)) {
        return tex_3;
    }
    uint2 c_3 = (uint2(naga_f2i32(cell_1)) & (3u).xx);
    uint a_3 = (c_3.x ^ c_3.y);
    uint b_3 = c_3.y;
    float bayer = float((((((a_3 & 1u) << 3u) | ((b_3 & 1u) << 2u)) | (a_3 & 2u)) | ((b_3 & 2u) >> 1u)));
    float3 straight_1 = (tex_3.xyz / (tex_3.w).xxx);
    float peak = max(straight_1.x, max(straight_1.y, straight_1.z));
    return float4((((peak > ((bayer + 0.5) / 16.0)) ? (straight_1 / (max(peak, 1e-5)).xxx) : (straight_1 * 0.08)) * tex_3.w), tex_3.w);
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
    Out v_1 = { fragmentinput_fs.position_3, fragmentinput_fs.uv_2, fragmentinput_fs.color_3, fragmentinput_fs.point_1, fragmentinput_fs.fade_1, fragmentinput_fs.options_2, fragmentinput_fs.shape_1, fragmentinput_fs.border_1, fragmentinput_fs.mask_1 };
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
    Out v_2 = { fragmentinput_fs_both.position_4, fragmentinput_fs_both.uv_3, fragmentinput_fs_both.color_4, fragmentinput_fs_both.point_2, fragmentinput_fs_both.fade_2, fragmentinput_fs_both.options_3, fragmentinput_fs_both.shape_2, fragmentinput_fs_both.border_2, fragmentinput_fs_both.mask_2 };
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
    Basic v_3 = { fragmentinput_fs_basic.position_6, fragmentinput_fs_basic.uv_5, fragmentinput_fs_basic.color_6, fragmentinput_fs_basic.texture_1, fragmentinput_fs_basic.firmness_2 };
    const float4 _e1 = basic(v_3);
    return _e1;
}

Both fs_basic_both(FragmentInput_fs_basic_both fragmentinput_fs_basic_both)
{
    Basic v_4 = { fragmentinput_fs_basic_both.position_7, fragmentinput_fs_basic_both.uv_6, fragmentinput_fs_basic_both.color_7, fragmentinput_fs_basic_both.texture_2, fragmentinput_fs_basic_both.firmness_3 };
    const float4 _e1 = basic(v_4);
    const Both both_1 = ConstructBoth(_e1, _e1);
    return both_1;
}

float4 zgx_image_effect(FragmentInput_zgx_image_effect fragmentinput_zgx_image_effect) : SV_Target0
{
    Out input = { fragmentinput_zgx_image_effect.position_8, fragmentinput_zgx_image_effect.uv_7, fragmentinput_zgx_image_effect.color_8, fragmentinput_zgx_image_effect.point_3, fragmentinput_zgx_image_effect.fade_3, fragmentinput_zgx_image_effect.options_4, fragmentinput_zgx_image_effect.shape_3, fragmentinput_zgx_image_effect.border_3, fragmentinput_zgx_image_effect.mask_3 };
    Out v_5 = (Out)0;
    float4 tex = (float4)0;

    if ((input.options.y <= 2.0)) {
        const float4 _e5 = shade(input);
        return _e5;
    }
    v_5 = input;
    float _e10 = v_5.options.w;
    if ((_e10 < -0.5)) {
        Out _e13 = v_5;
        const float4 _e14 = characters(_e13);
        tex = _e14;
        float _e18 = v_5.color.w;
        v_5.color = float4(1.0, 1.0, 1.0, _e18);
    } else {
        Out _e23 = v_5;
        const float4 _e24 = dithered(_e23);
        tex = _e24;
    }
    Out _e25 = v_5;
    float4 _e26 = tex;
    const float4 _e28 = shade_texel(_e25, _e26, true);
    return _e28;
}
