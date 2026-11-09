//! A zgx WebAssembly app in Rust: implement [`App`], name it with
//! [`app!`], build with `cargo build --release --target wasm32-wasip2`
//! (a `cdylib`), and show the `.wasm` with `ui.app(id, { wasm = … })`.
//!
//! ```ignore
//! struct Hello { size: (u32, u32) }
//! impl zgx_app::App for Hello {
//!     fn init(info: zgx_app::SurfaceInfo) -> Result<Self, String> {
//!         Ok(Hello { size: (info.width, info.height) })
//!     }
//!     fn frame(&mut self, _time: f64) {
//!         let (w, h) = self.size;
//!         let pixels = vec![255u8; (w * h * 4) as usize];
//!         zgx_app::host::present_pixels(&pixels, w, h).unwrap();
//!     }
//! }
//! zgx_app::app!(Hello);
//! ```
//!
//! [`gpu`] is WebGPU on the host's own device (`util` has descriptors with
//! WebGPU's defaults); [`host`] is the element (title, cursor, pointer
//! lock, pacing, CPU pixels).
pub mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "app",
        pub_export_macro: true,
        export_macro_name: "export_world",
        default_bindings_module: "zgx_app::bindings",
    });
}

pub use bindings::zgx::app::{
    gpu, host,
    types::{
        Button, ButtonEvent, Cursor, InputEvent, KeyEvent, LogLevel, Modifiers, Motion, Point,
        SurfaceInfo, WheelEvent, WheelUnit,
    },
};

/// An app: made once with the element's size, then told each input and
/// asked for frames (every display refresh while it shows, unless it calls
/// `host::set_animating(false)`; then after `host::request_frame()`).
pub trait App: Sized + 'static {
    fn init(info: SurfaceInfo) -> Result<Self, String>;
    /// Draw: `time` is seconds since the session began, as the display
    /// shows the frame. GPU objects are best made here, the first time (the
    /// device is always there in a frame), and again after `GpuReset`.
    fn frame(&mut self, time: f64);
    fn event(&mut self, _event: InputEvent) {}
}

/// Exports `$app` as the component's `zgx:app/app`.
#[macro_export]
macro_rules! app {
    ($app:ty) => {
        struct __ZgxApp;
        impl $crate::bindings::Guest for __ZgxApp {
            fn init(info: $crate::SurfaceInfo) -> ::core::result::Result<(), ::std::string::String> {
                $crate::__start::<$app>(info)
            }
            fn frame(time: f64) {
                $crate::__with::<$app>(|app| $crate::App::frame(app, time))
            }
            fn event(event: $crate::InputEvent) {
                $crate::__with::<$app>(|app| $crate::App::event(app, event))
            }
        }
        $crate::bindings::export_world!(__ZgxApp with_types_in $crate::bindings);
    };
}

thread_local! {
    static APP: std::cell::RefCell<Option<Box<dyn std::any::Any>>> = const { std::cell::RefCell::new(None) };
}
#[doc(hidden)]
pub fn __start<A: App>(info: SurfaceInfo) -> Result<(), String> {
    let app = A::init(info)?;
    APP.with(|cell| *cell.borrow_mut() = Some(Box::new(app)));
    Ok(())
}
#[doc(hidden)]
pub fn __with<A: App>(f: impl FnOnce(&mut A)) {
    APP.with(|cell| {
        if let Some(app) = cell
            .borrow_mut()
            .as_mut()
            .and_then(|a| a.downcast_mut::<A>())
        {
            f(app);
        }
    });
}

/// A line of the session's log.
pub fn log(message: impl AsRef<str>) {
    host::log(LogLevel::Info, message.as_ref());
}

/// WebGPU's defaults for the descriptors apps fill most.
pub mod util {
    use super::gpu::*;

    pub fn primitive(topology: PrimitiveTopology, cull_mode: CullMode) -> PrimitiveState {
        PrimitiveState {
            topology,
            strip_index_format: None,
            front_face: FrontFace::Ccw,
            cull_mode,
            unclipped_depth: false,
        }
    }
    pub fn multisample(count: u32) -> MultisampleState {
        MultisampleState {
            count,
            mask: u64::MAX,
            alpha_to_coverage_enabled: false,
        }
    }
    /// A colour target written whole, no blending.
    pub fn target(format: TextureFormat) -> ColorTargetState {
        ColorTargetState {
            format,
            blend: None,
            write_mask: ColorWrite::all(),
        }
    }
    pub fn depth(format: TextureFormat, compare: CompareFunction) -> DepthStencilState {
        DepthStencilState {
            format,
            depth_write_enabled: true,
            depth_compare: compare,
            stencil: None,
            depth_bias: 0,
            depth_bias_slope_scale: 0.,
            depth_bias_clamp: 0.,
        }
    }
    /// A 2D texture of one mip level.
    pub fn texture(
        label: &str,
        width: u32,
        height: u32,
        format: TextureFormat,
        usage: TextureUsage,
        sample_count: u32,
    ) -> TextureDescriptor {
        TextureDescriptor {
            label: Some(label.into()),
            size: Extent {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: TextureDimension::D2,
            format,
            usage,
            view_formats: Vec::new(),
        }
    }
    pub fn uniform_entry(binding: u32, visibility: ShaderStage) -> BindGroupLayoutEntry {
        BindGroupLayoutEntry {
            binding,
            visibility,
            binding_type: BindingType::Buffer(BufferBindingLayout {
                kind: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            }),
        }
    }
    /// A stencil face that always passes, doing `pass` with what passes.
    pub fn stencil_face(compare: CompareFunction, pass: StencilOperation) -> StencilFaceState {
        StencilFaceState {
            compare,
            fail_op: StencilOperation::Keep,
            depth_fail_op: StencilOperation::Keep,
            pass_op: pass,
        }
    }
    /// The same test and operation on both faces, every bit.
    pub fn stencil(compare: CompareFunction, pass: StencilOperation) -> StencilState {
        StencilState {
            front: stencil_face(compare, pass),
            back: stencil_face(compare, pass),
            read_mask: 0xff,
            write_mask: 0xff,
        }
    }
    pub fn view(texture: &Texture) -> TextureView {
        texture.create_view(None).expect("a texture's whole view")
    }
}
