struct NagaConstants {
    int first_vertex;
    int first_instance;
    uint other;
};
ConstantBuffer<NagaConstants> _NagaConstants: register(b2);

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

struct ProbeVolume {
    float4 origin;
    float4 spacing;
    float4 inverse;
    uint4 counts;
    float4 lo;
    float4 hi;
    float4 fade;
};

struct ProbeSet {
    ProbeVolume volumes[4];
    uint4 count;
    float4 texel;
};

static const float PI = 3.1415927;

cbuffer frame : register(b0) { Frame frame; }
Texture2D<float> shadow_map : register(t0);
SamplerState nagaSamplerHeap[2048]: register(s0, space0);
SamplerComparisonState nagaComparisonSamplerHeap[2048]: register(s2048, space0);
StructuredBuffer<uint> nagaGroup0SamplerIndexArray : register(t3, space0);
static const SamplerComparisonState shadow_sampler = nagaComparisonSamplerHeap[nagaGroup0SamplerIndexArray[0]];
cbuffer probe_set : register(b1) { ProbeSet probe_set; }
Texture2D<float4> probe_depth : register(t2);
static const SamplerState probe_sampler = nagaSamplerHeap[nagaGroup0SamplerIndexArray[1]];
Texture2D<float4> probe_irradiance : register(t1);

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
    float4 p_1 = mul(float4((world + (n_1 * (_e15 - (_e19 * facing)))), 1.0), _e11);
    float2 uv_1 = float2(((p_1.x * 0.5) + 0.5), (0.5 - (p_1.y * 0.5)));
    if (!(any((uv_1 < (0.0).xx)))) {
        local = any((uv_1 > (1.0).xx));
    } else {
        local = true;
    }
    bool _e50 = local;
    if (!(_e50)) {
        local_1 = (p_1.z > 1.0);
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
    float _e76 = shadow_map.SampleCmpLevelZero(shadow_sampler, (uv_1 + float2(-(half_), -(half_))), p_1.z);
    sum = (_e68 + _e76);
    float _e78 = sum;
    float _e85 = shadow_map.SampleCmpLevelZero(shadow_sampler, (uv_1 + float2(half_, -(half_))), p_1.z);
    sum = (_e78 + _e85);
    float _e87 = sum;
    float _e94 = shadow_map.SampleCmpLevelZero(shadow_sampler, (uv_1 + float2(-(half_), half_)), p_1.z);
    sum = (_e87 + _e94);
    float _e96 = sum;
    float _e102 = shadow_map.SampleCmpLevelZero(shadow_sampler, (uv_1 + float2(half_, half_)), p_1.z);
    sum = (_e96 + _e102);
    float _e104 = sum;
    return (_e104 * 0.25);
}

