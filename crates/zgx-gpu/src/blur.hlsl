struct NagaConstants {
    int first_vertex;
    int first_instance;
    uint other;
};
cbuffer NagaConstantsBuffer : register(b0) { NagaConstants _NagaConstants; }

struct Blur {
    float4 bounds;
    float4 effect;
    float4 area;
    float4 shape;
    uint4 slots;
    float4 round0_;
    float4 radii0_;
    float4 round1_;
    float4 radii1_;
};

struct Slot {
    float4 atlas;
    float4 region;
    uint4 owner;
};

struct Vertex {
    float4 p : SV_Position;
    nointerpolation uint index : LOC0;
};

struct Blend {
    float4 color;
    float weight;
    int _end_pad_0;
    int _end_pad_1;
    int _end_pad_2;
};

struct Both {
    float4 retained : SV_Target0;
    float4 drawable : SV_Target1;
};

struct Mixed {
    float4 color : SV_Target0;
    float4 amount : SV_Target1;
};

Texture2D<float4> source : register(t0);
Texture2D<float4> original : register(t1);
ByteAddressBuffer blurs : register(t2);
ByteAddressBuffer slots : register(t3);
SamplerState linear_ : register(s0);

struct VertexOutput_vs {
    nointerpolation uint index_1 : LOC0;
    float4 p_7 : SV_Position;
};

struct FragmentInput_fs_reduce {
    nointerpolation uint index_2 : LOC0;
    float4 p_8 : SV_Position;
};

struct FragmentInput_fs_across {
    nointerpolation uint index_3 : LOC0;
    float4 p_9 : SV_Position;
};

struct FragmentInput_fs_down {
    nointerpolation uint index_4 : LOC0;
    float4 p_10 : SV_Position;
};

struct FragmentInput_fs {
    nointerpolation uint index_5 : LOC0;
    float4 p_11 : SV_Position;
};

struct FragmentInput_fs_both {
    nointerpolation uint index_6 : LOC0;
    float4 p_12 : SV_Position;
};

struct FragmentInput_fs_mix {
    nointerpolation uint index_7 : LOC0;
    float4 p_13 : SV_Position;
};

float sigma(Blur b, float2 p)
{
    if ((b.effect.y <= 0.0)) {
        return b.effect.z;
    }
    float2 d = min((p - b.bounds.xy), ((b.bounds.xy + b.bounds.zw) - p));
    float t = clamp((min(d.x, d.y) / b.effect.y), 0.0, 1.0);
    return ((b.effect.z * t) * t);
}

float4 tap(int2 start, int x_2, int y_2, float2 size)
{
    float4 _e14 = source.SampleLevel(linear_, ((float2(asint(asuint(start) + asuint(int2(x_2, y_2)))) + (1.0).xx) / size), 0.0);
    return _e14;
}

int mirror(int x_3, int n)
{
    int m = ((x_3 < int(0)) ? asint(asuint(int(-1)) - asuint(x_3)) : x_3);
    return clamp(((m >= n) ? asint(asuint(asint(asuint(asint(asuint(int(2)) * asuint(n))) - asuint(int(1)))) - asuint(m)) : m), int(0), asint(asuint(n) - asuint(int(1))));
}

int2 naga_f2i32(float2 value) {
    return int2(clamp(value, -2147483600.0, 2147483500.0));
}

int naga_f2i32(float value) {
    return int(clamp(value, -2147483600.0, 2147483500.0));
}

Slot ConstructSlot(float4 arg0, float4 arg1, uint4 arg2) {
    Slot ret = (Slot)0;
    ret.atlas = arg0;
    ret.region = arg1;
    ret.owner = arg2;
    return ret;
}

uint2 NagaDimensions2D(Texture2D<float4> tex)
{
    uint4 ret;
    tex.GetDimensions(0, ret.x, ret.y, ret.z);
    return ret.xy;
}

