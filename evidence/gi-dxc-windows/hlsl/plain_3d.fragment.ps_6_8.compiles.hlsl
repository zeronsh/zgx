struct NagaConstants {
    int first_vertex;
    int first_instance;
    uint other;
};
ConstantBuffer<NagaConstants> _NagaConstants: register(b1);

struct Frame {
    row_major float4x4 view_projection;
    float4 camera;
    float4 sun_direction;
    float4 sun_color;
    float4 sky;
    float4 ground;
    float4 points[8][3];
    float4 params;
    row_major float4x4 shadow;
    float4 shadow_texel;
    float4 environment;
    float4 irradiance[9];
    float4 look;
};

struct Out {
    float4 position : SV_Position;
    float3 world : LOC0;
    float3 normal : LOC1;
    float4 color : LOC2;
    float3 emissive : LOC3;
    float3 material : LOC4;
    float2 uv : LOC5;
};

static const float PI = 3.1415927;

cbuffer frame : register(b0) { Frame frame; }
Texture2D<float> shadow_map : register(t0);
SamplerState nagaSamplerHeap[2048]: register(s0, space0);
SamplerComparisonState nagaComparisonSamplerHeap[2048]: register(s2048, space0);
StructuredBuffer<uint> nagaGroup0SamplerIndexArray : register(t1, space0);
static const SamplerComparisonState shadow_sampler = nagaComparisonSamplerHeap[nagaGroup0SamplerIndexArray[0]];

struct FragmentInput_fragment {
    float3 world_1 : LOC0;
    float3 normal : LOC1;
    float4 color_3 : LOC2;
    float3 emissive : LOC3;
    float3 material : LOC4;
    float2 uv : LOC5;
    float4 position : SV_Position;
    bool front_1 : SV_IsFrontFace;
};

float sunlit(float3 world, float3 n_1)
{
    bool local = (bool)0;
    bool local_1 = (bool)0;
    float sum = 0.0;

    float4 _e4 = frame.sun_direction;
    float facing = saturate(dot(n_1, -(_e4.xyz)));
    float4x4 _e11 = frame.shadow;
    float _e15 = frame.shadow_texel.z;
    float _e19 = frame.shadow_texel.w;
    float4 p = mul(float4((world + (n_1 * (_e15 - (_e19 * facing)))), 1.0), _e11);
    float2 uv_1 = float2(((p.x * 0.5) + 0.5), (0.5 - (p.y * 0.5)));
    if (!(any((uv_1 < (0.0).xx)))) {
        local = any((uv_1 > (1.0).xx));
    } else {
        local = true;
    }
    bool _e50 = local;
    if (!(_e50)) {
        local_1 = (p.z > 1.0);
    } else {
        local_1 = true;
    }
    bool _e58 = local_1;
    if (_e58) {
        return 1.0;
    }
    float _e63 = frame.shadow_texel.y;
    float half_ = (_e63 * 0.5);
    float _e68 = sum;
    float _e76 = shadow_map.SampleCmpLevelZero(shadow_sampler, (uv_1 + float2(-(half_), -(half_))), p.z);
    sum = (_e68 + _e76);
    float _e78 = sum;
    float _e85 = shadow_map.SampleCmpLevelZero(shadow_sampler, (uv_1 + float2(half_, -(half_))), p.z);
    sum = (_e78 + _e85);
    float _e87 = sum;
    float _e94 = shadow_map.SampleCmpLevelZero(shadow_sampler, (uv_1 + float2(-(half_), half_)), p.z);
    sum = (_e87 + _e94);
    float _e96 = sum;
    float _e102 = shadow_map.SampleCmpLevelZero(shadow_sampler, (uv_1 + float2(half_, half_)), p.z);
    sum = (_e96 + _e102);
    float _e104 = sum;
    return (_e104 * 0.25);
}

float ggx(float n_h, float a)
{
    float a2_ = (a * a);
    float d = (((n_h * n_h) * (a2_ - 1.0)) + 1.0);
    return (a2_ / ((PI * d) * d));
}

float smith(float n_v, float n_l, float a_1)
{
    float k = (a_1 * 0.5);
    return ((n_v / ((n_v * (1.0 - k)) + k)) * (n_l / ((n_l * (1.0 - k)) + k)));
}

