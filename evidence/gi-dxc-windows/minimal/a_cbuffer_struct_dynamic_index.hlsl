struct ProbeVolume { float4 origin; float4 spacing; float4 inverse; uint4 counts; float4 lo; float4 hi; float4 fade; };
struct ProbeSet { ProbeVolume volumes[4]; uint4 count; float4 texel; };
cbuffer probe_set : register(b1) { ProbeSet probe_set; }float4 fragment(float4 pos : SV_Position) : SV_Target0 {
    float4 sum = 0;
    for (uint i = 0; i < probe_set.count.x; i++) {
        ProbeVolume volume = probe_set.volumes[i];
        sum += volume.origin + volume.fade;
    }
    return sum;
}
