struct ProbeVolume { float4 origin; float4 spacing; float4 inverse; uint4 counts; float4 lo; float4 hi; float4 fade; };
struct ProbeSet { ProbeVolume volumes[4]; uint4 count; float4 texel; };
cbuffer probe_set : register(b1) { ProbeSet probe_set; }float4 fragment(float4 p : SV_Position) : SV_Target0 {
    for (uint i = 0u; i < probe_set.count.x; i++) {
        ProbeVolume volume = probe_set.volumes[i];
        if (volume.fade.x > p.x) { return volume.origin; }
    }
    return (0.0).xxxx;
}