float4 along(Vertex v_6, int2 step_)
{
    float ratio = (float)0;
    float w = 1.0;
    float4 sum_1 = (float4)0;
    float total = 1.0;
    int i_1 = int(1);
    bool local_1 = (bool)0;
    int k = (int)0;

    Slot s = ConstructSlot(asfloat(slots.Load4(v_6.index*48+0)), asfloat(slots.Load4(v_6.index*48+16)), asuint(slots.Load4(v_6.index*48+32)));
    int2 o = naga_f2i32(s.atlas.xy);
    int2 n_1 = naga_f2i32(s.atlas.zw);
    int2 l = asint(asuint(naga_f2i32(v_6.p.xy)) - asuint(o));
    float st = s.region.w;
    int r = naga_f2i32(ceil((3.0 * st)));
    float q = exp((-0.5 / (st * st)));
    int at = dot(l, step_);
    int extent = dot(n_1, step_);
    float2 size_1 = float2(NagaDimensions2D(source));
    float2 axis = float2(step_);
    ratio = q;
    float4 _e38 = source.Load(int3(asint(asuint(o) + asuint(l)), int(0)));
    sum_1 = _e38;
    uint2 loop_bound = uint2(4294967295u, 4294967295u);
    bool loop_init = true;
    while(true) {
        if (all(loop_bound == uint2(0u, 0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e149 = i_1;
            i_1 = asint(asuint(_e149) + asuint(int(2)));
        }
        loop_init = false;
        int _e44 = i_1;
        if ((_e44 <= r)) {
        } else {
            break;
        }
        {
            float _e46 = w;
            float _e47 = ratio;
            w = (_e46 * _e47);
            float _e49 = ratio;
            ratio = (_e49 * (q * q));
            float wa = w;
            float _e53 = w;
            float _e54 = ratio;
            w = (_e53 * _e54);
            float _e56 = ratio;
            ratio = (_e56 * (q * q));
            float _e59 = w;
            int _e60 = i_1;
            float wb = ((_e60 < r) ? _e59 : 0.0);
            int _e64 = i_1;
            if ((asint(asuint(asint(asuint(at) - asuint(_e64))) - asuint(int(1))) >= int(0))) {
                int _e72 = i_1;
                local_1 = (asint(asuint(asint(asuint(at) + asuint(_e72))) + asuint(int(1))) < extent);
            } else {
                local_1 = false;
            }
            bool _e78 = local_1;
            if (_e78) {
                int _e79 = i_1;
                float2 offset = (axis * (float(_e79) + (wb / (wa + wb))));
                float2 center_ = (float2(asint(asuint(o) + asuint(l))) + (0.5).xx);
                float4 _e90 = sum_1;
                float4 _e96 = source.SampleLevel(linear_, ((center_ - offset) / size_1), 0.0);
                float4 _e102 = source.SampleLevel(linear_, ((center_ + offset) / size_1), 0.0);
                sum_1 = (_e90 + ((_e96 + _e102) * (wa + wb)));
            } else {
                k = int(0);
                uint2 loop_bound_1 = uint2(4294967295u, 4294967295u);
                bool loop_init_1 = true;
                while(true) {
                    if (all(loop_bound_1 == uint2(0u, 0u))) { break; }
                    loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
                    if (!loop_init_1) {
                        int _e142 = k;
                        k = asint(asuint(_e142) + asuint(int(1)));
                    }
                    loop_init_1 = false;
                    int _e109 = k;
                    if ((_e109 < int(2))) {
                    } else {
                        break;
                    }
                    {
                        int _e112 = i_1;
                        int _e113 = k;
                        int j = asint(asuint(_e112) + asuint(_e113));
                        const int _e116 = mirror(asint(asuint(at) - asuint(j)), extent);
                        const int _e118 = mirror(asint(asuint(at) + asuint(j)), extent);
                        int2 across = asint(asuint(l) - asuint(asint(asuint(step_) * asuint(at))));
                        float4 _e121 = sum_1;
                        float4 _e127 = source.Load(int3(asint(asuint(asint(asuint(o) + asuint(across))) + asuint(asint(asuint(step_) * asuint(_e116)))), int(0)));
                        float4 _e133 = source.Load(int3(asint(asuint(asint(asuint(o) + asuint(across))) + asuint(asint(asuint(step_) * asuint(_e118)))), int(0)));
                        int _e135 = k;
                        sum_1 = (_e121 + ((_e127 + _e133) * ((_e135 == int(1)) ? wb : wa)));
                    }
                }
            }
            float _e144 = total;
            total = (_e144 + (2.0 * (wa + wb)));
        }
    }
    float4 _e152 = sum_1;
    float _e153 = total;
    return (_e152 / (_e153).xxxx);
}

float coverage(Blur b_1, float2 p_1)
{
    float2 half_ = (b_1.bounds.zw * 0.5);
    float2 q_1 = ((p_1 - b_1.bounds.xy) - half_);
    float r_1 = ((q_1.y < 0.0) ? ((q_1.x > 0.0) ? b_1.shape.y : b_1.shape.x) : ((q_1.x > 0.0) ? b_1.shape.z : b_1.shape.w));
    if ((r_1 <= 0.0)) {
        return 1.0;
    }
    float2 d_1 = ((abs(q_1) - half_) + (r_1).xx);
    float sdf = ((length(max(d_1, (0.0).xx)) + min(max(d_1.x, d_1.y), 0.0)) - r_1);
    return clamp((0.5 - sdf), 0.0, 1.0);
}

float kept(float4 box, float4 radii, float2 p_2)
{
    if ((box.z < 0.0)) {
        return 1.0;
    }
    float2 half_size = (box.zw * 0.5);
    float2 q_2 = ((p_2 - box.xy) - half_size);
    float radius = ((q_2.y < 0.0) ? ((q_2.x > 0.0) ? radii.y : radii.x) : ((q_2.x > 0.0) ? radii.z : radii.w));
    float2 d_2 = ((abs(q_2) - half_size) + (radius).xx);
    return clamp((0.5 - ((length(max(d_2, (0.0).xx)) + min(max(d_2.x, d_2.y), 0.0)) - radius)), 0.0, 1.0);
}

float amount(Blur b_2, float2 p_3)
{
    const float _e4 = coverage(b_2, p_3);
    const float _e8 = kept(b_2.round0_, b_2.radii0_, p_3);
    const float _e12 = kept(b_2.round1_, b_2.radii1_, p_3);
    return (((b_2.effect.x * _e4) * _e8) * _e12);
}

float4 level(Blur b_3, uint k_1, float2 p_4)
{
    Slot s_1 = ConstructSlot(asfloat(slots.Load4((b_3.slots.x + k_1)*48+0)), asfloat(slots.Load4((b_3.slots.x + k_1)*48+16)), asuint(slots.Load4((b_3.slots.x + k_1)*48+32)));
    float2 t_1 = clamp((((p_4 - s_1.region.xy) / (s_1.region.z).xx) - (0.5).xx), (0.0).xx, (s_1.atlas.zw - (1.0).xx));
    float4 _e40 = source.SampleLevel(linear_, (((s_1.atlas.xy + t_1) + (0.5).xx) / float2(NagaDimensions2D(source))), 0.0);
    return _e40;
}

Blend ConstructBlend(float4 arg0, float arg1) {
    Blend ret = (Blend)0;
    ret.color = arg0;
    ret.weight = arg1;
    return ret;
}

uint naga_f2u32(float value) {
    return uint(clamp(value, 0.0, 4294967000.0));
}

Blend blurred(Blur b_4, float2 p_5)
{
    uint n_2 = b_4.slots.y;
    if ((n_2 == 1u)) {
        const float4 _e7 = level(b_4, 0u, p_5);
        const Blend blend = ConstructBlend(_e7, 1.0);
        return blend;
    }
    const float _e10 = sigma(b_4, p_5);
    if ((_e10 <= b_4.effect.w)) {
        const float4 _e16 = level(b_4, (n_2 - 1u), p_5);
        const Blend blend_1 = ConstructBlend(_e16, (_e10 / b_4.effect.w));
        return blend_1;
    }
    float octave = log2((b_4.effect.z / _e10));
    uint k_2 = naga_f2u32(min(floor(octave), float((n_2 - 2u))));
    const float4 _e31 = level(b_4, k_2, p_5);
    const float4 _e34 = level(b_4, (k_2 + 1u), p_5);
    const Blend blend_2 = ConstructBlend(lerp(_e31, _e34, clamp((octave - float(k_2)), 0.0, 1.0)), 1.0);
    return blend_2;
}

float4 composite(Blur b_5, float4 p_6)
{
    Slot s_2 = ConstructSlot(asfloat(slots.Load4(b_5.slots.z*48+0)), asfloat(slots.Load4(b_5.slots.z*48+16)), asuint(slots.Load4(b_5.slots.z*48+32)));
    float4 before = original.Load(int3(asint(asuint(naga_f2i32(s_2.atlas.xy)) + asuint(naga_f2i32((p_6.xy - s_2.region.xy)))), int(0)));
    const Blend _e20 = blurred(b_5, p_6.xy);
    const float _e23 = amount(b_5, p_6.xy);
    return lerp(before, _e20.color, (_e23 * _e20.weight));
}

Vertex ConstructVertex(float4 arg0, uint arg1) {
    Vertex ret = (Vertex)0;
    ret.p = arg0;
    ret.index = arg1;
    return ret;
}

VertexOutput_vs vs(uint i : SV_VertexID, uint index : SV_InstanceID)
{
    float2 p_14 = ((float2(float((((_NagaConstants.first_vertex + i) << 1u) & 2u)), float(((_NagaConstants.first_vertex + i) & 2u))) * 2.0) - (1.0).xx);
    const Vertex vertex = ConstructVertex(float4(p_14, 0.0, 1.0), (_NagaConstants.first_instance + index));
    const VertexOutput_vs vertex_1 = { vertex.index, vertex.p };
    return vertex_1;
}

Blur ConstructBlur(float4 arg0, float4 arg1, float4 arg2, float4 arg3, uint4 arg4, float4 arg5, float4 arg6, float4 arg7, float4 arg8) {
    Blur ret = (Blur)0;
    ret.bounds = arg0;
    ret.effect = arg1;
    ret.area = arg2;
    ret.shape = arg3;
    ret.slots = arg4;
    ret.round0_ = arg5;
    ret.radii0_ = arg6;
    ret.round1_ = arg7;
    ret.radii1_ = arg8;
    return ret;
}

float4 fs_reduce(FragmentInput_fs_reduce fragmentinput_fs_reduce) : SV_Target0
{
    Vertex v = { fragmentinput_fs_reduce.p_8, fragmentinput_fs_reduce.index_2 };
    float4 sum = (0.0).xxxx;
    bool local = (bool)0;
    int y = int(0);
    int x = (int)0;
    int y_1 = (int)0;
    int x_1 = (int)0;

    Slot s_3 = ConstructSlot(asfloat(slots.Load4(v.index*48+0)), asfloat(slots.Load4(v.index*48+16)), asuint(slots.Load4(v.index*48+32)));
    Blur b_6 = ConstructBlur(asfloat(blurs.Load4(s_3.owner.x*144+0)), asfloat(blurs.Load4(s_3.owner.x*144+16)), asfloat(blurs.Load4(s_3.owner.x*144+32)), asfloat(blurs.Load4(s_3.owner.x*144+48)), asuint(blurs.Load4(s_3.owner.x*144+64)), asfloat(blurs.Load4(s_3.owner.x*144+80)), asfloat(blurs.Load4(s_3.owner.x*144+96)), asfloat(blurs.Load4(s_3.owner.x*144+112)), asfloat(blurs.Load4(s_3.owner.x*144+128)));
    int f = naga_f2i32(s_3.region.z);
    int2 start_1 = asint(asuint(naga_f2i32(s_3.region.xy)) + asuint(asint(asuint(asint(asuint(naga_f2i32(v.p.xy)) - asuint(naga_f2i32(s_3.atlas.xy)))) * asuint(f))));
    int2 end = min(asint(asuint(start_1) + asuint((f).xx)), naga_f2i32(b_6.area.zw));
    if ((f > int(1))) {
        local = all((asint(asuint(end) - asuint(start_1)) == (f).xx));
    } else {
        local = false;
    }
    bool _e43 = local;
    if (_e43) {
        float2 size_2 = float2(NagaDimensions2D(source));
        switch(f) {
            case 2: {
                float4 _e47 = sum;
                const float4 _e50 = tap(start_1, int(0), int(0), size_2);
                sum = (_e47 + _e50);
                break;
            }
            case 4: {
                float4 _e52 = sum;
                const float4 _e55 = tap(start_1, int(0), int(0), size_2);
                sum = (_e52 + _e55);
                float4 _e57 = sum;
                const float4 _e60 = tap(start_1, int(2), int(0), size_2);
                sum = (_e57 + _e60);
                float4 _e62 = sum;
                const float4 _e65 = tap(start_1, int(0), int(2), size_2);
                sum = (_e62 + _e65);
                float4 _e67 = sum;
                const float4 _e70 = tap(start_1, int(2), int(2), size_2);
                sum = (_e67 + _e70);
                break;
            }
            case 8: {
                float4 _e72 = sum;
                const float4 _e75 = tap(start_1, int(0), int(0), size_2);
                sum = (_e72 + _e75);
                float4 _e77 = sum;
                const float4 _e80 = tap(start_1, int(2), int(0), size_2);
                sum = (_e77 + _e80);
                float4 _e82 = sum;
                const float4 _e85 = tap(start_1, int(4), int(0), size_2);
                sum = (_e82 + _e85);
                float4 _e87 = sum;
                const float4 _e90 = tap(start_1, int(6), int(0), size_2);
                sum = (_e87 + _e90);
                float4 _e92 = sum;
                const float4 _e95 = tap(start_1, int(0), int(2), size_2);
                sum = (_e92 + _e95);
                float4 _e97 = sum;
                const float4 _e100 = tap(start_1, int(2), int(2), size_2);
                sum = (_e97 + _e100);
                float4 _e102 = sum;
                const float4 _e105 = tap(start_1, int(4), int(2), size_2);
                sum = (_e102 + _e105);
                float4 _e107 = sum;
                const float4 _e110 = tap(start_1, int(6), int(2), size_2);
                sum = (_e107 + _e110);
                float4 _e112 = sum;
                const float4 _e115 = tap(start_1, int(0), int(4), size_2);
                sum = (_e112 + _e115);
                float4 _e117 = sum;
                const float4 _e120 = tap(start_1, int(2), int(4), size_2);
                sum = (_e117 + _e120);
                float4 _e122 = sum;
                const float4 _e125 = tap(start_1, int(4), int(4), size_2);
                sum = (_e122 + _e125);
                float4 _e127 = sum;
                const float4 _e130 = tap(start_1, int(6), int(4), size_2);
                sum = (_e127 + _e130);
                float4 _e132 = sum;
                const float4 _e135 = tap(start_1, int(0), int(6), size_2);
                sum = (_e132 + _e135);
                float4 _e137 = sum;
                const float4 _e140 = tap(start_1, int(2), int(6), size_2);
                sum = (_e137 + _e140);
                float4 _e142 = sum;
                const float4 _e145 = tap(start_1, int(4), int(6), size_2);
                sum = (_e142 + _e145);
                float4 _e147 = sum;
                const float4 _e150 = tap(start_1, int(6), int(6), size_2);
                sum = (_e147 + _e150);
                break;
            }
            default: {
                uint2 loop_bound_2 = uint2(4294967295u, 4294967295u);
                bool loop_init_2 = true;
                while(true) {
                    if (all(loop_bound_2 == uint2(0u, 0u))) { break; }
                    loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
                    if (!loop_init_2) {
                        int _e168 = y;
                        y = asint(asuint(_e168) + asuint(int(2)));
                    }
                    loop_init_2 = false;
                    int _e154 = y;
                    if ((_e154 < f)) {
                    } else {
                        break;
                    }
                    {
                        x = int(0);
                        uint2 loop_bound_3 = uint2(4294967295u, 4294967295u);
                        bool loop_init_3 = true;
                        while(true) {
                            if (all(loop_bound_3 == uint2(0u, 0u))) { break; }
                            loop_bound_3 -= uint2(loop_bound_3.y == 0u, 1u);
                            if (!loop_init_3) {
                                int _e165 = x;
                                x = asint(asuint(_e165) + asuint(int(2)));
                            }
                            loop_init_3 = false;
                            int _e158 = x;
                            if ((_e158 < f)) {
                            } else {
                                break;
                            }
                            {
                                float4 _e160 = sum;
                                int _e161 = x;
                                int _e162 = y;
                                const float4 _e163 = tap(start_1, _e161, _e162, size_2);
                                sum = (_e160 + _e163);
                            }
                        }
                    }
                }
                break;
            }
        }
        float4 _e171 = sum;
        return ((_e171 * 4.0) / (float(asint(asuint(f) * asuint(f)))).xxxx);
    }
    y_1 = start_1.y;
    uint2 loop_bound_4 = uint2(4294967295u, 4294967295u);
    bool loop_init_4 = true;
    while(true) {
        if (all(loop_bound_4 == uint2(0u, 0u))) { break; }
        loop_bound_4 -= uint2(loop_bound_4.y == 0u, 1u);
        if (!loop_init_4) {
            int _e200 = y_1;
            y_1 = asint(asuint(_e200) + asuint(int(1)));
        }
        loop_init_4 = false;
        int _e180 = y_1;
        if ((_e180 < end.y)) {
        } else {
            break;
        }
        {
            x_1 = start_1.x;
            uint2 loop_bound_5 = uint2(4294967295u, 4294967295u);
            bool loop_init_5 = true;
            while(true) {
                if (all(loop_bound_5 == uint2(0u, 0u))) { break; }
                loop_bound_5 -= uint2(loop_bound_5.y == 0u, 1u);
                if (!loop_init_5) {
                    int _e197 = x_1;
                    x_1 = asint(asuint(_e197) + asuint(int(1)));
                }
                loop_init_5 = false;
                int _e185 = x_1;
                if ((_e185 < end.x)) {
                } else {
                    break;
                }
                {
                    float4 _e188 = sum;
                    int _e190 = x_1;
                    int _e191 = y_1;
                    float4 _e194 = source.Load(int3(int2(_e190, _e191), int(0)));
                    sum = (_e188 + _e194);
                }
            }
        }
    }
    float4 _e202 = sum;
    return (_e202 / (float(max(asint(asuint(asint(asuint(end.x) - asuint(start_1.x))) * asuint(asint(asuint(end.y) - asuint(start_1.y)))), int(1)))).xxxx);
}

float4 fs_across(FragmentInput_fs_across fragmentinput_fs_across) : SV_Target0
{
    Vertex v_1 = { fragmentinput_fs_across.p_9, fragmentinput_fs_across.index_3 };
    const float4 _e4 = along(v_1, int2(int(1), int(0)));
    return _e4;
}

float4 fs_down(FragmentInput_fs_down fragmentinput_fs_down) : SV_Target0
{
    Vertex v_2 = { fragmentinput_fs_down.p_10, fragmentinput_fs_down.index_4 };
    const float4 _e4 = along(v_2, int2(int(0), int(1)));
    return _e4;
}

float4 fs(FragmentInput_fs fragmentinput_fs) : SV_Target0
{
    Vertex v_3 = { fragmentinput_fs.p_11, fragmentinput_fs.index_5 };
    Blur _e4 = ConstructBlur(asfloat(blurs.Load4(v_3.index*144+0)), asfloat(blurs.Load4(v_3.index*144+16)), asfloat(blurs.Load4(v_3.index*144+32)), asfloat(blurs.Load4(v_3.index*144+48)), asuint(blurs.Load4(v_3.index*144+64)), asfloat(blurs.Load4(v_3.index*144+80)), asfloat(blurs.Load4(v_3.index*144+96)), asfloat(blurs.Load4(v_3.index*144+112)), asfloat(blurs.Load4(v_3.index*144+128)));
    const float4 _e6 = composite(_e4, v_3.p);
    return _e6;
}

Both ConstructBoth(float4 arg0, float4 arg1) {
    Both ret = (Both)0;
    ret.retained = arg0;
    ret.drawable = arg1;
    return ret;
}

Both fs_both(FragmentInput_fs_both fragmentinput_fs_both)
{
    Vertex v_4 = { fragmentinput_fs_both.p_12, fragmentinput_fs_both.index_6 };
    Blur _e4 = ConstructBlur(asfloat(blurs.Load4(v_4.index*144+0)), asfloat(blurs.Load4(v_4.index*144+16)), asfloat(blurs.Load4(v_4.index*144+32)), asfloat(blurs.Load4(v_4.index*144+48)), asuint(blurs.Load4(v_4.index*144+64)), asfloat(blurs.Load4(v_4.index*144+80)), asfloat(blurs.Load4(v_4.index*144+96)), asfloat(blurs.Load4(v_4.index*144+112)), asfloat(blurs.Load4(v_4.index*144+128)));
    const float4 _e6 = composite(_e4, v_4.p);
    const Both both = ConstructBoth(_e6, _e6);
    return both;
}

Mixed ConstructMixed(float4 arg0, float4 arg1) {
    Mixed ret = (Mixed)0;
    ret.color = arg0;
    ret.amount = arg1;
    return ret;
}

Mixed fs_mix(FragmentInput_fs_mix fragmentinput_fs_mix)
{
    Vertex v_5 = { fragmentinput_fs_mix.p_13, fragmentinput_fs_mix.index_7 };
    Blur b_7 = ConstructBlur(asfloat(blurs.Load4(v_5.index*144+0)), asfloat(blurs.Load4(v_5.index*144+16)), asfloat(blurs.Load4(v_5.index*144+32)), asfloat(blurs.Load4(v_5.index*144+48)), asuint(blurs.Load4(v_5.index*144+64)), asfloat(blurs.Load4(v_5.index*144+80)), asfloat(blurs.Load4(v_5.index*144+96)), asfloat(blurs.Load4(v_5.index*144+112)), asfloat(blurs.Load4(v_5.index*144+128)));
    const Blend _e7 = blurred(b_7, v_5.p.xy);
    const float _e10 = amount(b_7, v_5.p.xy);
    float a = (_e10 * _e7.weight);
    const Mixed mixed = ConstructMixed((_e7.color * a), (a).xxxx);
    return mixed;
}
