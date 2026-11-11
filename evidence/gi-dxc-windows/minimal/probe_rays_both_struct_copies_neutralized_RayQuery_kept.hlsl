struct NagaConstants {
    int first_vertex;
    int first_instance;
    uint other;
};
ConstantBuffer<NagaConstants> _NagaConstants: register(b3);

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

struct Trace {
    row_major float4x4 turn;
    uint first;
    uint count;
    uint rays;
    uint total;
    float far;
    float lift;
    uint lights;
    uint seed;
};

struct RayInstance {
    float4 albedo;
    float4 emissive;
    uint4 geometry;
};

struct RayDesc_ {
    uint flags;
    uint cull_mask;
    float tmin;
    float tmax;
    float3 origin;
    int _pad5_0;
    float3 dir;
    int _end_pad_0;
};

struct RayIntersection {
    uint kind;
    float t;
    uint instance_custom_data;
    uint instance_index;
    uint sbt_record_offset;
    uint geometry_index;
    uint primitive_index;
    float2 barycentrics;
    bool front_face;
    int _pad9_0;
    int _pad9_1;
    row_major float4x3 object_to_world;
    int _pad10_0;
    row_major float4x3 world_to_object;
    int _end_pad_0;
};

RayDesc RayDescFromRayDesc_(RayDesc_ arg0) {
    RayDesc ret = (RayDesc)0;
    ret.Origin = arg0.origin;
    ret.TMin = arg0.tmin;
    ret.Direction = arg0.dir;
    ret.TMax = arg0.tmax;
    return ret;
}

static const float PI = 3.1415927;

cbuffer frame : register(b0) { Frame frame; }
Texture2D<float4> probe_irradiance : register(t0);
Texture2D<float4> probe_depth : register(t1);
cbuffer probe_set : register(b1) { ProbeSet probe_set; }
SamplerState nagaSamplerHeap[2048]: register(s0, space0);
SamplerComparisonState nagaComparisonSamplerHeap[2048]: register(s2048, space0);
StructuredBuffer<uint> nagaGroup0SamplerIndexArray : register(t7, space0);
static const SamplerState probe_sampler = nagaSamplerHeap[nagaGroup0SamplerIndexArray[0]];
RaytracingAccelerationStructure scene : register(t2);
ByteAddressBuffer vertices_ : register(t3);
ByteAddressBuffer indices_ : register(t4);
ByteAddressBuffer instances : register(t5);
RWByteAddressBuffer rays : register(u0);
cbuffer trace : register(b2) { Trace trace; }
ByteAddressBuffer lights : register(t6);

float2 octahedral(float3 d)
{
    float2 p_1 = (d.xy / (max(((abs(d.x) + abs(d.y)) + abs(d.z)), 1e-12)).xx);
    if ((d.z >= 0.0)) {
        return p_1;
    }
    return (((1.0).xx - abs(p_1.yx)) * ((p_1 >= (0.0).xx) ? (1.0).xx : (-1.0).xx));
}

uint naga_mod(uint lhs, uint rhs) {
    return lhs % (rhs == 0u ? 1u : rhs);
}

uint naga_div(uint lhs, uint rhs) {
    return lhs / (rhs == 0u ? 1u : rhs);
}

float2 probe_uv(uint index, float3 d_1, float tile, float2 texel)
{
    float2 corner_1 = (float2(float(naga_mod(index, 64u)), float(naga_div(index, 64u))) * tile);
    const float2 _e15 = octahedral(d_1);
    return (((corner_1 + (1.0).xx) + (((_e15 * 0.5) + (0.5).xx) * (tile - 2.0))) * texel);
}

uint naga_f2u32(float value) {
    return uint(clamp(value, 0.0, 4294967000.0));
}

float4 probe_light(float3 p, float3 n_1, float3 v)
{ return (0.0).xxxx; }

float3 fibonacci(uint i_1, uint n_2)
{
    float phi = (6.2831855 * frac((float(i_1) * 0.618034)));
    float z = (1.0 - (((2.0 * float(i_1)) + 1.0) / float(n_2)));
    float r = sqrt(max((1.0 - (z * z)), 0.0));
    return float3((r * cos(phi)), (r * sin(phi)), z);
}

