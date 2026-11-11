// The mesh's fragment stage.

//T // The surface's normal tilted by a normal map's texel (0 to 1 each, half
//T // way flat), its frame from the screen's derivatives of the point and its
//T // texture coordinates (as Khronos's viewer makes it without tangents).
//T fn bumped(ng: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, duvx: vec2<f32>, duvy: vec2<f32>, texel: vec3<f32>) -> vec3<f32> {
//T     let det = duvx.x * duvy.y - duvy.x * duvx.y;
//T     let along = (duvy.y * dpx - duvx.y * dpy) * sign(det);
//T     let t = along - ng * dot(ng, along);
//T     if abs(det) < 1e-12 || dot(t, t) < 1e-20 {
//T         return ng;
//T     }
//T     let tangent = normalize(t);
//T     let m = texel * 2.0 - 1.0;
//T     return normalize(tangent * m.x + cross(ng, tangent) * m.y + ng * m.z);
//T }

@fragment fn fragment(in: Out, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // Its maps (none in an untextured pipeline): its colour (white without
    // one), in linear light; metal and roughness; which way it faces.
    var texel = vec4(1.0);
    var surface = vec4(1.0);
    var bump = vec3(0.5, 0.5, 1.0);
    var glow = vec3(1.0);
    var occlusion = 1.0;
//T     var uvs = array<vec2<f32>, 5>(in.uv, in.uv, in.uv, in.uv, in.uv);
//U     for (var i = 0u; i < 5u; i++) {
//U         uvs[i] = placed(i, in.uv, in.second_uv);
//U     }
//T     texel = textureSample(base_texture, map_sampler, uvs[0]);
//T     surface = textureSample(metal_rough_texture, map_sampler, uvs[1]);
//T     bump = textureSample(normal_texture, map_sampler, uvs[2]).xyz;
//T     glow = textureSample(emission_texture, map_sampler, uvs[3]).rgb;
//T     occlusion = textureSample(occlusion_texture, map_sampler, uvs[4]).r;
//G     let passing = textureSample(transmission_texture, map_sampler, in.uv).r;
//G     let thick = textureSample(thickness_texture, map_sampler, in.uv).g;
//L     let coat_amount = layers.coat.x * textureSample(coat_texture, map_sampler, in.uv).r;
//L     let coat_given = clamp(layers.coat.y * textureSample(coat_roughness_texture, map_sampler, in.uv).g, 0.02, 1.0);
//L     let coat_bump = textureSample(coat_normal_texture, map_sampler, in.uv).xyz;
//L     let coat_duvx = dpdx(in.uv);
//L     let coat_duvy = dpdy(in.uv);
//T     let dpx = dpdx(in.world);
//T     let dpy = dpdy(in.world);
//T     let duvx = dpdx(uvs[2]);
//T     let duvy = dpdy(uvs[2]);
    // How fast the surface's normal turns across the pixel, in uniform
    // control flow: widened into its roughness below (geometric specular
    // anti-aliasing, Tokuyoshi and Kaplanyan), so a highlight on a curve
    // too fine for the pixels blurs instead of flickering.
    // Multisampled pipelines' alone: one drawing a sample a pixel skips it.
    var widening = 0.0;
//A let facing = normalize(in.normal);
//A let turning_x = dpdx(facing);
//A let turning_y = dpdy(facing);
//A widening = min(0.3 * (dot(turning_x, turning_x) + dot(turning_y, turning_y)), 0.18);
    // A custom material's surface (`zgx_surface`): what its shaders make
    // of what the mesh's material and maps give.
//V     var zs: SurfaceIn;
//V     zs.world_position = in.world;
//V     zs.geometric_normal = normalize(in.normal) * select(-1.0, 1.0, front);
//V     zs.normal = zs.geometric_normal;
//V //T     zs.normal = bumped(zs.geometric_normal, dpx, dpy, duvx, duvy, bump);
//V     zs.uv = in.uv;
//V     zs.uv1 = in.uv;
//V //U     zs.uv1 = in.second_uv;
//V     zs.color = in.color;
//V     zs.view = normalize(frame.camera.xyz - in.world);
//V     zs.depth = -(frame.view * vec4(in.world, 1.0)).z;
//V     zs.screen_uv = in.position.xy / frame.size;
//V     zs.front_facing = front;
//V     zs.instance = in.instance;
//D     if zgx_opacity(zs) < zgx_cutoff(in.material.z) {
//D         discard;
//D     }
//V     zgx_so_far = Surface(in.color.rgb * texel.rgb, in.material.x * surface.b, clamp(in.material.y * surface.g, 0.02, 1.0), zs.normal, in.emissive * glow, occlusion, clamp(select(1.0, in.color.a * texel.a, in.color.a < 1.0), 0.0, 1.0));
//V     let zm = zgx_surface(zs, in.material.z <= 0.5);
//V     occlusion = zm.occlusion;
//K     if texel.a < cutoff_of(in.material.z) {
//K         discard;
//K     }
//O     var base = in.color.rgb * texel.rgb;
//V     var base = zm.base_color;
    var lit: vec3<f32>;
    if in.material.z > 0.5 {
        lit = base;
    } else {
        var n = normalize(in.normal);
        if !front {
            n = -n;
        }
        // A coat faces as its own normal map says, not as the surface's
        // under it does.
//L         let coat_n = bumped(n, dpx, dpy, coat_duvx, coat_duvy, coat_bump);
//T         n = bumped(n, dpx, dpy, duvx, duvy, bump);
//V         n = zm.normal;
        let v = normalize(frame.camera.xyz - in.world);
//O         let metallic = in.material.x * surface.b;
//O         let given = clamp(in.material.y * surface.g, 0.02, 1.0);
//V         let metallic = zm.metallic;
//V         let given = clamp(zm.roughness, 0.02, 1.0);
        let a = given * given;
        let roughness = sqrt(sqrt(clamp(a * a + widening, 0.0, 1.0)));
//L         let coat_a = coat_given * coat_given;
//L         let coat_rough = sqrt(sqrt(clamp(coat_a * coat_a + widening, 0.0, 1.0)));
        // What passes through takes the place of its diffuse light.
//G         let see_through = layers.a.x * passing * (1.0 - metallic);
//G         let tinted = base;
//G         base *= 1.0 - see_through;
        var sun = frame.sun_color.rgb;
        if frame.params.w > 0.5 {
//I             sun *= sunlit(in.world, n);
//H             sun *= sunlit_cascaded(in.world, n);
        }
//Q         lit = shade(base, metallic, roughness, n, v, -frame.sun_direction.xyz, sun);
//R         lit = shade_specular(base, metallic, roughness, n, v, -frame.sun_direction.xyz, sun);
        // A coat's own highlights: glass's (a black dielectric's alone).
//L         var coat_lit = shade(vec3(0.0), 0.0, coat_rough, coat_n, v, -frame.sun_direction.xyz, sun);
//Y         for (var i = 0u; i < u32(frame.params.x); i++) {
//Y             let light = frame.points[i];
//Y             let to = light[0].xyz - in.world;
//Y             let distance = max(length(to), 1e-4);
//Y             let falloff = pow(clamp(1.0 - pow(distance / light[0].w, 4.0), 0.0, 1.0), 2.0) / max(distance * distance, 1e-4);
            // Within a spot's cone (all round: 1).
//Y             let cone = clamp(dot(light[2].xyz, -to / distance) + light[2].w, 0.0, 1.0);
//Y //Q             lit += shade(base, metallic, roughness, n, v, to / distance, light[1].rgb * light[1].w * falloff * cone * cone);
//Y //R             lit += shade_specular(base, metallic, roughness, n, v, to / distance, light[1].rgb * light[1].w * falloff * cone * cone);
//Y //L             coat_lit += shade(vec3(0.0), 0.0, coat_rough, coat_n, v, to / distance, light[1].rgb * light[1].w * falloff * cone * cone);
//Y         }
        // Clustered: the fragment's cluster's lights (`lights.wgsl`).
//X         let span = cluster_span(in.position.xy, in.world);
//X         for (var i = span.x; i < span.y; i++) {
//X             let lamp = lamp_at(cluster_light(i), in.world);
            // A light listed but not reaching costs no shading.
//X             if lamp.reach > 0.0 {
//X //Q                 lit += shade(base, metallic, widened(roughness, lamp.widen), n, v, lamp.l, lamp.radiance);
//X //R                 lit += shade_specular(base, metallic, widened(roughness, lamp.widen), n, v, lamp.l, lamp.radiance);
//X //L                 coat_lit += shade(vec3(0.0), 0.0, widened(coat_rough, lamp.widen), coat_n, v, lamp.l, lamp.radiance);
//X             }
//X         }
        // Ambient light: the hemisphere seen by the surface for its diffuse
        // part, and along its reflection, by Fresnel, for its specular one
        // (what makes metals their colour away from the lights).
        let ambient = mix(frame.ground.rgb, frame.sky.rgb, n.y * 0.5 + 0.5);
        let r = reflect(-v, n);
        let reflected = mix(frame.ground.rgb, frame.sky.rgb, r.y * 0.5 + 0.5);
//Q         let f0 = mix(vec3(0.04), base, metallic);
//R         let f0 = mix(layers.spec.w * min(layers.spec.rgb, vec3(1.0)), base, metallic);
        let n_v = max(dot(n, v), 0.0);
        let rough = roughness * roughness;
//Q         let fresnel = f0 + (max(vec3(1.0 - rough), f0) - f0) * pow(1.0 - n_v, 5.0);
//R         let fresnel = f0 + (max(vec3(1.0 - rough) * mix(layers.spec.w, 1.0, metallic), f0) - f0) * pow(1.0 - n_v, 5.0);
        var surround = ambient * base * (1.0 - metallic) * (1.0 - fresnel);
        surround += reflected * fresnel * (1.0 - 0.5 * rough);
        // Lit by an environment instead: its diffuse light, and its light
        // along the reflection blurred as rough as the surface (the cube's
        // level) times the split sum's factor.
//E     let along = textureSampleLevel(environment_cube, environment_sampler, turned(r), roughness * frame.environment.w).rgb;
//E     surround = irradiance(turned(n)) * base * (1.0 - metallic) * (1.0 - fresnel);
//E     surround = (surround + along * reflectance(f0, roughness, n_v)) * frame.environment.x;
        // Inside a probe volume, its probes' diffuse light takes the place
        // of the hemisphere's or environment's (`probes.wgsl`).
//probed         let probed = probe_light(in.world, n, v);
//probed_unlit         let around = ambient;
//probed //E     let around = irradiance(turned(n)) * frame.environment.x;
//probed         surround += (probed.rgb - around) * probed.a * base * (1.0 - metallic) * (1.0 - fresnel);
        lit += surround * occlusion;
//O         lit += in.emissive * glow;
//V         lit += zm.emissive;
        // A coat reflects its surroundings as glass, and what is under it
        // (its light included) shows the less the more it reflects.
//L         let coat_r = reflect(-v, coat_n);
//L         let coat_n_v = max(dot(coat_n, v), 0.0);
//L         let coat_rough2 = coat_rough * coat_rough;
//L         let coat_fresnel = 0.04 + (max(1.0 - coat_rough2, 0.04) - 0.04) * pow(1.0 - coat_n_v, 5.0);
//L         var coat_around = mix(frame.ground.rgb, frame.sky.rgb, coat_r.y * 0.5 + 0.5) * coat_fresnel * (1.0 - 0.5 * coat_rough2);
//E //L     coat_around = textureSampleLevel(environment_cube, environment_sampler, turned(coat_r), coat_rough * frame.environment.w).rgb * reflectance(vec3(0.04), coat_rough, coat_n_v) * frame.environment.x;
//W         var under = 1.0;
//L         under = 1.0 - coat_amount * (0.04 + 0.96 * pow(1.0 - coat_n_v, 5.0));
//L         lit = lit * under + coat_amount * (coat_lit + coat_around * occlusion);
        lit = tone_map(lit * frame.params.z);
        // What shows through, behind already mapped, less what reflects.
//G         let facing_f0 = pow((layers.a.y - 1.0) / (layers.a.y + 1.0), 2.0);
//G         let reflects = facing_f0 + (1.0 - facing_f0) * pow(1.0 - n_v, 5.0);
//G         lit += through(in.world, n, v, roughness, in.scale * thick) * tinted * see_through * (1.0 - reflects) * under;
    }
    if frame.params.y > 0.5 {
        lit = encode(clamp(lit, vec3(0.0), vec3(1.0)));
    }
    // A translucent mesh's texture adds its own alpha; an opaque one's
    // is ignored (glTF's OPAQUE).
//O     let alpha = clamp(select(1.0, in.color.a * texel.a, in.color.a < 1.0), 0.0, 1.0);
//V     let alpha = clamp(zm.opacity, 0.0, 1.0);
    return vec4(lit * alpha, alpha);
}
