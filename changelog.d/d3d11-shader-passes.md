---
section: fixed
---
- **Direct3D 11 runs layer shaders' passes** (zgx-gpu, `ZGX_D3D11`): a layer shader reading passes or tagged boxes (`FillShader::with_inputs`, a script's `ui.layer_shader` with passes) draws through them as on Direct3D 12, its passes drawn with Direct3D 11 ahead of the frame (feedback, steps and once-only passes alike), where its layer used to composite as painted. One reading pictures or GPU views still composites as painted, named once on stderr.