float3 ray_direction(uint i_2)
{
    float4x4 _e3 = trace.turn;
    uint _e6 = trace.rays;
    const float3 _e7 = fibonacci(i_2, _e6);
    return normalize(mul(float4(_e7, 0.0), _e3).xyz);
}

float3 probe_place(uint index_1)
{
    uint i_3 = 0u;
    bool local_2 = (bool)0;

    uint2 loop_bound_2 = uint2(4294967295u, 4294967295u);
    bool loop_init_2 = true;
    while(true) {
        if (all(loop_bound_2 == uint2(0u, 0u))) { break; }
        loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
        if (!loop_init_2) {
            uint _e57 = i_3;
            i_3 = (_e57 + 1u);
        }
        loop_init_2 = false;
        uint _e3 = i_3;
        uint _e7 = probe_set.count.x;
        if ((_e3 < _e7)) {
        } else {
            break;
        }
        {
            uint _e11 = i_3;
            ProbeVolume volume_1 = probe_set.volumes[0];
            uint first = naga_f2u32(volume_1.origin.w);
            uint n_6 = ((volume_1.counts.x * volume_1.counts.y) * volume_1.counts.z);
            if ((index_1 >= first)) {
                local_2 = (index_1 < (first + n_6));
            } else {
                local_2 = false;
            }
            bool _e31 = local_2;
            if (_e31) {
                uint local_5 = (index_1 - first);
                uint3 at_4 = uint3(naga_mod(local_5, volume_1.counts.x), naga_mod(naga_div(local_5, volume_1.counts.x), volume_1.counts.y), naga_div(local_5, (volume_1.counts.x * volume_1.counts.y)));
                return (volume_1.origin.xyz + (float3(at_4) * volume_1.spacing.xyz));
            }
        }
    }
    return (0.0).xxx;
}

RayDesc_ ConstructRayDesc_(uint arg0, uint arg1, float arg2, float arg3, float3 arg4, float3 arg5) {
    RayDesc_ ret = (RayDesc_)0;
    ret.flags = arg0;
    ret.cull_mask = arg1;
    ret.tmin = arg2;
    ret.tmax = arg3;
    ret.origin = arg4;
    ret.dir = arg5;
    return ret;
}

RayIntersection GetCommittedIntersection(RayQuery<RAY_FLAG_NONE> rq, uint rq_tracker) {
    RayIntersection ret = (RayIntersection)0;
    if (((rq_tracker & 4) == 4)) {
        ret.kind = rq.CommittedStatus();
        if( rq.CommittedStatus() == COMMITTED_NOTHING) {} else {
            ret.t = rq.CommittedRayT();
            ret.instance_custom_data = rq.CommittedInstanceID();
            ret.instance_index = rq.CommittedInstanceIndex();
            ret.sbt_record_offset = rq.CommittedInstanceContributionToHitGroupIndex();
            ret.geometry_index = rq.CommittedGeometryIndex();
            ret.primitive_index = rq.CommittedPrimitiveIndex();
            if( rq.CommittedStatus() == COMMITTED_TRIANGLE_HIT ) {
                ret.barycentrics = rq.CommittedTriangleBarycentrics();
                ret.front_face = rq.CommittedTriangleFrontFace();
            }
            ret.object_to_world = rq.CommittedObjectToWorld4x3();
            ret.world_to_object = rq.CommittedWorldToObject4x3();
        }
    }
    return ret;
}

