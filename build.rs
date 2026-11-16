//! Build script: derives the Windows exe icon (.ico) and a small runtime icon
//! from `assets/icon.png`, so only a few KB are embedded instead of the full image.

use image::imageops::FilterType;
use image::{ImageFormat, codecs::ico::{IcoEncoder, IcoFrame}, ExtendedColorType};
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.png");
    println!("cargo:rerun-if-changed=build.rs");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let src = image::open("assets/icon.png").expect("assets/icon.png").to_rgba8();

    // Runtime icon (window / title bar / in-app logo).
    let rt = image::imageops::resize(&src, 256, 256, FilterType::Lanczos3);
    rt.save_with_format(out.join("icon256.png"), ImageFormat::Png).unwrap();

    // Multi-size .ico for the executable.
    let mut frames = Vec::new();
    for size in [256u32, 64, 48, 32, 24, 16] {
        let img = image::imageops::resize(&src, size, size, FilterType::Lanczos3);
        frames.push(IcoFrame::as_png(img.as_raw(), size, size, ExtendedColorType::Rgba8).unwrap());
    }
    let ico_path = out.join("app.ico");
    let f = std::fs::File::create(&ico_path).unwrap();
    IcoEncoder::new(f).encode_images(&frames).unwrap();

    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico_path.to_str().unwrap());
        res.set("ProductName", "ACOM Controller");
        res.set("FileDescription", "ACOM Controller");
        res.compile().expect("compile windows resources");
    }
}
