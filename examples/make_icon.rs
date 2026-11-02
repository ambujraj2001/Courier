//! Regenerates the app icons: PNGs used by `cargo bundle` and the Windows
//! `courier.ico` embedded by build.rs. Run `cargo run --example make_icon`.

fn main() {
    for size in [16u32, 32, 128, 256, 512, 1024] {
        let rgba = courier::app::theme::icon_rgba(size);
        let img = image::RgbaImage::from_raw(size, size, rgba).expect("icon buffer");
        // 1024 px is the 512 pt @2x icon in an .icns file.
        let path = if size == 1024 {
            "assets/icon/icon_512x512@2x.png".to_string()
        } else {
            format!("assets/icon/icon_{size}x{size}.png")
        };
        img.save(&path).expect("write icon");
        println!("wrote {path}");
    }

    // Windows icon with the sizes Explorer and the taskbar use.
    let frames: Vec<_> = [16u32, 24, 32, 48, 64, 128, 256]
        .into_iter()
        .map(|size| {
            image::codecs::ico::IcoFrame::as_png(
                &courier::app::theme::icon_rgba(size),
                size,
                size,
                image::ExtendedColorType::Rgba8,
            )
            .expect("icon frame")
        })
        .collect();
    let file = std::fs::File::create("assets/icon/courier.ico").expect("create courier.ico");
    image::codecs::ico::IcoEncoder::new(file).encode_images(&frames).expect("write courier.ico");
    println!("wrote assets/icon/courier.ico");
}