bool blocked(float3 start, float3 d_2, float distance_)
{
    RayQuery<RAY_FLAG_NONE> q_1;
    uint naga_query_init_tracker_for_q_1 = 0;

    {
        RayDesc_ naga_desc = ConstructRayDesc_(5u, 255u, 0.0, distance_, start, d_2);
        float naga_tmin = naga_desc.tmin;
        float naga_tmax = naga_desc.tmax;
        float3 naga_origin = naga_desc.origin;
        float3 naga_dir = naga_desc.dir;
        uint naga_flags = naga_desc.flags;
        bool naga_tmin_valid = (naga_tmin >= 0.0) && (naga_tmin <= naga_tmax) && !(((asuint(naga_tmin) & 2139095040) == 2139095040) && ((asuint(naga_tmin) & 0x7fffff) != 0));
        bool naga_tmax_valid = !(((asuint(naga_tmax) & 2139095040) == 2139095040) && ((asuint(naga_tmax) & 0x7fffff) != 0));
        bool naga_origin_valid = !any((((asuint(naga_origin) & 2139095040) == 2139095040) && ((asuint(naga_origin) & 0x7fffff) != 0)));
        bool naga_dir_valid = !any((((asuint(naga_dir) & 2139095040) == 2139095040) && ((asuint(naga_dir) & 0x7fffff) != 0)));
        bool naga_contains_opaque = ((naga_flags & 1) == 1);
        bool naga_contains_no_opaque = ((naga_flags & 2) == 2);
        bool naga_contains_cull_opaque = ((naga_flags & 64) == 64);
        bool naga_contains_cull_no_opaque = ((naga_flags & 128) == 128);
        bool naga_contains_cull_front = ((naga_flags & 32) == 32);
        bool naga_contains_cull_back = ((naga_flags & 16) == 16);
        bool naga_contains_skip_triangles = ((naga_flags & 256) == 256);
        bool naga_contains_skip_aabbs = ((naga_flags & 512) == 512);
        bool naga_contains_skip_triangles_aabbs =  (naga_contains_skip_aabbs && naga_contains_skip_triangles) ;
        bool naga_contains_skip_triangles_cull =  (naga_contains_cull_front && naga_contains_skip_triangles) || (naga_contains_cull_front && naga_contains_cull_back) || (naga_contains_cull_back && naga_contains_skip_triangles) ;
        bool naga_contains_multiple_opaque =  (naga_contains_cull_no_opaque && naga_contains_opaque) || (naga_contains_cull_no_opaque && naga_contains_no_opaque) || (naga_contains_cull_no_opaque && naga_contains_cull_opaque) || (naga_contains_cull_opaque && naga_contains_opaque) || (naga_contains_cull_opaque && naga_contains_no_opaque) || (naga_contains_no_opaque && naga_contains_opaque) ;
        if (naga_tmin_valid && naga_tmax_valid && naga_origin_valid && naga_dir_valid && !(naga_contains_skip_triangles_aabbs || naga_contains_skip_triangles_cull || naga_contains_multiple_opaque)) {
            naga_query_init_tracker_for_q_1 = naga_query_init_tracker_for_q_1 | 1;
            q_1.TraceRayInline(scene, naga_desc.flags, naga_desc.cull_mask, RayDescFromRayDesc_(naga_desc));
        }
    }
    bool _e9 = false;
    {
        bool naga_has_initialized = ((naga_query_init_tracker_for_q_1 & 1) == 1);
        bool naga_has_finished = ((naga_query_init_tracker_for_q_1 & 4) == 4);
        if (naga_has_initialized && !naga_has_finished) {
            _e9 = q_1.Proceed();
            naga_query_init_tracker_for_q_1 = naga_query_init_tracker_for_q_1 | 2;
            if (!_e9) { naga_query_init_tracker_for_q_1 = naga_query_init_tracker_for_q_1 | 4; }
    }}
    return (GetCommittedIntersection(q_1, naga_query_init_tracker_for_q_1).kind != 0u);
}

float3 sky_along(float3 d_3)
{
    float3 light_1 = (float3)0;

    float4 _e3 = frame.ground;
    float4 _e7 = frame.sky;
    light_1 = ((d_3.y >= 0.0) ? _e7.xyz : _e3.xyz);
    float3 _e14 = light_1;
    return _e14;
}

float3 around(float3 n_3)
{
    float3 light_2 = (float3)0;

    float4 _e3 = frame.ground;
    float4 _e7 = frame.sky;
    light_2 = lerp(_e3.xyz, _e7.xyz, ((n_3.y * 0.5) + 0.5));
    float3 _e16 = light_2;
    return _e16;
}

