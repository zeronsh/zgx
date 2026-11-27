// A box's corners as `CALayer`'s `cornerCurve = continuous` draws them, read from
// QuartzCore's default.metallib: `UberShader::supercircle_image`, with
// `fill_round_rect`'s and `stroke_round_rect`'s arguments. `continuous_source` puts
// `corner_edge` in draw.wgsl's `shade_texel`, where `shape.w` is above 1.5. Its
// fwidth becomes this gradient: the same at every pixel, however a box has moved.

// `a`: how far into the corner, 1 at a side and 0 at 1.52866 radii from it, each way.
// `t`: each way's progress to the circular corner. The field, 0 on the curve, and
// its gradient in `a`.
fn supercircle(a:vec2<f32>, t:vec2<f32>)->vec3<f32> {
 let high=max(a.x,a.y); let low=min(a.x,a.y);
 let s=clamp(low/max(high,1e-20),0.,1.);
 let p=(((3.15600991-0.926054001*s)*s-3.64122009)*s+1.26803005)*s+0.268530995;
 let dp=((3.*3.15600991-4.*0.926054001*s)*s-2.*3.64122009)*s+1.26803005;
 let bulge=1./(1.-s*s*p);
 // Along s: bulge's slope, across and down by which way is the larger.
 let along=-bulge*bulge*s*(2.*p+s*dp);
 let ds=select(vec2(-low/max(high*high,1e-20),1./max(high,1e-20)),vec2(1./max(high,1e-20),-low/max(high*high,1e-20)),a.y>a.x);
 let reach=length(a);
 let round=a/max(reach,1e-20)+along*ds;
 let v=max(a*1.52866495-vec2(0.528665006),vec2(0.));
 let vs=length(v);
 let circle=v/max(vs,1e-20);
 let k=select(t.y,t.x,a.y>a.x);
 return vec3(mix(reach+1.-bulge,vs*0.654165566+0.345834434,k)-1.,mix(round,circle,k));
}
// `at`: a pixel's distance from the box's near sides, across and down. Its distance
// outside the curve (negative inside), and the normal there with both parts positive,
// in logical units, so that `shade`'s ramp over the normal's parts is the field's over
// its gradient's.
fn corner_edge(at:vec2<f32>, radius:f32, size:vec2<f32>)->vec3<f32> {
 let reach=radius*1.52866495;
 // Past the corner: the straight sides.
 if at.x>=reach||at.y>=reach { return select(vec3(-at.y,0.,1.),vec3(-at.x,1.,0.),at.x<at.y); }
 let t=clamp(vec2(2.89155704)-1.44577852*size/reach,vec2(0.),vec2(1.));
 let field=supercircle(max((vec2(reach)-at)/reach,vec2(0.)),t);
 let slope=length(field.yz);
 if slope<1e-5 { return vec3(field.x*reach,1.,0.); }
 // The gradient grows without bound towards a=0, where only the angle changes: up to
 // the most it reaches near the curve (1.07), a distance; a shadow reads it further in.
 return vec3(field.x*reach/min(slope,1.5),abs(field.yz)/slope);
}
