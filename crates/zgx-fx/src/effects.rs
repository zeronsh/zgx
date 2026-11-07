//! Each effect's parameters, presets and how they reach its shader.
use super::{Fx, FxField, FxKind, FxValue, Shots};
use zgx::scene::Color;
use zgx::style::rgb;

const fn gray(value: u8) -> Color {
    Color(value, value, value, 255)
}
macro_rules! source {
    (layer, $file:literal) => {
        concat!(include_str!("common.wgsl"), "\n", include_str!($file))
    };
    (fill, $file:literal) => {
        concat!(
            include_str!("common.wgsl"),
            "\n",
            include_str!("fills.wgsl"),
            "\n",
            include_str!($file)
        )
    };
}
macro_rules! kind {
    (layer) => {
        FxKind::Layer
    };
    (fill) => {
        FxKind::Fill
    };
}
/// A choice of an effect's, by name in Luau.
macro_rules! choice {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident = $text:literal $(= $default:tt)?),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub enum $name {
            $($(#[$vmeta])* $(#[$default])? $variant),*
        }
        impl FxField for $name {
            fn write(&self, lanes: &mut [f32]) {
                lanes[0] = *self as u8 as f32;
            }
            fn from_value(value: FxValue<'_>) -> Option<Self> {
                match value {
                    $(FxValue::Text($text) => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }
    };
}
/// An effect: its parameters and where each goes (row and lane; row 9, not in
/// its uniforms), its presets, and what follows from them. `seed` and
/// `intensity` come with each.
macro_rules! effect {
    (
        $(#[$meta:meta])*
        $name:ident: $kind:ident $luau:literal $file:literal {
            $($(#[$fmeta:meta])* $field:ident: $ty:ty = $default:expr => $row:literal $lane:literal,)*
        }
        $(presets {
            $($(#[$pmeta:meta])* $preset:ident { $($pfield:ident: $pvalue:expr),* $(,)? })*
        })?
        fps: $fps:expr,
        bleed: $bleed:expr,
        reach: $reach:expr,
        frost: $frost:expr,
        animated: $animated:expr,
        idle: $idle:expr,
        life: $life:expr,
        one_shot: $one_shot:literal,
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name {
            $($(#[$fmeta])* pub $field: $ty,)*
            /// Which of the frames it could show: the same seed and time, the same pixels.
            pub seed: u32,
            /// How strong it is, 0 to 1 or past it. At 0 it isn't drawn.
            pub intensity: f32,
        }
        impl Default for $name {
            fn default() -> Self {
                Self {
                    $($field: $default,)*
                    seed: 0,
                    intensity: 1.,
                }
            }
        }
        impl $name {
            $($($(#[$pmeta])* pub fn $preset() -> Self {
                Self {
                    $($pfield: $pvalue,)*
                    ..Self::default()
                }
            })*)?
            /// Its name, as the Luau module has it.
            pub const NAME: &'static str = $luau;
            /// A preset by name, as the Luau module names it.
            pub fn preset(name: &str) -> Option<Self> {
                match name {
                    $($(stringify!($preset) => Some(Self::$preset()),)*)?
                    _ => None,
                }
            }
            /// The names of its presets.
            pub const PRESETS: &'static [&'static str] = &[$($(stringify!($preset)),*)?];
            /// Set a parameter by name: whether it has one of that name and type.
            pub fn set(&mut self, name: &str, value: FxValue<'_>) -> bool {
                match name {
                    "seed" => {
                        return f32::from_value(value).map(|seed| self.seed = seed.max(0.) as u32).is_some();
                    }
                    "intensity" => {
                        return f32::from_value(value).map(|intensity| self.intensity = intensity).is_some();
                    }
                    $(stringify!($field) => {
                        return <$ty>::from_value(value).map(|new| self.$field = new).is_some();
                    })*
                    _ => {}
                }
                false
            }
            /// The names of its parameters.
            pub const FIELDS: &'static [&'static str] = &[$(stringify!($field),)* "seed", "intensity"];
            const SOURCE: &'static str = source!($kind, $file);
        }
        impl From<$name> for Fx {
            fn from(e: $name) -> Fx {
                let mut params = [[0f32; 4]; 4];
                $(
                    if $row < 4 {
                        <$ty as FxField>::write(&e.$field, &mut params[$row & 3][$lane..]);
                    }
                )*
                params[3][0] = (e.seed & 0xff_ffff) as f32;
                params[3][1] = e.intensity;
                let frost: f32 = $frost(&e);
                let (bleed, reach, animated, idle, life): (f32, f32, bool, bool, f32) = (
                    $bleed(&e),
                    $reach(&e),
                    $animated(&e),
                    e.intensity <= 0. || $idle(&e),
                    $life(&e),
                );
                Fx {
                    name: $luau,
                    kind: kind!($kind),
                    source: $name::SOURCE,
                    fps: $fps,
                    params,
                    bleed: bleed.max(0.),
                    reach: reach.max(0.),
                    frost: frost.max(0.),
                    animated,
                    idle,
                    shots: $one_shot.then(Shots::default),
                    life,
                }
            }
        }
    };
}
choice!(
    /// How a glow's light looks.
    GlowStyle {
        /// Soft light.
        Soft = "soft" = default,
        /// A white-hot hairline, bloom both sides.
        Neon = "neon",
        /// Three colours flowing round the box.
        Aura = "aura",
    }
);
effect!(
    /// Light around the box, following its corners: `Glow::neon()`.
    Glow: layer "glow" "glow.wgsl" {
        /// Its colour.
        color: Color = rgb(0x5ea5ff) => 0 0,
        /// How far it spreads, in points; it's gone by two and a half of these.
        radius: f32 = 14. => 0 3,
        /// An aura's second colour.
        second: Color = rgb(0xa99cff) => 1 0,
        /// Which light.
        style: GlowStyle = GlowStyle::Soft => 1 3,
        /// An aura's third colour.
        third: Color = rgb(0xff7ac6) => 2 0,
        /// How much it lights the box's inside, 0 to 1.
        inner: f32 = 0. => 2 3,
        /// How much it breathes, 0 (still) to 1.
        breathe: f32 = 0. => 3 3,
    }
    presets {
        /// Soft light.
        soft {}
        /// A neon tube's.
        neon { style: GlowStyle::Neon, color: rgb(0x22d3ee), radius: 12. }
        /// Three colours flowing round the box.
        aura { style: GlowStyle::Aura, radius: 18., inner: 0.12 }
    }
    fps: None,
    bleed: |e: &Glow| e.radius * 2.5,
    reach: |_: &Glow| 0.,
    frost: |_: &Glow| 0.,
    animated: |e: &Glow| e.breathe > 0. || e.style == GlowStyle::Aura,
    idle: |_: &Glow| false,
    life: |_: &Glow| 0.,
    one_shot: false,
);
choice!(
    /// How a pulse goes.
    PulseStyle {
        /// Rings travelling out from the edge.
        Radar = "radar" = default,
        /// A halo growing and easing back.
        Breathe = "breathe",
        /// Two swells a period.
        Heartbeat = "heartbeat",
    }
);
effect!(
    /// Rings or a halo from the box's edge, over and over.
    Pulse: layer "pulse" "pulse.wgsl" {
        /// Its colour.
        color: Color = rgb(0x5ea5ff) => 0 0,
        /// How far it reaches, in points.
        reach: f32 = 36. => 0 3,
        /// Rings at once (radar), up to 6.
        count: f32 = 3. => 1 0,
        /// Seconds a period.
        period: f32 = 2.4 => 1 1,
        /// A ring's width, in points.
        width: f32 = 1.6 => 1 2,
        /// How it goes.
        style: PulseStyle = PulseStyle::Radar => 1 3,
    }
    presets {
        /// Rings travelling out.
        radar {}
        /// A halo growing and easing back.
        breathe { style: PulseStyle::Breathe, period: 3.2 }
        /// A heart's beat.
        heartbeat { style: PulseStyle::Heartbeat, period: 1.2, color: rgb(0xff5d73) }
    }
    fps: None,
    bleed: |e: &Pulse| e.reach,
    reach: |_: &Pulse| 0.,
    frost: |_: &Pulse| 0.,
    animated: |_: &Pulse| true,
    idle: |_: &Pulse| false,
    life: |_: &Pulse| 0.,
    one_shot: false,
);
effect!(
    /// Rings from where it's fired, within the box: `Fx::fire(x, y)`.
    Ripple: layer "ripple" "ripple.wgsl" {
        /// Its colour.
        color: Color = gray(255) => 0 0,
        /// How far the rings travel, in points.
        reach: f32 = 180. => 0 3,
        /// Seconds a ripple lives.
        life: f32 = 0.75 => 9 0,
    }
    fps: None,
    bleed: |_: &Ripple| 0.,
    reach: |_: &Ripple| 0.,
    frost: |_: &Ripple| 0.,
    animated: |_: &Ripple| true,
    idle: |_: &Ripple| false,
    life: |e: &Ripple| e.life,
    one_shot: true,
);
effect!(
    /// A bright band crossing the content: a loading skeleton, "Thinking…".
    Shimmer: layer "shimmer" "shimmer.wgsl" {
        /// Its colour.
        color: Color = gray(255) => 0 0,
        /// The band's width, in points.
        band: f32 = 28. => 0 3,
        /// Seconds a sweep, its pause in them.
        period: f32 = 1.8 => 1 0,
        /// The way it travels, in radians.
        angle: f32 = 0.35 => 1 1,
        /// The share of its period it rests, 0 to 1.
        pause: f32 = 0.3 => 1 2,
        /// Over the whole box rather than what the content paints.
        on_box: bool = false => 1 3,
    }
    presets {
        /// A sheen on what's painted.
        sheen {}
        /// A skeleton's: over the box.
        skeleton { on_box: true, intensity: 0.5, pause: 0.1 }
    }
    fps: None,
    bleed: |_: &Shimmer| 0.,
    reach: |_: &Shimmer| 0.,
    frost: |_: &Shimmer| 0.,
    animated: |_: &Shimmer| true,
    idle: |_: &Shimmer| false,
    life: |_: &Shimmer| 0.,
    one_shot: false,
);
effect!(
    /// Four-point stars twinkling in and past the box.
    Sparkles: layer "sparkles" "sparkles.wgsl" {
        /// Their colour.
        color: Color = rgb(0xffe08a) => 0 0,
        /// A star's size, in points.
        size: f32 = 11. => 0 3,
        /// Stars at once, up to 12.
        count: f32 = 6. => 1 0,
        /// Seconds of each.
        life: f32 = 1.5 => 1 1,
        /// How far past the box they land, in points.
        margin: f32 = 10. => 1 2,
    }
    fps: None,
    bleed: |e: &Sparkles| e.margin + e.size * 2.,
    reach: |_: &Sparkles| 0.,
    frost: |_: &Sparkles| 0.,
    animated: |_: &Sparkles| true,
    idle: |e: &Sparkles| e.count <= 0.,
    life: |_: &Sparkles| 0.,
    one_shot: false,
);
effect!(
    /// Curtains of light across the box, at most 60 steps a second.
    Aurora: layer "aurora" "aurora.wgsl" {
        /// A curtain's colour.
        color: Color = rgb(0x4ff7c5) => 0 0,
        /// Its size, in points.
        scale: f32 = 120. => 0 3,
        /// The second curtain's colour.
        second: Color = rgb(0x7a7bff) => 1 0,
        /// How fast they drift.
        speed: f32 = 0.25 => 1 3,
        /// The third curtain's colour.
        third: Color = rgb(0xdc7bff) => 2 0,
        /// How far in from the box's edge they fade, in points.
        feather: f32 = 24. => 2 3,
    }
    fps: Some(60.),
    bleed: |_: &Aurora| 0.,
    reach: |_: &Aurora| 0.,
    frost: |_: &Aurora| 0.,
    animated: |_: &Aurora| true,
    idle: |_: &Aurora| false,
    life: |_: &Aurora| 0.,
    one_shot: false,
);
effect!(
    /// Fine noise over the content, still unless given a rate.
    Grain: layer "grain" "grain.wgsl" {
        /// How strong, 0 to 1.
        amount: f32 = 0.05 => 0 0,
        /// One noise for all channels, not one each.
        mono: bool = true => 0 1,
        /// New noise this many times a second; 0, still.
        fps: f32 = 0. => 0 2,
    }
    fps: Some(24.),
    bleed: |_: &Grain| 0.,
    reach: |_: &Grain| 0.,
    frost: |_: &Grain| 0.,
    animated: |e: &Grain| e.fps > 0.,
    idle: |e: &Grain| e.amount <= 0.,
    life: |_: &Grain| 0.,
    one_shot: false,
);
effect!(
    /// Lightning crawling along the box's edge.
    Electric: layer "electric" "electric.wgsl" {
        /// Its colour.
        color: Color = rgb(0x8fd0ff) => 0 0,
        /// How far it strays from the edge, in points.
        amplitude: f32 = 5. => 0 3,
        /// Its jaggedness per point.
        frequency: f32 = 0.05 => 1 0,
        /// Strikes a second.
        speed: f32 = 3.2 => 1 1,
        /// The hot core's width, in points.
        core: f32 = 0.7 => 1 2,
        /// The glow's width, in points.
        glow: f32 = 5. => 1 3,
        /// Strands at once, up to 3.
        strands: f32 = 2. => 2 0,
        /// How much each dies between strikes, 0 to 1.
        flicker: f32 = 0.45 => 2 1,
    }
    presets {
        /// A storm round the box.
        storm {}
        /// A calm crawling hairline.
        charged { amplitude: 2., speed: 1.2, flicker: 0.15, strands: 1., glow: 3. }
    }
    fps: None,
    bleed: |e: &Electric| e.amplitude * 1.2 + e.glow * 4.,
    reach: |_: &Electric| 0.,
    frost: |_: &Electric| 0.,
    animated: |_: &Electric| true,
    idle: |_: &Electric| false,
    life: |_: &Electric| 0.,
    one_shot: false,
);
effect!(
    /// Flames rising from the box's top edge, with embers. Noise, not a
    /// simulated fluid: a frame's cost is its pixels.
    Fire: layer "fire" "fire.wgsl" {
        /// The colour of its coolest tips.
        color: Color = rgb(0xc92a0e) => 0 0,
        /// How tall, in points.
        height: f32 = 54. => 0 3,
        /// Its tongues' size, in points.
        scale: f32 = 20. => 1 0,
        /// How fast it rises.
        speed: f32 = 1.5 => 1 1,
        /// Embers at once, up to 12.
        embers: f32 = 8. => 1 2,
        /// How much its tongues bend, 0 to 2.
        warp: f32 = 1. => 1 3,
    }
    presets {
        /// A fire.
        flames {}
        /// A blue gas flame.
        plasma { color: rgb(0x2b5cff), height: 26., scale: 12., embers: 0. }
    }
    fps: None,
    bleed: |e: &Fire| e.height * 1.4,
    reach: |_: &Fire| 0.,
    frost: |_: &Fire| 0.,
    animated: |_: &Fire| true,
    idle: |_: &Fire| false,
    life: |_: &Fire| 0.,
    one_shot: false,
);
effect!(
    /// The content smeared along its velocity. Still at rest (no shader,
    /// nothing past a plain paint): give it the velocity as it moves.
    MotionBlur: layer "motion_blur" "motion.wgsl" {
        /// Across, in points over the shutter.
        dx: f32 = 0. => 0 0,
        /// Down, in points over the shutter.
        dy: f32 = 0. => 0 1,
        /// Taps along it, up to 32.
        samples: f32 = 16. => 0 2,
    }
    fps: None,
    bleed: |e: &MotionBlur| e.dx.abs().max(e.dy.abs()) * 0.5 + 1.,
    reach: |_: &MotionBlur| 0.,
    frost: |_: &MotionBlur| 0.,
    animated: |_: &MotionBlur| false,
    idle: |e: &MotionBlur| e.dx == 0. && e.dy == 0.,
    life: |_: &MotionBlur| 0.,
    one_shot: false,
);
effect!(
    /// What's behind the box wavers, rising, as over a hot road.
    HeatHaze: layer "heat_haze" "haze.wgsl" {
        /// How far it bends, in points.
        amount: f32 = 7. => 0 0,
        /// The waves' size, in points.
        scale: f32 = 42. => 0 1,
        /// How fast they rise.
        speed: f32 = 1.2 => 0 2,
    }
    fps: None,
    bleed: |_: &HeatHaze| 0.,
    reach: |e: &HeatHaze| e.amount.abs() * 1.5,
    frost: |_: &HeatHaze| 0.,
    animated: |_: &HeatHaze| true,
    idle: |e: &HeatHaze| e.amount == 0.,
    life: |_: &HeatHaze| 0.,
    one_shot: false,
);
effect!(
    /// A ring bending what's behind the box, travelling from where it's
    /// fired: `Fx::fire(x, y)`.
    Shockwave: layer "shockwave" "shockwave.wgsl" {
        /// How far it bends, in points.
        amount: f32 = 18. => 0 0,
        /// How far it travels, in points.
        reach: f32 = 260. => 0 1,
        /// The ring's width, in points.
        width: f32 = 28. => 0 2,
        /// Seconds it lives.
        life: f32 = 0.9 => 9 0,
    }
    fps: None,
    bleed: |_: &Shockwave| 0.,
    reach: |e: &Shockwave| e.amount.abs() * 1.5,
    frost: |_: &Shockwave| 0.,
    animated: |_: &Shockwave| true,
    idle: |_: &Shockwave| false,
    life: |e: &Shockwave| e.life,
    one_shot: true,
);
effect!(
    /// A pane of glass over what's behind the box: bent at its rim, its colours
    /// split there, frosted if asked, with a highlight and a shadow. Intensity
    /// is its presence: it appears by bending, not fading.
    Lens: layer "lens" "lens.wgsl" {
        /// How wide its rim is, in points.
        bevel: f32 = 14. => 0 0,
        /// How far its rim bends, in points.
        depth: f32 = 16. => 0 1,
        /// How much its colours split at the rim, 0 to 1.
        dispersion: f32 = 0.14 => 0 2,
        /// How much it enlarges, 1 not at all.
        magnification: f32 = 1.04 => 0 3,
        /// Its frost: the Gaussian's sigma of what's behind it (as
        /// `backdrop_blur`), in points; 0, clear.
        frost: f32 = 0. => 1 0,
        /// How much it saturates what's behind it, 1 not at all.
        saturation: f32 = 1. => 1 1,
        /// How much it lifts what's dark, 0 to 1.
        lift: f32 = 0. => 1 2,
        /// How much of `tint` it takes on, 0 to 1.
        tint_amount: f32 = 0. => 1 3,
        /// A colour it takes on.
        tint: Color = gray(255) => 2 0,
        /// Its highlight's strength, 0 to 1.
        rim: f32 = 0.7 => 2 3,
        /// Its shadow's strength, 0 to 1.
        shadow: f32 = 0.3 => 3 3,
    }
    presets {
        /// Clear glass.
        clear {}
        /// Frosted glass.
        frosted { frost: 16., saturation: 1.5, lift: 0.05, tint_amount: 0.12, depth: 8. }
    }
    fps: None,
    bleed: |e: &Lens| if e.shadow > 0. { 28. } else { 0. },
    reach: |e: &Lens| e.depth.abs() * 1.2,
    frost: |e: &Lens| e.frost,
    animated: |_: &Lens| false,
    idle: |_: &Lens| false,
    life: |_: &Lens| 0.,
    one_shot: false,
);
effect!(
    /// A cathode-ray tube's look over what's behind the box.
    Crt: layer "crt" "crt.wgsl" {
        /// How much the glass bulges, 0 to 1.
        curve: f32 = 0.08 => 0 0,
        /// How strong its scan lines are, 0 to 1.
        scanlines: f32 = 0.5 => 0 1,
        /// How strong its phosphor mask is, 0 to 1.
        mask: f32 = 0.3 => 0 2,
        /// How far its colours split, in points.
        split: f32 = 0.8 => 0 3,
    }
    fps: None,
    bleed: |_: &Crt| 0.,
    reach: |e: &Crt| e.split.abs() + 2.,
    frost: |_: &Crt| 0.,
    animated: |_: &Crt| false,
    idle: |_: &Crt| false,
    life: |_: &Crt| 0.,
    one_shot: false,
);
effect!(
    /// What's behind the box with its channels pulled apart.
    ChromaSplit: layer "chroma_split" "chroma.wgsl" {
        /// Across, in points.
        dx: f32 = 3. => 0 0,
        /// Down, in points.
        dy: f32 = 0. => 0 1,
        /// Outward from the middle, in points at the corner.
        radial: f32 = 0. => 0 2,
    }
    fps: None,
    bleed: |_: &ChromaSplit| 0.,
    reach: |e: &ChromaSplit| e.dx.abs().max(e.dy.abs()) + e.radial.abs() + 1.,
    frost: |_: &ChromaSplit| 0.,
    animated: |_: &ChromaSplit| false,
    idle: |e: &ChromaSplit| e.dx == 0. && e.dy == 0. && e.radial == 0.,
    life: |_: &ChromaSplit| 0.,
    one_shot: false,
);
effect!(
    /// Ribbed glass: each rib a little lens over what's behind the box.
    FlutedGlass: layer "fluted_glass" "fluted.wgsl" {
        /// A rib's width, in points.
        rib: f32 = 14. => 0 0,
        /// The way it runs across, in radians.
        angle: f32 = 0. => 0 1,
        /// How far each bends, in points.
        refraction: f32 = 9. => 0 2,
        /// How much its colours split, 0 to 1.
        dispersion: f32 = 0.15 => 0 3,
        /// Its frost: the Gaussian's sigma of what's behind it, in points.
        frost: f32 = 0. => 1 0,
        /// Its sheen's strength, 0 to 1.
        highlight: f32 = 0.5 => 1 1,
    }
    presets {
        /// Reeded glass.
        reeded {}
        /// Fine ribs.
        fine { rib: 6., refraction: 4. }
        /// Frosted ribs.
        frosted { frost: 6. }
        /// Wide ribs.
        wide { rib: 30., refraction: 18. }
    }
    fps: None,
    bleed: |_: &FlutedGlass| 0.,
    reach: |e: &FlutedGlass| e.refraction.abs() * 1.2,
    frost: |e: &FlutedGlass| e.frost,
    animated: |_: &FlutedGlass| false,
    idle: |_: &FlutedGlass| false,
    life: |_: &FlutedGlass| 0.,
    one_shot: false,
);
effect!(
    /// A polished metal surface, rippling, reflecting a studio.
    LiquidMetal: fill "liquid_metal" "liquid_metal.wgsl" {
        /// Its tint at normal incidence.
        tint: Color = gray(210) => 0 0,
        /// How much it ripples; 0, a still slab at a quarter of the cost.
        flow: f32 = 0.6 => 0 3,
        /// The ripples' size, in points.
        scale: f32 = 100. => 1 3,
        /// How fast they move.
        speed: f32 = 0.35 => 2 3,
        /// How wide its rolled rim is, in points.
        bevel: f32 = 14. => 3 3,
    }
    presets {
        /// Chrome.
        chrome {}
        /// Gold.
        gold { tint: rgb(0xf0c040) }
        /// Rose gold.
        rose { tint: rgb(0xf0a090) }
        /// Gunmetal.
        gunmetal { tint: rgb(0x70757d), flow: 0.4 }
    }
    fps: None,
    bleed: |_: &LiquidMetal| 0.,
    reach: |_: &LiquidMetal| 0.,
    frost: |_: &LiquidMetal| 0.,
    animated: |e: &LiquidMetal| e.flow > 0.,
    idle: |_: &LiquidMetal| false,
    life: |_: &LiquidMetal| 0.,
    one_shot: false,
);
effect!(
    /// Colours drifting into one another.
    MeshGradient: fill "mesh_gradient" "mesh.wgsl" {
        /// The first colour.
        a: Color = rgb(0x1b1f3b) => 0 0,
        /// How far the colours drift, 0 to 1 of the box.
        drift: f32 = 0.12 => 0 3,
        /// The second colour.
        b: Color = rgb(0xff6f91) => 1 0,
        /// How sharply each holds its place.
        sharpness: f32 = 1.6 => 1 3,
        /// The third colour.
        c: Color = rgb(0xffc75f) => 2 0,
        /// How fast they drift.
        speed: f32 = 0.25 => 2 3,
        /// How much the space between them bends, 0 to 1.
        warp: f32 = 0.35 => 3 3,
    }
    presets {
        /// A dusk.
        dusk {}
        /// A lagoon.
        lagoon { a: rgb(0x042f4b), b: rgb(0x00a8a8), c: rgb(0xa3f7bf) }
        /// An orchid.
        orchid { a: rgb(0x2a0a4a), b: rgb(0xb5179e), c: rgb(0xf7aef8) }
        /// Graphite.
        graphite { a: rgb(0x14161a), b: rgb(0x3a3f47), c: rgb(0x8d939c), drift: 0.08 }
    }
    fps: None,
    bleed: |_: &MeshGradient| 0.,
    reach: |_: &MeshGradient| 0.,
    frost: |_: &MeshGradient| 0.,
    animated: |e: &MeshGradient| e.drift > 0. && e.speed != 0.,
    idle: |_: &MeshGradient| false,
    life: |_: &MeshGradient| 0.,
    one_shot: false,
);
choice!(
    /// How a dither scatters its tone.
    DitherPattern {
        /// A Bayer matrix's dots.
        Bayer = "bayer" = default,
        /// White noise.
        Noise = "noise",
    }
);
effect!(
    /// A ramp quantized between two colours in square pixels.
    Dither: fill "dither" "dither.wgsl" {
        /// The first colour.
        a: Color = rgb(0x12121a) => 0 0,
        /// A pixel's size, in points.
        pixel: f32 = 2. => 0 3,
        /// The second colour.
        b: Color = rgb(0xe8e6df) => 1 0,
        /// The way the ramp runs, in radians.
        angle: f32 = 1.2 => 1 3,
        /// Its pattern.
        pattern: DitherPattern = DitherPattern::Bayer => 2 3,
        /// How fast its tone drifts.
        speed: f32 = 0. => 3 3,
    }
    presets {
        /// Bayer dots.
        bayer {}
        /// White noise, coarser.
        grain { pattern: DitherPattern::Noise, pixel: 3. }
    }
    fps: Some(30.),
    bleed: |_: &Dither| 0.,
    reach: |_: &Dither| 0.,
    frost: |_: &Dither| 0.,
    animated: |e: &Dither| e.speed != 0.,
    idle: |_: &Dither| false,
    life: |_: &Dither| 0.,
    one_shot: false,
);
choice!(
    /// A gradient's shape.
    GradientKind {
        /// Along a line.
        Linear = "linear" = default,
        /// Outward from the middle.
        Radial = "radial",
        /// Round the middle.
        Conic = "conic",
    }
);
effect!(
    /// A gradient whose colours stipple into each other.
    GrainGradient: fill "grain_gradient" "grain_gradient.wgsl" {
        /// The first colour.
        a: Color = rgb(0x1a0b2e) => 0 0,
        /// How much noise scatters it, 0 to 1.
        amount: f32 = 0.4 => 0 3,
        /// The second colour.
        b: Color = rgb(0xe0533d) => 1 0,
        /// Its shape.
        shape: GradientKind = GradientKind::Linear => 1 3,
        /// The third colour.
        c: Color = rgb(0xfbd38d) => 2 0,
        /// The way it runs, in radians.
        angle: f32 = 1.2 => 2 3,
        /// New noise this many times a second; 0, still.
        fps: f32 = 0. => 3 3,
    }
    presets {
        /// An ember.
        ember {}
        /// A halo.
        halo { shape: GradientKind::Radial, a: rgb(0xfff1c2), b: rgb(0x7a7bff), c: rgb(0x0b0b1e) }
        /// A sweep.
        sweep { shape: GradientKind::Conic, a: rgb(0x0b3d91), b: rgb(0x22d3ee), c: rgb(0xfde68a) }
    }
    fps: Some(24.),
    bleed: |_: &GrainGradient| 0.,
    reach: |_: &GrainGradient| 0.,
    frost: |_: &GrainGradient| 0.,
    animated: |e: &GrainGradient| e.fps > 0.,
    idle: |_: &GrainGradient| false,
    life: |_: &GrainGradient| 0.,
    one_shot: false,
);
effect!(
    /// Blobs merging, their edge antialiased, a glossy rim.
    Metaballs: fill "metaballs" "metaballs.wgsl" {
        /// A blob's colour.
        a: Color = rgb(0xff5d3a) => 0 0,
        /// A blob's size, as a fraction of the box's shorter side.
        size: f32 = 0.2 => 0 3,
        /// Another blob's colour.
        b: Color = rgb(0xffc24b) => 1 0,
        /// How fast they move.
        speed: f32 = 0.4 => 1 3,
        /// What's behind them.
        back: Color = rgb(0x24060a) => 2 0,
        /// How glossy, 0 to 1.
        gloss: f32 = 0.5 => 2 3,
        /// How many, up to 8.
        count: f32 = 6. => 3 3,
    }
    presets {
        /// Lava.
        lava {}
        /// Mint.
        mint { a: rgb(0x38e8b0), b: rgb(0x9bf6ff), back: rgb(0x04211f) }
    }
    fps: None,
    bleed: |_: &Metaballs| 0.,
    reach: |_: &Metaballs| 0.,
    frost: |_: &Metaballs| 0.,
    animated: |e: &Metaballs| e.speed != 0.,
    idle: |_: &Metaballs| false,
    life: |_: &Metaballs| 0.,
    one_shot: false,
);
effect!(
    /// A twist over warped noise, through a colour ramp. Among the heaviest
    /// per pixel: for cards rather than whole windows.
    Swirl: fill "swirl" "swirl.wgsl" {
        /// The first colour.
        a: Color = rgb(0x0b1020) => 0 0,
        /// Its size, in points.
        scale: f32 = 140. => 0 3,
        /// The second colour.
        b: Color = rgb(0x3b5bdb) => 1 0,
        /// How fast it turns.
        speed: f32 = 0.2 => 1 3,
        /// The third colour.
        c: Color = rgb(0xe9ecef) => 2 0,
        /// How much it twists.
        twist: f32 = 0.6 => 2 3,
        /// How much it warps.
        warp: f32 = 1. => 3 3,
    }
    presets {
        /// Ink.
        ink {}
        /// Marble.
        marble { a: rgb(0xece7df), b: rgb(0xa89f91), c: rgb(0x3a3631), twist: 0.2, speed: 0.08 }
    }
    fps: Some(30.),
    bleed: |_: &Swirl| 0.,
    reach: |_: &Swirl| 0.,
    frost: |_: &Swirl| 0.,
    animated: |e: &Swirl| e.speed != 0.,
    idle: |_: &Swirl| false,
    life: |_: &Swirl| 0.,
    one_shot: false,
);
effect!(
    /// Orbiting cells with exact seams.
    Voronoi: fill "voronoi" "voronoi.wgsl" {
        /// A cell's colour.
        a: Color = rgb(0x18324f) => 0 0,
        /// A cell's size, in points.
        scale: f32 = 36. => 0 3,
        /// Another cell's colour.
        b: Color = rgb(0x2f7a9b) => 1 0,
        /// Turns a second.
        speed: f32 = 0.08 => 1 3,
        /// The seam's colour.
        seam: Color = rgb(0xb7e3f0) => 2 0,
        /// The seam's width, in points.
        edge: f32 = 1.5 => 2 3,
        /// How much each cell darkens from its middle, 0 to 1.
        depth: f32 = 0.4 => 3 3,
    }
    presets {
        /// Cells.
        cells {}
        /// Stained glass.
        stained { a: rgb(0xb5322a), b: rgb(0x1f5fa8), seam: rgb(0x14110f), edge: 2.5, speed: 0. }
    }
    fps: Some(30.),
    bleed: |_: &Voronoi| 0.,
    reach: |_: &Voronoi| 0.,
    frost: |_: &Voronoi| 0.,
    animated: |e: &Voronoi| e.speed != 0.,
    idle: |_: &Voronoi| false,
    life: |_: &Voronoi| 0.,
    one_shot: false,
);
effect!(
    /// Layered bands, each shading the one behind.
    Waves: fill "waves" "waves.wgsl" {
        /// The farthest colour.
        a: Color = rgb(0x8ecae6) => 0 0,
        /// How tall the crests are, as a fraction of the box.
        amplitude: f32 = 0.05 => 0 3,
        /// The middle colour.
        b: Color = rgb(0x219ebc) => 1 0,
        /// Crests across the box.
        frequency: f32 = 1.6 => 1 3,
        /// The nearest colour.
        c: Color = rgb(0x023047) => 2 0,
        /// How fast they move.
        speed: f32 = 0.4 => 2 3,
        /// How soft an edge is, as a fraction of the box.
        softness: f32 = 0.006 => 3 3,
    }
    presets {
        /// The sea.
        sea {}
        /// Dunes.
        dunes { a: rgb(0xf6d6ad), b: rgb(0xd99a6c), c: rgb(0x7b4b35), amplitude: 0.08, frequency: 0.8, speed: 0.08 }
    }
    fps: None,
    bleed: |_: &Waves| 0.,
    reach: |_: &Waves| 0.,
    frost: |_: &Waves| 0.,
    animated: |e: &Waves| e.speed != 0.,
    idle: |_: &Waves| false,
    life: |_: &Waves| 0.,
    one_shot: false,
);
choice!(
    /// What a halftone's dots follow.
    HalftoneTone {
        /// Smaller toward the corners.
        Radial = "radial" = default,
        /// Smaller toward the bottom.
        Linear = "linear",
    }
);
effect!(
    /// Round dots on a rotated screen, sized by a tone.
    Halftone: fill "halftone" "halftone.wgsl" {
        /// The ink's colour.
        ink: Color = rgb(0x111111) => 0 0,
        /// A cell's size, in points.
        cell: f32 = 6. => 0 3,
        /// The paper's colour.
        base: Color = rgb(0xf4efe6) => 1 0,
        /// The screen's angle, in radians.
        angle: f32 = 0.785 => 1 3,
        /// What the dots follow.
        tone: HalftoneTone = HalftoneTone::Radial => 2 3,
    }
    presets {
        /// A printed page.
        print {}
    }
    fps: None,
    bleed: |_: &Halftone| 0.,
    reach: |_: &Halftone| 0.,
    frost: |_: &Halftone| 0.,
    animated: |_: &Halftone| false,
    idle: |_: &Halftone| false,
    life: |_: &Halftone| 0.,
    one_shot: false,
);
effect!(
    /// Moulded plastic under one soft light. Never moves.
    Plastic: fill "plastic" "plastic.wgsl" {
        /// Its colour.
        color: Color = rgb(0xd7263d) => 0 0,
        /// 0 glossy to 1 matte.
        roughness: f32 = 0.22 => 0 3,
        /// How much its face bulges; below 0, dished as a keycap.
        dome: f32 = 0.35 => 1 3,
        /// How wide its rolled rim is, in points.
        bevel: f32 = 6. => 2 3,
        /// Where its light comes from, in radians from above.
        angle: f32 = -0.5 => 3 3,
    }
    presets {
        /// Glossy.
        glossy {}
        /// Matte.
        matte { roughness: 0.7 }
        /// A dished keycap.
        dished { dome: -0.4, roughness: 0.5 }
    }
    fps: None,
    bleed: |_: &Plastic| 0.,
    reach: |_: &Plastic| 0.,
    frost: |_: &Plastic| 0.,
    animated: |_: &Plastic| false,
    idle: |_: &Plastic| false,
    life: |_: &Plastic| 0.,
    one_shot: false,
);

effect!(
    /// A ripple through what's painted, from where it's fired: a wave at a
    /// finite speed, dying exponentially: `Fx::fire(x, y)`.
    Wave: layer "wave" "wave.wgsl" {
        /// How far it moves pixels, in points.
        amplitude: f32 = 12. => 0 0,
        /// How fast it swings, in radians a second.
        frequency: f32 = 15. => 0 1,
        /// How fast it dies.
        decay: f32 = 8. => 0 2,
        /// How fast it travels, in points a second.
        speed: f32 = 1200. => 0 3,
    }
    fps: None,
    bleed: |_: &Wave| 0.,
    reach: |_: &Wave| 0.,
    frost: |_: &Wave| 0.,
    animated: |_: &Wave| true,
    idle: |_: &Wave| false,
    life: |_: &Wave| 2.,
    one_shot: true,
);
effect!(
    /// Clear glass: flat in its middle and at its very edge, what's behind it
    /// bent in the band between, each colour by its own amount.
    RimLens: layer "rim_lens" "rim_lens.wgsl" {
        /// How wide the band is, in points.
        width: f32 = 18. => 0 0,
        /// How far it bends at most, in points.
        strength: f32 = 14. => 0 1,
        /// How much its colours split, 0 to 1.
        dispersion: f32 = 0.18 => 0 2,
        /// Its lit edge's strength, 0 to 1.
        highlight: f32 = 0.25 => 0 3,
        /// A colour it takes on.
        tint: Color = gray(255) => 1 0,
        /// How much of it, 0 to 1.
        tint_amount: f32 = 0.04 => 1 3,
    }
    presets {
        /// Clear glass.
        clear {}
        /// A wide, strong rim.
        thick { width: 30., strength: 26., dispersion: 0.26 }
    }
    fps: None,
    bleed: |_: &RimLens| 0.,
    reach: |e: &RimLens| e.strength.abs() * (1. + e.dispersion.abs()) + 1.,
    frost: |_: &RimLens| 0.,
    animated: |_: &RimLens| false,
    idle: |_: &RimLens| false,
    life: |_: &RimLens| 0.,
    one_shot: false,
);
/// A table per effect: what `Fx::build`, `catalog` and the docs go through.
macro_rules! family {
    ($($name:ident),* $(,)?) => {
        impl Fx {
            /// Make the effect `name` (as the Luau module has it), from
            /// `preset` if one's named, with `fields` set over that. `Err`
            /// says what's wrong, for a script's error.
            pub fn build<'a>(
                name: &str,
                preset: Option<&str>,
                fields: impl IntoIterator<Item = (&'a str, FxValue<'a>)>,
            ) -> Result<Fx, String> {
                $(if <$name>::NAME == name {
                    let mut effect = match preset {
                        Some(preset) => <$name>::preset(preset).ok_or_else(|| {
                            format!("fx.{name} has no preset `{preset}` (it has: {})", <$name>::PRESETS.join(", "))
                        })?,
                        None => <$name>::default(),
                    };
                    for (field, value) in fields {
                        if !effect.set(field, value) {
                            return Err(format!(
                                "fx.{name}: `{field}` isn't one of its parameters, or isn't of its type: {:?} (it has: {})",
                                value,
                                <$name>::FIELDS.join(", ")
                            ));
                        }
                    }
                    return Ok(effect.into());
                })*
                Err(format!("fx has no effect `{name}` (it has: {})", NAMES.join(", ")))
            }
        }
        /// Every effect's name, as the Luau module has it.
        pub const NAMES: &[&str] = &[$(<$name>::NAME),*];
        /// Every effect's name, how it draws and its shader's source.
        pub fn sources() -> Vec<(&'static str, FxKind, &'static str)> {
            vec![$((<$name>::NAME, Fx::from(<$name>::default()).kind, <$name>::SOURCE)),*]
        }
        /// Every effect and preset, named, for tests, galleries and tools: at
        /// default parameters otherwise, each with a one-shot alive.
        pub fn catalog() -> Vec<(String, Fx)> {
            let mut all = Vec::new();
            $(
                let fx: Fx = <$name>::default().into();
                fx.fire(40., 30.);
                all.push((<$name>::NAME.to_owned(), fx));
                for preset in <$name>::PRESETS {
                    let effect = <$name>::preset(preset).expect("its own preset");
                    all.push((format!("{} {preset}", <$name>::NAME), effect.into()));
                }
            )*
            all
        }
    };
}
family!(
    Glow,
    Pulse,
    Ripple,
    Shimmer,
    Sparkles,
    Aurora,
    Grain,
    Electric,
    Fire,
    MotionBlur,
    HeatHaze,
    Shockwave,
    Lens,
    Crt,
    ChromaSplit,
    FlutedGlass,
    LiquidMetal,
    MeshGradient,
    Dither,
    GrainGradient,
    Metaballs,
    Swirl,
    Voronoi,
    Waves,
    Halftone,
    Plastic,
    Wave,
    RimLens,
);
