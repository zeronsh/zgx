# gi on Direct3D 12 with DXC: evidence

Windows 11 25H2 (build 26200), an RTX 4090 (driver 32.0.16.1742) and WARP
(d3d10warp.dll 10.0.26100.9711). DXC is `dxcompiler.dll` 1.9.2602.17
(21d28f727) from Windows SDK 10.0.28000, selected with
`WGPU_DX12_COMPILER=dxc` and the DLL on `PATH`. Branch base: aaa/integration
43670793c.

## What fails

A `gi` build whose world has a probe volume, with ray queries requested
(`zgx_gpu::request_ray_queries`), fails three pipelines under DXC, on the
4090 and on WARP alike:

| pipeline | module, entry, target | DXC's error |
| --- | --- | --- |
| `3D probe rays` (compute) | `3D probe rays`, `trace_rays`, `cs_6_8` | `error: llvm::cast<X>() argument of incompatible type!` |
| `3D opaque` (fragment) | `3D mesh variant`, `fragment`, `ps_6_8` | same |
| `3D translucent` (fragment) | `3D mesh variant`, `fragment`, `ps_6_8` (the same source) | same |

`hlsl/` holds the HLSL naga handed DXC for each, with its arguments
(`*.args.txt`) and DXC's whole error text (`*.error.txt`), plus the plain
`3D` fragment shader of the same run, which compiles. The arguments are
wgpu-hal's: `-E <entry> -T <target> -HV 2018 -no-warnings -Ges -Zi -Od`.
`-Zi -Od` are there because zgx-gpu sets wgpu's DEBUG instance flag in
debug builds (`instance_flags(.., !cfg!(debug_assertions))`).

## Only at -Od

`dxc.exe` of the same SDK, on the dumped files:

| file | `-Zi -Od` | `-Od` | `-Zi` | none |
| --- | --- | --- | --- | --- |
| mesh_variant.fragment | FAIL | FAIL | OK | OK |
| probe_rays.trace_rays | FAIL | FAIL | OK | OK |
| plain_3d.fragment | OK | OK | OK | OK |

So a release build (no DEBUG flag, no `-Od`) compiles these; debug builds
of an app that ships `dxcompiler.dll`, and the test suite, don't.

## Which worlds meet it (question a)

`tools/scratch_dxc.rs`, one process per case, `gi` built in
(`--all-features`), DXC, the 4090:

| case | ray queries | probes | pipelines failing |
| --- | --- | --- | --- |
| plain world, not requested | NotRequested | NoVolumes | none |
| probe volume, not requested | NotRequested | NoRayQueries | none |
| plain world, requested | Available | NoVolumes | none |
| probe volume, requested | Available | Traced (fails) | probe rays, opaque, translucent |

An app with `gi` compiled in but no probe volume, or without ray queries,
compiles nothing affected: only the probe-lit variant and the trace shader
carry the pattern below.

## The trigger: not ray queries (question b)

Bisected on the dumped fragment shader (`-Zi -Od`): stubbing `probe_light`
fixes it; removing both `SampleLevel`s doesn't; a constant volume index
(`probe_set.volumes[0]`) fixes it. The minimal repro, `minimal/k_*.hlsl`
(nine lines), fails at `-Od` on ps_6_0, ps_6_5 and ps_6_8, with `-HV 2018`
and `-HV 2021`:

```hlsl
struct Small { float4 a; float4 b; };
cbuffer small_set : register(b2) { Small smalls[4]; uint4 n; }
float4 fragment(float4 p : SV_Position) : SV_Target0 {
    for (uint i = 0u; i < n.x; i++) {
        Small s = smalls[i];
        if (s.b.x > p.x) { return s.a; }
    }
    return (0.0).xxxx;
}
```

A whole struct copied out of a cbuffer array at a dynamic index, inside a
loop that returns from within. The same without the loop, or reading fields
in place (`minimal/f_*.hlsl`), or summing without an early return
(`minimal/a_*.hlsl`), compiles. That is DXC's alone (no naga or wgpu in
it), a candidate for an upstream DXC issue.

The compute shader has the pattern twice (`probe_light` and
`probe_place`). With both copies' index made constant and every ray query
kept, it compiles at `-Zi -Od`
(`minimal/probe_rays_both_struct_copies_neutralized_RayQuery_kept.hlsl`):
naga's `RayQuery` HLSL (`TraceRayInline`, `Proceed`, the committed
intersection) is fine with this DXC.

## A workaround in our WGSL

`tools/wgsl-pointer-workaround.patch`: `let volume = &probe_set.volumes[i];`
in `probes_trace.wgsl` and `shader/probes.wgsl`, so naga reads each field in
place rather than copying the struct. With it, under DXC and debug flags,
all 9 tests of `cargo test -p zgx-3d --all-features --test gi` pass on the
4090 and on WARP (`--test-threads=1`), the probes trace, and the probe-lit
room matches Metal's `gi_room.png` within 1 level (mean 0.06 on the 4090,
0.01 on WARP, 160x160). Not applied on any branch: it is the gi lane's to
take.

## Also

- Under FXC (zgx's default on Windows), ray queries are unavailable
  (`RayQueries::CompilerIsFxc`), so every probe test returns early and
  passes: gi is untested on Windows unless a run selects DXC.

## Reproduce

1. `git apply evidence/gi-dxc-windows/tools/dump-hlsl.patch` (writes what
   DXC is handed, and its errors, to `ZGX_DUMP_HLSL`).
2. Copy `tools/scratch_dxc.rs` to `crates/zgx-3d/tests/`.
3. With `dxcompiler.dll` on `PATH`: `WGPU_DX12_COMPILER=dxc ZGX_DUMP_HLSL=<dir>
   cargo test -p zgx-3d --all-features --test scratch_dxc -- --exact
   case_probes_requested --nocapture`, and the same for the other cases.
4. `dxc.exe -E fragment -T ps_6_8 -HV 2018 -Od minimal/k_small_struct_loop_early_return.hlsl`.