typedef float4 ret_Constructarray3_float4_[3];
ret_Constructarray3_float4_ Constructarray3_float4_(float4 arg0, float4 arg1, float4 arg2) {
    float4 ret[3] = { arg0, arg1, arg2 };
    return ret;
}

float4 lamp(uint i_4, float3 at, float3 n_4)
{
    bool local_3 = (bool)0;

    float4 point_[3] = Constructarray3_float4_(asfloat(lights.Load4(i_4*48+0)), asfloat(lights.Load4(i_4*48+16)), asfloat(lights.Load4(i_4*48+32)));
    float3 to = (point_[0].xyz - at);
    float distance_3 = max(length(to), 0.0001);
    float3 l = (to / (distance_3).xxx);
    float n_l = dot(n_4, l);
    if (!((n_l <= 0.0))) {
        local_3 = (distance_3 >= point_[0].w);
    } else {
        local_3 = true;
    }
    bool _e24 = local_3;
    if (_e24) {
        return (0.0).xxxx;
    }
    float falloff = (pow(clamp((1.0 - pow((distance_3 / point_[0].w), 4.0)), 0.0, 1.0), 2.0) / (distance_3 * distance_3));
    float cone = clamp((dot(point_[2].xyz, -(l)) + point_[2].w), 0.0, 1.0);
    return float4((((((point_[1].xyz * point_[1].w) * falloff) * cone) * cone) * n_l), distance_3);
}

float3 toward(uint i_5, float3 at_1, float distance_1)
{
    float4 _e6 = asfloat(lights.Load4(0+i_5*48));
    return ((_e6.xyz - at_1) / (distance_1).xxx);
}

float luminance(float3 c)
{
    return dot(c, float3(0.2126, 0.7152, 0.0722));
}

uint hash(uint x)
{
    uint h = (uint)0;

    h = ((x * 747796405u) + 2891336453u);
    uint _e6 = h;
    uint _e7 = h;
    uint _e13 = h;
    h = (((_e6 >> ((_e7 >> 28u) + 4u)) ^ _e13) * 277803737u);
    uint _e17 = h;
    uint _e20 = h;
    return ((_e17 >> 22u) ^ _e20);
}