float ggx(float n_h, float a)
{
    float a2_ = (a * a);
    float d_2 = (((n_h * n_h) * (a2_ - 1.0)) + 1.0);
    return (a2_ / ((PI * d_2) * d_2));
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
    float d_3 = (1.0 - 0.76);
    float new_peak = (1.0 - ((d_3 * d_3) / ((peak + d_3) - 0.76)));
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
    float3 v_2 = mul((color_1 / (0.6).xxx), into);
    float3 a_3 = ((v_2 * (v_2 + (0.0245786).xxx)) - (9.0537e-5).xxx);
    float3 b = ((v_2 * ((0.983729 * v_2) + (0.432951).xxx)) + (0.238081).xxx);
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

float2 octahedral(float3 d)
{
    float2 p_2 = (d.xy / (max(((abs(d.x) + abs(d.y)) + abs(d.z)), 1e-12)).xx);
    if ((d.z >= 0.0)) {
        return p_2;
    }
    return (((1.0).xx - abs(p_2.yx)) * ((p_2 >= (0.0).xx) ? (1.0).xx : (-1.0).xx));
}

uint naga_mod(uint lhs, uint rhs) {
    return lhs % (rhs == 0u ? 1u : rhs);
}

uint naga_div(uint lhs, uint rhs) {
    return lhs / (rhs == 0u ? 1u : rhs);
}

float2 probe_uv(uint index, float3 d_1, float tile, float2 texel_1)
{
    float2 corner_1 = (float2(float(naga_mod(index, 64u)), float(naga_div(index, 64u))) * tile);
    const float2 _e15 = octahedral(d_1);
    return (((corner_1 + (1.0).xx) + (((_e15 * 0.5) + (0.5).xx) * (tile - 2.0))) * texel_1);
}

uint naga_f2u32(float value) {
    return uint(clamp(value, 0.0, 4294967000.0));
}

float4 probe_light(float3 p, float3 n_3, float3 v_1)
{
    uint i_1 = 0u;
    float3 sum_1 = (float3)0;
    float weights = (float)0;
    uint corner = (uint)0;
    float weight = (float)0;

    uint2 loop_bound = uint2(4294967295u, 4294967295u);
    bool loop_init = true;
    while(true) {
        if (all(loop_bound == uint2(0u, 0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e231 = i_1;
            i_1 = (_e231 + 1u);
        }
        loop_init = false;
        uint _e5 = i_1;
        uint _e9 = probe_set.count.x;
        if ((_e5 < _e9)) {
        } else {
            break;
        }
        {
            uint _e13 = i_1;
            ProbeVolume volume = probe_set.volumes[_e13];
            float3 outside = (max(max((volume.lo.xyz - p), (p - volume.hi.xyz)), (0.0).xxx) / volume.fade.xyz);
            float fade = saturate((1.0 - max(outside.x, max(outside.y, outside.z))));
            if ((fade <= 0.0)) {
                continue;
            }
            float3 off = (p + (((n_3 * 0.2) + (v_1 * 0.8)) * volume.inverse.w));
            float3 grid = ((off - volume.origin.xyz) * volume.inverse.xyz);
            float3 last = float3((volume.counts.xyz - (1u).xxx));
            float3 base_2 = clamp(floor(grid), (0.0).xxx, max((last - (1.0).xxx), (0.0).xxx));
            float3 t = clamp((grid - base_2), (0.0).xxx, (1.0).xxx);
            sum_1 = (0.0).xxx;
            weights = 0.0;
            corner = 0u;
            uint2 loop_bound_1 = uint2(4294967295u, 4294967295u);
            bool loop_init_1 = true;
            while(true) {
                if (all(loop_bound_1 == uint2(0u, 0u))) { break; }
                loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
                if (!loop_init_1) {
                    uint _e221 = corner;
                    corner = (_e221 + 1u);
                }
                loop_init_1 = false;
                uint _e83 = corner;
                if ((_e83 < 8u)) {
                } else {
                    break;
                }
                {
                    uint _e86 = corner;
                    uint _e89 = corner;
                    uint _e94 = corner;
                    float3 step_ = float3(uint3((_e86 & 1u), ((_e89 >> 1u) & 1u), ((_e94 >> 2u) & 1u)));
                    float3 at = min((base_2 + step_), last);
                    uint index_1 = ((naga_f2u32(volume.origin.w) + naga_f2u32(at.x)) + (volume.counts.x * (naga_f2u32(at.y) + (volume.counts.y * naga_f2u32(at.z)))));
                    float3 probe = (volume.origin.xyz + (at * volume.spacing.xyz));
                    float3 trilinear = lerp(((1.0).xxx - t), t, step_);
                    float3 apart = (probe - p);
                    float3 toward = (apart / (max(length(apart), 1e-6)).xxx);
                    float facing_1 = ((dot(toward, n_3) + 1.0) * 0.5);
                    weight = ((facing_1 * facing_1) + 0.2);
                    float3 out_ = (off - probe);
                    float distance_ = length(out_);
                    float4 _e157 = probe_set.texel;
                    const float2 _e159 = probe_uv(index_1, (out_ / (max(distance_, 1e-6)).xxx), 16.0, _e157.zw);
                    float4 _e161 = probe_depth.SampleLevel(probe_sampler, _e159, 0.0);
                    float2 moments = _e161.xy;
                    if ((distance_ > moments.x)) {
                        float variance = abs((moments.y - (moments.x * moments.x)));
                        float past = (distance_ - moments.x);
                        float chebyshev = (variance / (variance + (past * past)));
                        float _e176 = weight;
                        weight = (_e176 * max(((chebyshev * chebyshev) * chebyshev), 0.05));
                    }
                    float _e182 = weight;
                    weight = max(_e182, 1e-6);
                    float _e185 = weight;
                    if ((_e185 < 0.2)) {
                        float _e188 = weight;
                        float _e189 = weight;
                        float _e190 = weight;
                        weight = (_e188 * ((_e189 * _e190) * 25.0));
                    }
                    float _e195 = weight;
                    weight = (_e195 * ((trilinear.x * trilinear.y) * trilinear.z));
                    float4 _e207 = probe_set.texel;
                    const float2 _e209 = probe_uv(index_1, n_3, 8.0, _e207.xy);
                    float4 _e211 = probe_irradiance.SampleLevel(probe_sampler, _e209, 0.0);
                    float3 light = _e211.xyz;
                    float3 _e213 = sum_1;
                    float _e214 = weight;
                    sum_1 = (_e213 + (light * _e214));
                    float _e217 = weights;
                    float _e218 = weight;
                    weights = (_e217 + _e218);
                }
            }
            float3 _e223 = sum_1;
            float _e224 = weights;
            return float4((_e223 / (max(_e224, 1e-9)).xxx), fade);
        }
    }
    return (0.0).xxxx;
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

    float3 facing_2 = normalize(in_.normal);
    float3 turning_x = ddx(facing_2);
    float3 turning_y = ddy(facing_2);
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
        float3 v_3 = normalize((_e50.xyz - in_.world));
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
        const float3 _e101 = shade(_e93, metallic_1, roughness_1, _e94, v_3, -(_e97.xyz), _e100);
        lit_ = _e101;
        uint2 loop_bound_2 = uint2(4294967295u, 4294967295u);
        bool loop_init_2 = true;
        while(true) {
            if (all(loop_bound_2 == uint2(0u, 0u))) { break; }
            loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
            if (!loop_init_2) {
                uint _e167 = i;
                i = (_e167 + 1u);
            }
            loop_init_2 = false;
            uint _e104 = i;
            float _e108 = frame.params.x;
            if ((_e104 < naga_f2u32(_e108))) {
            } else {
                break;
            }
            {
                uint _e113 = i;
                float4 light_1[3] = frame.points[_e113];
                float3 to = (light_1[0].xyz - in_.world);
                float distance_1 = max(length(to), 0.0001);
                float falloff = (pow(clamp((1.0 - pow((distance_1 / light_1[0].w), 4.0)), 0.0, 1.0), 2.0) / max((distance_1 * distance_1), 0.0001));
                float cone = clamp((dot(light_1[2].xyz, (-(to) / (distance_1).xxx)) + light_1[2].w), 0.0, 1.0);
                float3 _e151 = lit_;
                float3 _e152 = base;
                float3 _e153 = n;
                const float3 _e164 = shade(_e152, metallic_1, roughness_1, _e153, v_3, (to / (distance_1).xxx), ((((light_1[1].xyz * light_1[1].w) * falloff) * cone) * cone));
                lit_ = (_e151 + _e164);
            }
        }
        float4 _e171 = frame.ground;
        float4 _e175 = frame.sky;
        float _e178 = n.y;
        float3 around = lerp(_e171.xyz, _e175.xyz, ((_e178 * 0.5) + 0.5));
        float3 _e185 = n;
        float3 r = reflect(-(v_3), _e185);
        float4 _e189 = frame.ground;
        float4 _e193 = frame.sky;
        float3 reflected = lerp(_e189.xyz, _e193.xyz, ((r.y * 0.5) + 0.5));
        float3 _e201 = base;
        float3 f0_1 = lerp((0.04).xxx, _e201, metallic_1);
        float3 _e205 = n;
        float n_v_2 = max(dot(_e205, v_3), 0.0);
        float rough = (roughness_1 * roughness_1);
        float3 fresnel_1 = (f0_1 + ((max(((1.0 - rough)).xxx, f0_1) - f0_1) * pow((1.0 - n_v_2), 5.0)));
        float3 _e221 = base;
        surround = (((around * _e221) * (1.0 - metallic_1)) * ((1.0).xxx - fresnel_1));
        float3 _e231 = surround;
        surround = (_e231 + ((reflected * fresnel_1) * (1.0 - (0.5 * rough))));
        float3 _e240 = n;
        const float4 _e241 = probe_light(in_.world, _e240, v_3);
        float3 _e242 = surround;
        float3 _e247 = base;
        surround = (_e242 + (((((_e241.xyz - around) * _e241.w) * _e247) * (1.0 - metallic_1)) * ((1.0).xxx - fresnel_1)));
        float3 _e257 = lit_;
        float3 _e258 = surround;
        float _e259 = occlusion;
        lit_ = (_e257 + (_e258 * _e259));
        float3 _e262 = lit_;
        float3 _e264 = glow;
        lit_ = (_e262 + (in_.emissive * _e264));
        float3 _e267 = lit_;
        float _e271 = frame.params.z;
        const float3 _e273 = tone_map((_e267 * _e271));
        lit_ = _e273;
    }
    float _e277 = frame.params.y;
    if ((_e277 > 0.5)) {
        float3 _e280 = lit_;
        const float3 _e286 = encode(clamp(_e280, (0.0).xxx, (1.0).xxx));
        lit_ = _e286;
    }
    float _e290 = texel.w;
    float alpha = clamp(((in_.color.w < 1.0) ? (in_.color.w * _e290) : 1.0), 0.0, 1.0);
    float3 _e301 = lit_;
    return float4((_e301 * alpha), alpha);
}