float3 shade(float3 base_1, float metallic, float roughness, float3 n_2, float3 v, float3 l, float3 radiance)
{
    float3 h = ((v + l) / (max(length((v + l)), 0.0001)).xxx);
    float n_l_1 = max(dot(n_2, l), 0.0);
    float n_v_1 = max(dot(n_2, v), 0.0001);
    float n_h_1 = max(dot(n_2, h), 0.0);
    float a_2 = (roughness * roughness);
    float3 f0_ = lerp((0.04).xxx, base_1, metallic);
    float3 fresnel = (f0_ + (((1.0).xxx - f0_) * pow((1.0 - max(dot(h, v), 0.0)), 5.0)));
    const float _e39 = ggx(n_h_1, a_2);
    const float _e41 = smith(n_v_1, n_l_1, a_2);
    float3 specular = (((fresnel * _e39) * _e41) / (((4.0 * n_v_1) * max(n_l_1, 0.0001))).xxx);
    float3 diffuse = (((((1.0).xxx - fresnel) * (1.0 - metallic)) * base_1) / (3.1415927).xxx);
    return (((diffuse + specular) * radiance) * n_l_1);
}

float3 neutral(float3 color)
{
    float3 c = (float3)0;

    float x = min(color.x, min(color.y, color.z));
    float offset = ((x < 0.08) ? (x - ((6.25 * x) * x)) : 0.04);
    c = (color - (offset).xxx);
    float _e20 = c.x;
    float _e22 = c.y;
    float _e24 = c.z;
    float peak = max(_e20, max(_e22, _e24));
    if ((peak < 0.76)) {
        float3 _e28 = c;
        return _e28;
    }
    float d_1 = (1.0 - 0.76);
    float new_peak = (1.0 - ((d_1 * d_1) / ((peak + d_1) - 0.76)));
    float3 _e37 = c;
    c = (_e37 * (new_peak / peak));
    float g = (1.0 - (1.0 / ((0.15 * (peak - new_peak)) + 1.0)));
    float3 _e48 = c;
    return lerp(_e48, (new_peak).xxx, g);
}

float3 aces(float3 color_1)
{
    float3x3 into = float3x3(float3(0.59719, 0.076, 0.0284), float3(0.35458, 0.90834, 0.13383), float3(0.04823, 0.01566, 0.83777));
    float3x3 back = float3x3(float3(1.60475, -0.10208, -0.00327), float3(-0.53108, 1.10813, -0.07276), float3(-0.07367, -0.00605, 1.07602));
    float3 v_1 = mul((color_1 / (0.6).xxx), into);
    float3 a_3 = ((v_1 * (v_1 + (0.0245786).xxx)) - (9.0537e-5).xxx);
    float3 b = ((v_1 * ((0.983729 * v_1) + (0.432951).xxx)) + (0.238081).xxx);
    return clamp(mul((a_3 / b), back), (0.0).xxx, (1.0).xxx);
}

float3 tone_map(float3 color_2)
{
    float _e4 = frame.look.x;
    if ((_e4 > 1.5)) {
        return color_2;
    }
    float _e10 = frame.look.x;
    if ((_e10 > 0.5)) {
        const float3 _e13 = aces(color_2);
        return _e13;
    }
    const float3 _e14 = neutral(color_2);
    return _e14;
}

float3 encode(float3 c_1)
{
    float3 low = (c_1 * 12.92);
    float3 high = ((1.055 * pow(c_1, (0.41666666).xxx)) - (0.055).xxx);
    return ((c_1 <= (0.0031308).xxx) ? low : high);
}

uint naga_f2u32(float value) {
    return uint(clamp(value, 0.0, 4294967000.0));
}