float3 lamps(float3 at_2, float3 n_5, float3 lifted, uint ray)
{
    float3 light_3 = (0.0).xxx;
    uint i_6 = 0u;
    float total = 0.0;
    uint i_7 = 0u;
    float pick = (float)0;
    uint i_8 = 0u;
    bool local_4 = (bool)0;

    uint _e9 = trace.lights;
    if ((_e9 <= 8u)) {
        uint2 loop_bound_3 = uint2(4294967295u, 4294967295u);
        bool loop_init_3 = true;
        while(true) {
            if (all(loop_bound_3 == uint2(0u, 0u))) { break; }
            loop_bound_3 -= uint2(loop_bound_3.y == 0u, 1u);
            if (!loop_init_3) {
                uint _e40 = i_6;
                i_6 = (_e40 + 1u);
            }
            loop_init_3 = false;
            uint _e14 = i_6;
            uint _e17 = trace.lights;
            if ((_e14 < _e17)) {
            } else {
                break;
            }
            {
                uint _e19 = i_6;
                const float4 _e20 = lamp(_e19, at_2, n_5);
                if (any((_e20.xyz > (0.0).xxx))) {
                    uint _e26 = i_6;
                    const float3 _e28 = toward(_e26, at_2, _e20.w);
                    float _e32 = trace.lift;
                    const bool _e34 = blocked(lifted, _e28, (_e20.w - _e32));
                    if (!(_e34)) {
                        float3 _e36 = light_3;
                        light_3 = (_e36 + _e20.xyz);
                    }
                }
            }
        }
        float3 _e42 = light_3;
        return (_e42 / (3.1415927).xxx);
    }
    uint2 loop_bound_4 = uint2(4294967295u, 4294967295u);
    bool loop_init_4 = true;
    while(true) {
        if (all(loop_bound_4 == uint2(0u, 0u))) { break; }
        loop_bound_4 -= uint2(loop_bound_4.y == 0u, 1u);
        if (!loop_init_4) {
            uint _e62 = i_7;
            i_7 = (_e62 + 1u);
        }
        loop_init_4 = false;
        uint _e50 = i_7;
        uint _e53 = trace.lights;
        if ((_e50 < _e53)) {
        } else {
            break;
        }
        {
            float _e55 = total;
            uint _e56 = i_7;
            const float4 _e57 = lamp(_e56, at_2, n_5);
            const float _e59 = luminance(_e57.xyz);
            total = (_e55 + _e59);
        }
    }
    float _e64 = total;
    if ((_e64 <= 0.0)) {
        return (0.0).xxx;
    }
    uint _e73 = trace.seed;
    const uint _e77 = hash(((ray * 9781u) + (_e73 * 6271u)));
    float _e83 = total;
    pick = ((float((_e77 >> 8u)) / 16777216.0) * _e83);
    uint2 loop_bound_5 = uint2(4294967295u, 4294967295u);
    bool loop_init_5 = true;
    while(true) {
        if (all(loop_bound_5 == uint2(0u, 0u))) { break; }
        loop_bound_5 -= uint2(loop_bound_5.y == 0u, 1u);
        if (!loop_init_5) {
            uint _e135 = i_8;
            i_8 = (_e135 + 1u);
        }
        loop_init_5 = false;
        uint _e88 = i_8;
        uint _e91 = trace.lights;
        if ((_e88 < _e91)) {
        } else {
            break;
        }
        {
            uint _e93 = i_8;
            const float4 _e94 = lamp(_e93, at_2, n_5);
            const float _e96 = luminance(_e94.xyz);
            if ((_e96 <= 0.0)) {
                continue;
            }
            float _e99 = pick;
            pick = (_e99 - _e96);
            float _e101 = pick;
            if (!((_e101 <= 0.0))) {
                uint _e107 = i_8;
                uint _e112 = trace.lights;
                local_4 = ((_e107 + 1u) == _e112);
            } else {
                local_4 = true;
            }
            bool _e115 = local_4;
            if (_e115) {
                uint _e116 = i_8;
                const float3 _e118 = toward(_e116, at_2, _e94.w);
                float _e122 = trace.lift;
                const bool _e124 = blocked(lifted, _e118, (_e94.w - _e122));
                if (_e124) {
                    return (0.0).xxx;
                }
                float _e128 = total;
                return ((_e94.xyz * (_e128 / _e96)) / (3.1415927).xxx);
            }
        }
    }
    return (0.0).xxx;
}

float3 vertex_at(uint i_9)
{
    float _e5 = asfloat(vertices_.Load((i_9 * 8u)*4));
    float _e12 = asfloat(vertices_.Load(((i_9 * 8u) + 1u)*4));
    float _e19 = asfloat(vertices_.Load(((i_9 * 8u) + 2u)*4));
    return float3(_e5, _e12, _e19);
}

float3 normal_at(uint i_10)
{
    float _e7 = asfloat(vertices_.Load(((i_10 * 8u) + 3u)*4));
    float _e14 = asfloat(vertices_.Load(((i_10 * 8u) + 4u)*4));
    float _e21 = asfloat(vertices_.Load(((i_10 * 8u) + 5u)*4));
    return float3(_e7, _e14, _e21);
}

RayInstance ConstructRayInstance(float4 arg0, float4 arg1, uint4 arg2) {
    RayInstance ret = (RayInstance)0;
    ret.albedo = arg0;
    ret.emissive = arg1;
    ret.geometry = arg2;
    return ret;
}

