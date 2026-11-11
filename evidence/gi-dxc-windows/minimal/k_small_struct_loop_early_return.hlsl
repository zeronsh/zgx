struct Small { float4 a; float4 b; };
cbuffer small_set : register(b2) { Small smalls[4]; uint4 n; }
float4 fragment(float4 p : SV_Position) : SV_Target0 {
    for (uint i = 0u; i < n.x; i++) {
        Small s = smalls[i];
        if (s.b.x > p.x) { return s.a; }
    }
    return (0.0).xxxx;
}
