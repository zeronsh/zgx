//! An image whose pixels are made when first read (`ImageData::on_demand`).
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use zgx::image::ImageData;

#[test]
fn pixels_made_on_demand_are_made_once_when_first_read() {
    let made = Arc::new(AtomicUsize::new(0));
    let counted = made.clone();
    let image = ImageData::on_demand(2, 1, move || {
        counted.fetch_add(1, Ordering::Relaxed);
        Arc::from(&[1, 2, 3, 4, 5, 6, 7, 8][..])
    })
    .unwrap();
    // Nothing made until read; its size and budget known before.
    assert_eq!(made.load(Ordering::Relaxed), 0);
    assert_eq!((image.width(), image.height(), image.bytes()), (2, 1, 8));
    let copy = image.transformed(zgx::affine::Affine::IDENTITY);
    assert_eq!(copy.pixels(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(image.pixels(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(&*image.pixels_arc(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(&*image.pixels_now(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(made.load(Ordering::Relaxed), 1, "made more than once");
    // Bytes of the wrong length read as transparent black.
    let short = ImageData::on_demand(2, 1, || Arc::from(&[9][..])).unwrap();
    assert_eq!(short.pixels(), &[0; 8]);
    assert!(ImageData::on_demand(0, 1, || Arc::from(&[][..])).is_err());
    // A renderer's frame, premultiplied already: said so, drawn as it is.
    assert!(!image.is_premultiplied());
    let frame =
        ImageData::on_demand_premultiplied(1, 1, || Arc::from(&[64, 0, 0, 128][..])).unwrap();
    assert!(frame.is_premultiplied());
    assert_eq!(frame.pixels(), &[64, 0, 0, 128]);
}
