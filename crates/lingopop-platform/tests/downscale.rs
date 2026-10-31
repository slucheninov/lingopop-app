use lingopop_platform::capture::{downscale_to_fit, CapturedImage};
fn image(w: u32, h: u32) -> CapturedImage {
    CapturedImage::new(w, h, vec![255; (w * h * 4) as usize]).unwrap()
}
#[test]
fn downscale_keeps_aspect_ratio() {
    let out = downscale_to_fit(&image(12, 8), 6);
    assert_eq!((out.width, out.height), (6, 4));
}
#[test]
fn downscale_noop_when_fits() {
    let im = image(4, 3);
    assert_eq!(downscale_to_fit(&im, 4), im);
}
#[test]
fn downscale_averages_pixels() {
    let im = CapturedImage::new(
        2,
        2,
        vec![
            0, 20, 40, 60, 40, 60, 80, 100, 80, 100, 120, 140, 120, 140, 160, 180,
        ],
    )
    .unwrap();
    assert_eq!(downscale_to_fit(&im, 1).rgba, [60, 80, 100, 120]);
}
#[test]
fn downscale_never_zero_side() {
    for (w, h) in [(100, 1), (1, 100)] {
        let out = downscale_to_fit(&image(w, h), 2);
        assert!(out.width >= 1 && out.height >= 1 && out.width <= 2 && out.height <= 2);
    }
    assert_eq!(downscale_to_fit(&image(2, 2), 0).width, 1);
}
