struct Job {
    float4 rect : LOC0;
    float4 uv : LOC1;
    float4 color : LOC2;
    float4 fade : LOC3;
    float4 options : LOC4;
    float4 shape : LOC5;
    float4 border : LOC6;
};

struct Out {
    float4 position : SV_Position;
    nointerpolation float4 rect : LOC0;
    nointerpolation float4 color : LOC1;
    nointerpolation float4 fade : LOC2;
    nointerpolation float4 options : LOC3;
    nointerpolation float4 shape : LOC4;
    nointerpolation float4 border : LOC5;
};

Texture2D<float4> source : register(t0);
SamplerState linear_ : register(s0);
Texture2D<float4> mask : register(t1);

struct VertexOutput_vs {
    nointerpolation float4 rect : LOC0;
    nointerpolation float4 color_1 : LOC1;
    nointerpolation float4 fade : LOC2;
    nointerpolation float4 options : LOC3;
    nointerpolation float4 shape : LOC4;
    nointerpolation float4 border : LOC5;
    float4 position : SV_Position;
};

struct FragmentInput_fs_across {
    nointerpolation float4 rect_1 : LOC0;
    nointerpolation float4 color_2 : LOC1;
    nointerpolation float4 fade_1 : LOC2;
    nointerpolation float4 options_1 : LOC3;
    nointerpolation float4 shape_1 : LOC4;
    nointerpolation float4 border_1 : LOC5;
    float4 position_1 : SV_Position;
};

struct FragmentInput_fs_down {
    nointerpolation float4 rect_2 : LOC0;
    nointerpolation float4 color_3 : LOC1;
    nointerpolation float4 fade_2 : LOC2;
    nointerpolation float4 options_2 : LOC3;
    nointerpolation float4 shape_2 : LOC4;
    nointerpolation float4 border_2 : LOC5;
    float4 position_2 : SV_Position;
};

bool inside(Out v_2)
{
    bool local_3 = (bool)0;

    if (all((v_2.position.xy >= v_2.fade.xy))) {
        local_3 = all((v_2.position.xy < v_2.fade.zw));
    } else {
        local_3 = false;
    }
    bool _e16 = local_3;
    return _e16;
}

VertexOutput_vs vs(Job job, uint i : SV_VertexID)
{
    Out o = (Out)0;

    float2 corner = float2(float((i & 1u)), float((i >> 1u)));
    float2 p = ((job.rect.xy + (corner * job.rect.zw)) / job.uv.xy);
    o.position = float4(((p.x * 2.0) - 1.0), (1.0 - (p.y * 2.0)), 0.0, 1.0);
    o.rect = job.rect;
    o.color = job.color;
    o.fade = job.fade;
    o.options = job.options;
    o.shape = job.shape;
    o.border = job.border;
    Out _e45 = o;
    const Out out_ = _e45;
    const VertexOutput_vs out_1 = { out_.rect, out_.color, out_.fade, out_.options, out_.shape, out_.border, out_.position };
    return out_1;
}

int naga_neg(int val) {
    return asint(-asuint(val));
}

int naga_div(int lhs, int rhs) {
    return lhs / (((lhs == int(-2147483647 - 1) & rhs == -1) | (rhs == 0)) ? 1 : rhs);
}

int naga_f2i32(float value) {
    return int(clamp(value, -2147483600.0, 2147483500.0));
}

uint2 NagaDimensions2D(Texture2D<float4> tex)
{
    uint4 ret;
    tex.GetDimensions(0, ret.x, ret.y, ret.z);
    return ret.xy;
}