[numthreads(64, 1, 1)]
void trace_rays(uint3 id : SV_DispatchThreadID)
{
    RayQuery<RAY_FLAG_NONE> q;
    uint naga_query_init_tracker_for_q = 0;
    float3 n = (float3)0;
    float3 light = (0.0).xxx;
    bool local = (bool)0;
    bool local_1 = (bool)0;

    uint _e4 = trace.count;
    uint _e7 = trace.rays;
    if ((id.x >= (_e4 * _e7))) {
        return;
    }
    uint _e13 = trace.rays;
    uint ray_1 = naga_mod(id.x, _e13);
    uint _e17 = trace.first;
    uint _e21 = trace.rays;
    uint _e26 = trace.total;
    uint probe_1 = naga_mod((_e17 + naga_div(id.x, _e21)), _e26);
    const float3 _e28 = probe_place(probe_1);
    const float3 _e29 = ray_direction(ray_1);
    {
        RayDesc_ naga_desc = ConstructRayDesc_(1u, 255u, 0.0, 10000.0, _e28, _e29);
        float naga_tmin = naga_desc.tmin;
        float naga_tmax = naga_desc.tmax;
        float3 naga_origin = naga_desc.origin;
        float3 naga_dir = naga_desc.dir;
        uint naga_flags = naga_desc.flags;
        bool naga_tmin_valid = (naga_tmin >= 0.0) && (naga_tmin <= naga_tmax) && !(((asuint(naga_tmin) & 2139095040) == 2139095040) && ((asuint(naga_tmin) & 0x7fffff) != 0));
        bool naga_tmax_valid = !(((asuint(naga_tmax) & 2139095040) == 2139095040) && ((asuint(naga_tmax) & 0x7fffff) != 0));
        bool naga_origin_valid = !any((((asuint(naga_origin) & 2139095040) == 2139095040) && ((asuint(naga_origin) & 0x7fffff) != 0)));
        bool naga_dir_valid = !any((((asuint(naga_dir) & 2139095040) == 2139095040) && ((asuint(naga_dir) & 0x7fffff) != 0)));
        bool naga_contains_opaque = ((naga_flags & 1) == 1);
        bool naga_contains_no_opaque = ((naga_flags & 2) == 2);
        bool naga_contains_cull_opaque = ((naga_flags & 64) == 64);
        bool naga_contains_cull_no_opaque = ((naga_flags & 128) == 128);
        bool naga_contains_cull_front = ((naga_flags & 32) == 32);
        bool naga_contains_cull_back = ((naga_flags & 16) == 16);
        bool naga_contains_skip_triangles = ((naga_flags & 256) == 256);
        bool naga_contains_skip_aabbs = ((naga_flags & 512) == 512);
        bool naga_contains_skip_triangles_aabbs =  (naga_contains_skip_aabbs && naga_contains_skip_triangles) ;
        bool naga_contains_skip_triangles_cull =  (naga_contains_cull_front && naga_contains_skip_triangles) || (naga_contains_cull_front && naga_contains_cull_back) || (naga_contains_cull_back && naga_contains_skip_triangles) ;
        bool naga_contains_multiple_opaque =  (naga_contains_cull_no_opaque && naga_contains_opaque) || (naga_contains_cull_no_opaque && naga_contains_no_opaque) || (naga_contains_cull_no_opaque && naga_contains_cull_opaque) || (naga_contains_cull_opaque && naga_contains_opaque) || (naga_contains_cull_opaque && naga_contains_no_opaque) || (naga_contains_no_opaque && naga_contains_opaque) ;
        if (naga_tmin_valid && naga_tmax_valid && naga_origin_valid && naga_dir_valid && !(naga_contains_skip_triangles_aabbs || naga_contains_skip_triangles_cull || naga_contains_multiple_opaque)) {
            naga_query_init_tracker_for_q = naga_query_init_tracker_for_q | 1;
            q.TraceRayInline(scene, naga_desc.flags, naga_desc.cull_mask, RayDescFromRayDesc_(naga_desc));
        }
    }
    bool _e37 = false;
    {
        bool naga_has_initialized = ((naga_query_init_tracker_for_q & 1) == 1);
        bool naga_has_finished = ((naga_query_init_tracker_for_q & 4) == 4);
        if (naga_has_initialized && !naga_has_finished) {
            _e37 = q.Proceed();
            naga_query_init_tracker_for_q = naga_query_init_tracker_for_q | 2;
            if (!_e37) { naga_query_init_tracker_for_q = naga_query_init_tracker_for_q | 4; }
    }}
    RayIntersection hit = GetCommittedIntersection(q, naga_query_init_tracker_for_q);
    if ((hit.kind == 0u)) {
        const float3 _e45 = sky_along(_e29);
        float _e48 = trace.far;
        rays.Store4(id.x*16, asuint(float4(_e45, _e48)));
        return;
    }
    if (!(hit.front_face)) {
        rays.Store4(id.x*16, asuint(float4(0.0, 0.0, 0.0, (-(hit.t) * 0.2))));
        return;
    }
    RayInstance instance = ConstructRayInstance(asfloat(instances.Load4(hit.instance_custom_data*48+0)), asfloat(instances.Load4(hit.instance_custom_data*48+16)), asuint(instances.Load4(hit.instance_custom_data*48+32)));
    uint first_1 = (instance.geometry.y + (hit.primitive_index * 3u));
    uint base_1 = instance.geometry.x;
    uint _e77 = asuint(indices_.Load(first_1*4));
    uint i0_ = (base_1 + _e77);
    uint _e83 = asuint(indices_.Load((first_1 + 1u)*4));
    uint i1_ = (base_1 + _e83);
    uint _e89 = asuint(indices_.Load((first_1 + 2u)*4));
    uint i2_ = (base_1 + _e89);
    float3 b = float3(((1.0 - hit.barycentrics.x) - hit.barycentrics.y), hit.barycentrics);
    const float3 _e100 = normal_at(i0_);
    const float3 _e103 = normal_at(i1_);
    const float3 _e107 = normal_at(i2_);
    float3 local_6 = (((_e100 * b.x) + (_e103 * b.y)) + (_e107 * b.z));
    float4x3 w = hit.world_to_object;
    float3 turned = float3(dot(w[0], local_6), dot(w[1], local_6), dot(w[2], local_6));
    n = ((dot(turned, turned) > 1e-24) ? (turned / (max(length(turned), 1e-12)).xxx) : -(_e29));
    float3 _e130 = n;
    if ((dot(_e130, _e29) > 0.0)) {
        float3 _e134 = n;
        n = -(_e134);
    }
    float3 at_5 = (_e28 + (_e29 * hit.t));
    float3 _e139 = n;
    float _e142 = trace.lift;
    float3 lifted_1 = (at_5 + (_e139 * _e142));
    float4 _e150 = frame.sun_direction;
    float3 sun = -(_e150.xyz);
    float3 _e153 = n;
    float facing_1 = dot(_e153, sun);
    if ((facing_1 > 0.0)) {
        float4 _e161 = frame.sun_color;
        local = any((_e161.xyz > (0.0).xxx));
    } else {
        local = false;
    }
    bool _e168 = local;
    if (_e168) {
        const bool _e172 = blocked(lifted_1, sun, 10000.0);
        local_1 = !(_e172);
    } else {
        local_1 = false;
    }
    bool _e175 = local_1;
    if (_e175) {
        float3 _e176 = light;
        float4 _e179 = frame.sun_color;
        light = (_e176 + ((_e179.xyz * facing_1) / (3.1415927).xxx));
    }
    float3 _e186 = light;
    float3 _e187 = n;
    const float3 _e189 = lamps(at_5, _e187, lifted_1, id.x);
    light = (_e186 + _e189);
    float3 _e191 = n;
    const float4 _e193 = probe_light(at_5, _e191, -(_e29));
    float3 _e194 = light;
    float3 _e195 = n;
    const float3 _e196 = around(_e195);
    light = (_e194 + lerp(_e196, _e193.xyz, _e193.w));
    float3 albedo = (instance.albedo.xyz * (1.0 - instance.albedo.w));
    float3 _e211 = light;
    float _e219 = trace.far;
    rays.Store4(id.x*16, asuint(float4(((albedo * _e211) + instance.emissive.xyz), min(hit.t, _e219))));
    return;
}
