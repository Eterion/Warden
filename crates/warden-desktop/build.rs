//! Tauri's build step: the app's own commands become permissions the window's capability grants. Also takes the
//! taskbar's layers out of the app icon (`icons/icon.ico`, generated from `icons/warden.svg`) as raw RGBA, so the
//! window can give the taskbar the layer for its size.

use std::path::Path;

/// The ICO layers the window's taskbar button uses (`shell::TASKBAR_ICONS`).
const TASKBAR_LAYERS: [u32; 4] = [24, 32, 48, 64];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["pick_folder", "quit"])),
    )
    .expect("tauri build step");

    println!("cargo:rerun-if-changed=icons/icon.ico");
    let ico = std::fs::read("icons/icon.ico").expect("icons/icon.ico");
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    for size in TASKBAR_LAYERS {
        let rgba = layer(&ico, size);
        std::fs::write(Path::new(&out).join(format!("app-icon-{size}.rgba")), rgba)
            .expect("write an icon layer");
    }
}

/// The ICO's `size` by `size` layer as RGBA pixels. Every layer is a PNG (as `tauri icon` writes them).
fn layer(ico: &[u8], size: u32) -> Vec<u8> {
    let u16_at = |at: usize| u16::from_le_bytes([ico[at], ico[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes([ico[at], ico[at + 1], ico[at + 2], ico[at + 3]]);
    assert!(
        ico.len() >= 6 && u16_at(2) == 1,
        "icons/icon.ico is not an icon file"
    );
    for entry in (0..usize::from(u16_at(4))).map(|i| 6 + i * 16) {
        // A width or height of 0 means 256.
        let side = |b: u8| if b == 0 { 256 } else { u32::from(b) };
        if side(ico[entry]) != size || side(ico[entry + 1]) != size {
            continue;
        }
        let (len, at) = (u32_at(entry + 8) as usize, u32_at(entry + 12) as usize);
        let mut decoder = png::Decoder::new(&ico[at..at + len]);
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder
            .read_info()
            .unwrap_or_else(|e| panic!("icons/icon.ico's {size} pixel layer is not a PNG: {e}"));
        let mut pixels = vec![0; reader.output_buffer_size()];
        let frame = reader
            .next_frame(&mut pixels)
            .expect("decode an icon layer");
        assert_eq!((frame.width, frame.height), (size, size), "icon layer size");
        pixels.truncate(frame.buffer_size());
        return match frame.color_type {
            png::ColorType::Rgba => pixels,
            png::ColorType::Rgb => pixels
                .chunks(3)
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            other => panic!("icons/icon.ico's {size} pixel layer is {other:?}, not RGB or RGBA"),
        };
    }
    panic!("icons/icon.ico has no {size} by {size} layer");
}