float4 fs_across(FragmentInput_fs_across fragmentinput_fs_across) : SV_Target0
{
    Out v = { fragmentinput_fs_across.position_1, fragmentinput_fs_across.rect_1, fragmentinput_fs_across.color_2, fragmentinput_fs_across.fade_1, fragmentinput_fs_across.options_1, fragmentinput_fs_across.shape_1, fragmentinput_fs_across.border_1 };
    float4 sum = (0.0).xxxx;
    float total = 0.0;
    int k = (int)0;
    float4 block = (float4)0;
    int y = (int)0;
    int x = (int)0;

    const bool _e1 = inside(v);
    if (!(_e1)) {
        return (0.0).xxxx;
    }
    float f = v.color.z;
    float s = max(v.color.w, 0.05);
    int r = naga_f2i32(ceil((3.0 * s)));
    float2 size = float2(NagaDimensions2D(source));
    float2 center_ = (v.color.xy + ((v.position.xy - v.rect.xy) * f));
    int n = max(naga_div(naga_f2i32(f), int(2)), int(1));
    float2 first = ((center_ - ((f * 0.5)).xx) + ((f > 1.5) ? (1.0).xx : (0.5).xx));
    k = naga_neg(r);
    uint2 loop_bound = uint2(4294967295u, 4294967295u);
    bool loop_init = true;
    while(true) {
        if (all(loop_bound == uint2(0u, 0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e110 = k;
            k = asint(asuint(_e110) + asuint(int(1)));
        }
        loop_init = false;
        int _e51 = k;
        if ((_e51 <= r)) {
        } else {
            break;
        }
        {
            int _e53 = k;
            int _e54 = k;
            float w = exp(((-0.5 * float(asint(asuint(_e53) * asuint(_e54)))) / (s * s)));
            block = (0.0).xxxx;
            y = int(0);
            uint2 loop_bound_1 = uint2(4294967295u, 4294967295u);
            bool loop_init_1 = true;
            while(true) {
                if (all(loop_bound_1 == uint2(0u, 0u))) { break; }
                loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
                if (!loop_init_1) {
                    int _e98 = y;
                    y = asint(asuint(_e98) + asuint(int(1)));
                }
                loop_init_1 = false;
                int _e67 = y;
                if ((_e67 < n)) {
                } else {
                    break;
                }
                {
                    x = int(0);
                    uint2 loop_bound_2 = uint2(4294967295u, 4294967295u);
                    bool loop_init_2 = true;
                    while(true) {
                        if (all(loop_bound_2 == uint2(0u, 0u))) { break; }
                        loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
                        if (!loop_init_2) {
                            int _e95 = x;
                            x = asint(asuint(_e95) + asuint(int(1)));
                        }
                        loop_init_2 = false;
                        int _e71 = x;
                        if ((_e71 < n)) {
                        } else {
                            break;
                        }
                        {
                            float4 _e73 = block;
                            int _e76 = k;
                            int _e79 = x;
                            int _e84 = y;
                            float4 _e92 = source.SampleLevel(linear_, ((first + float2(((float(_e76) * f) + float(asint(asuint(int(2)) * asuint(_e79)))), float(asint(asuint(int(2)) * asuint(_e84))))) / size), 0.0);
                            block = (_e73 + _e92);
                        }
                    }
                }
            }
            float4 _e100 = sum;
            float4 _e101 = block;
            sum = (_e100 + (_e101 * (w / float(asint(asuint(n) * asuint(n))))));
            float _e107 = total;
            total = (_e107 + w);
        }
    }
    float4 _e112 = sum;
    float _e113 = total;
    return (_e112 / (_e113).xxxx);
}

int2 naga_f2i32(float2 value) {
    return int2(clamp(value, -2147483600.0, 2147483500.0));
}

int4 naga_f2i32(float4 value) {
    return int4(clamp(value, -2147483600.0, 2147483500.0));
}

float4 fs_down(FragmentInput_fs_down fragmentinput_fs_down) : SV_Target0
{
    Out v_1 = { fragmentinput_fs_down.position_2, fragmentinput_fs_down.rect_2, fragmentinput_fs_down.color_3, fragmentinput_fs_down.fade_2, fragmentinput_fs_down.options_2, fragmentinput_fs_down.shape_2, fragmentinput_fs_down.border_2 };
    float4 sum_1 = (0.0).xxxx;
    float total_1 = 0.0;
    bool local = (bool)0;
    int k_1 = (int)0;
    bool local_1 = (bool)0;
    float4 color = (float4)0;
    bool local_2 = (bool)0;

    const bool _e1 = inside(v_1);
    if (!(_e1)) {
        return (0.0).xxxx;
    }
    float s_1 = v_1.color.w;
    int r_1 = ((s_1 > 0.0) ? naga_f2i32(ceil((3.0 * s_1))) : int(0));
    int2 at = naga_f2i32(floor(((v_1.position.xy - v_1.rect.xy) + v_1.color.xy)));
    int4 range = naga_f2i32(v_1.options);
    if ((at.x >= range.x)) {
        local = (at.x < range.z);
    } else {
        local = false;
    }
    bool _e41 = local;
    if (_e41) {
        k_1 = naga_neg(r_1);
        uint2 loop_bound_3 = uint2(4294967295u, 4294967295u);
        bool loop_init_3 = true;
        while(true) {
            if (all(loop_bound_3 == uint2(0u, 0u))) { break; }
            loop_bound_3 -= uint2(loop_bound_3.y == 0u, 1u);
            if (!loop_init_3) {
                int _e81 = k_1;
                k_1 = asint(asuint(_e81) + asuint(int(1)));
            }
            loop_init_3 = false;
            int _e44 = k_1;
            if ((_e44 <= r_1)) {
            } else {
                break;
            }
            {
                int _e46 = k_1;
                int _e47 = k_1;
                float w_1 = ((r_1 > int(0)) ? exp(((-0.5 * float(asint(asuint(_e46) * asuint(_e47)))) / (s_1 * s_1))) : 1.0);
                int _e60 = k_1;
                int y_1 = asint(asuint(at.y) + asuint(_e60));
                if ((y_1 >= range.y)) {
                    local_1 = (y_1 < range.w);
                } else {
                    local_1 = false;
                }
                bool _e69 = local_1;
                if (_e69) {
                    float4 _e70 = sum_1;
                    float4 _e75 = source.Load(int3(int2(at.x, y_1), int(0)));
                    sum_1 = (_e70 + (_e75 * w_1));
                }
                float _e78 = total_1;
                total_1 = (_e78 + w_1);
            }
        }
    } else {
        total_1 = 1.0;
    }
    float4 _e84 = sum_1;
    float _e85 = total_1;
    color = (_e84 / (_e85).xxxx);
    if ((v_1.border.x > 0.5)) {
        float2 m = floor((v_1.shape.xy + ((v_1.position.xy - v_1.rect.xy) * v_1.shape.zw)));
        float2 dims = float2(NagaDimensions2D(mask));
        if (all((m >= (0.0).xx))) {
            local_2 = all((m < dims));
        } else {
            local_2 = false;
        }
        bool _e117 = local_2;
        if (_e117) {
            float4 _e118 = color;
            float4 _e122 = mask.Load(int3(naga_f2i32(m), int(0)));
            color = (_e118 * _e122.w);
        } else {
            color = (0.0).xxxx;
        }
    }
    if ((v_1.border.y >= 0.0)) {
        float _e134 = color.w;
        float _e137 = color.w;
        color = float4((v_1.border.yzw * _e134), _e137);
    }
    float4 _e139 = color;
    return _e139;
}
