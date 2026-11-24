//! What scripts ask of the renderer: zgx-gpu's image decoding and shader
//! checks, and in the browser (wasm32), where zgx-dom shows views and no
//! renderer is linked, their stand-ins.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use zgx_gpu::{
    assets::decode_image, assets::decode_image_kept, check_fill_shader, check_layer_inputs,
    check_layer_shader, warm_fill_shader,
};

#[cfg(target_arch = "wasm32")]
pub(crate) use web::*;

#[cfg(target_arch = "wasm32")]
mod web {
    use std::sync::Arc;
    use zgx::image::ImageData;

    /// Images are not decoded in the browser yet: the image shows its label.
    pub(crate) fn decode_image_kept(_: Arc<[u8]>) -> Result<Arc<ImageData>, String> {
        Err("images aren't decoded in the browser yet".into())
    }
    /// Shaders draw only on a GPU, which the browser host doesn't have:
    /// accepted unchecked, and shown as zgx-dom shows GPU content.
    pub(crate) fn check_fill_shader(_: &str) -> Result<(), String> {
        Ok(())
    }
    pub(crate) fn check_layer_shader(_: &str) -> Result<(), String> {
        Ok(())
    }
    pub(crate) fn check_layer_inputs(
        _: &str,
        _: &zgx::decoration::ShaderInputs,
    ) -> Result<(), String> {
        Ok(())
    }
    pub(crate) fn warm_fill_shader(_: &Arc<zgx::decoration::FillShader>) {}
}