float4 fragment(FragmentInput_fragment fragmentinput_fragment) : SV_Target0
{
    Out in_ = { fragmentinput_fragment.position, fragmentinput_fragment.world_1, fragmentinput_fragment.normal, fragmentinput_fragment.color_3, fragmentinput_fragment.emissive, fragmentinput_fragment.material, fragmentinput_fragment.uv };
    bool front = fragmentinput_fragment.front_1;
    float4 texel = (1.0).xxxx;
    float4 surface = (1.0).xxxx;
    float3 bump = float3(0.5, 0.5, 1.0);
    float3 glow = (1.0).xxx;
    float occlusion = 1.0;
    float widening = 0.0;
    float3 base = (float3)0;
    float3 lit_ = (float3)0;
    float3 n = (float3)0;
    float3 sun = (float3)0;
    uint i = 0u;
    float3 surround = (float3)0;

    float3 facing_1 = normalize(in_.normal);
    float3 turning_x = ddx(facing_1);
    float3 turning_y = ddy(facing_1);
    widening = min((0.3 * (dot(turning_x, turning_x) + dot(turning_y, turning_y))), 0.18);
    float4 _e32 = texel;
    base = (in_.color.xyz * _e32.xyz);
    if ((in_.material.z > 0.5)) {
        float3 _e41 = base;
        lit_ = _e41;
    } else {
        n = normalize(in_.normal);
        if (!(front)) {
            float3 _e46 = n;
            n = -(_e46);
        }
        float4 _e50 = frame.camera;
        float3 v_2 = normalize((_e50.xyz - in_.world));
        float _e58 = surface.z;
        float metallic_1 = (in_.material.x * _e58);
        float _e63 = surface.y;
        float given = clamp((in_.material.y * _e63), 0.02, 1.0);
        float a_4 = (given * given);
        float _e70 = widening;
        float roughness_1 = sqrt(sqrt(clamp(((a_4 * a_4) + _e70), 0.0, 1.0)));
        float4 _e79 = frame.sun_color;
        sun = _e79.xyz;
        float _e85 = frame.params.w;
        if ((_e85 > 0.5)) {
            float3 _e88 = sun;
            float3 _e90 = n;
            const float _e91 = sunlit(in_.world, _e90);
            sun = (_e88 * _e91);
        }
        float3 _e93 = base;
        float3 _e94 = n;
        float4 _e97 = frame.sun_direction;
        float3 _e100 = sun;
        const float3 _e101 = shade(_e93, metallic_1, roughness_1, _e94, v_2, -(_e97.xyz), _e100);
        lit_ = _e101;
        uint2 loop_bound = uint2(4294967295u, 4294967295u);
        bool loop_init = true;
        while(true) {
            if (all(loop_bound == uint2(0u, 0u))) { break; }
            loop_bound -= uint2(loop_bound.y == 0u, 1u);
            if (!loop_init) {
                uint _e167 = i;
                i = (_e167 + 1u);
            }
            loop_init = false;
            uint _e104 = i;
            float _e108 = frame.params.x;
            if ((_e104 < naga_f2u32(_e108))) {
            } else {
                break;
            }
            {
                uint _e113 = i;
                float4 light[3] = frame.points[_e113];
                float3 to = (light[0].xyz - in_.world);
                float distance_ = max(length(to), 0.0001);
                float falloff = (pow(clamp((1.0 - pow((distance_ / light[0].w), 4.0)), 0.0, 1.0), 2.0) / max((distance_ * distance_), 0.0001));
                float cone = clamp((dot(light[2].xyz, (-(to) / (distance_).xxx)) + light[2].w), 0.0, 1.0);
                float3 _e151 = lit_;
                float3 _e152 = base;
                float3 _e153 = n;
                const float3 _e164 = shade(_e152, metallic_1, roughness_1, _e153, v_2, (to / (distance_).xxx), ((((light[1].xyz * light[1].w) * falloff) * cone) * cone));
                lit_ = (_e151 + _e164);
            }
        }
        float4 _e171 = frame.ground;
        float4 _e175 = frame.sky;
        float _e178 = n.y;
        float3 ambient = lerp(_e171.xyz, _e175.xyz, ((_e178 * 0.5) + 0.5));
        float3 _e185 = n;
        float3 r = reflect(-(v_2), _e185);
        float4 _e189 = frame.ground;
        float4 _e193 = frame.sky;
        float3 reflected = lerp(_e189.xyz, _e193.xyz, ((r.y * 0.5) + 0.5));
        float3 _e201 = base;
        float3 f0_1 = lerp((0.04).xxx, _e201, metallic_1);
        float3 _e205 = n;
        float n_v_2 = max(dot(_e205, v_2), 0.0);
        float rough = (roughness_1 * roughness_1);
        float3 fresnel_1 = (f0_1 + ((max(((1.0 - rough)).xxx, f0_1) - f0_1) * pow((1.0 - n_v_2), 5.0)));
        float3 _e221 = base;
        surround = (((ambient * _e221) * (1.0 - metallic_1)) * ((1.0).xxx - fresnel_1));
        float3 _e231 = surround;
        surround = (_e231 + ((reflected * fresnel_1) * (1.0 - (0.5 * rough))));
        float3 _e239 = lit_;
        float3 _e240 = surround;
        float _e241 = occlusion;
        lit_ = (_e239 + (_e240 * _e241));
        float3 _e244 = lit_;
        float3 _e246 = glow;
        lit_ = (_e244 + (in_.emissive * _e246));
        float3 _e249 = lit_;
        float _e253 = frame.params.z;
        const float3 _e255 = tone_map((_e249 * _e253));
        lit_ = _e255;
    }
    float _e259 = frame.params.y;
    if ((_e259 > 0.5)) {
        float3 _e262 = lit_;
        const float3 _e268 = encode(clamp(_e262, (0.0).xxx, (1.0).xxx));
        lit_ = _e268;
    }
    float _e272 = texel.w;
    float alpha = clamp(((in_.color.w < 1.0) ? (in_.color.w * _e272) : 1.0), 0.0, 1.0);
    float3 _e283 = lit_;
    return float4((_e283 * alpha), alpha);
}
