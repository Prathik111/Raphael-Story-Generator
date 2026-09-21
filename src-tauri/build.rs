use std::{
    fs,
    io,
    path::{Path, PathBuf},
};

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let icon_path = manifest_dir.join("icons").join("icon.ico");

    if let Err(error) = ensure_fallback_icon(&icon_path) {
        panic!("failed to prepare Tauri Windows icon: {error}");
    }

    println!("cargo:rerun-if-changed={}", icon_path.display());
    tauri_build::build();
}

fn ensure_fallback_icon(path: &Path) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }

    let Some(parent) = path.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "icon path has no parent directory",
        ));
    };

    fs::create_dir_all(parent)?;
    fs::write(path, fallback_ico())?;
    Ok(())
}

fn fallback_ico() -> Vec<u8> {
    // A self-contained 32x32 32-bit BGRA Windows ICO.
    // It is intentionally simple so a fresh clone can build without
    // requiring a binary asset to already exist in the repository.
    const WIDTH: usize = 32;
    const HEIGHT: usize = 32;
    const XOR_SIZE: usize = WIDTH * HEIGHT * 4;
    const AND_SIZE: usize = ((WIDTH + 31) / 32) * 4 * HEIGHT;
    const DIB_SIZE: usize = 40;
    const IMAGE_SIZE: usize = DIB_SIZE + XOR_SIZE + AND_SIZE;
    const IMAGE_OFFSET: usize = 6 + 16;

    let mut bytes = Vec::with_capacity(IMAGE_OFFSET + IMAGE_SIZE);

    // ICONDIR
    bytes.extend_from_slice(&0u16.to_le_bytes()); // reserved
    bytes.extend_from_slice(&1u16.to_le_bytes()); // icon type
    bytes.extend_from_slice(&1u16.to_le_bytes()); // image count

    // ICONDIRENTRY
    bytes.push(WIDTH as u8);
    bytes.push(HEIGHT as u8);
    bytes.push(0); // color count
    bytes.push(0); // reserved
    bytes.extend_from_slice(&1u16.to_le_bytes()); // planes
    bytes.extend_from_slice(&32u16.to_le_bytes()); // bit count
    bytes.extend_from_slice(&(IMAGE_SIZE as u32).to_le_bytes());
    bytes.extend_from_slice(&(IMAGE_OFFSET as u32).to_le_bytes());

    // BITMAPINFOHEADER
    bytes.extend_from_slice(&(DIB_SIZE as u32).to_le_bytes());
    bytes.extend_from_slice(&(WIDTH as i32).to_le_bytes());
    bytes.extend_from_slice(&((HEIGHT * 2) as i32).to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // planes
    bytes.extend_from_slice(&32u16.to_le_bytes()); // bpp
    bytes.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    bytes.extend_from_slice(&(XOR_SIZE as u32).to_le_bytes());
    bytes.extend_from_slice(&0i32.to_le_bytes()); // x ppm
    bytes.extend_from_slice(&0i32.to_le_bytes()); // y ppm
    bytes.extend_from_slice(&0u32.to_le_bytes()); // colors used
    bytes.extend_from_slice(&0u32.to_le_bytes()); // important colors

    // Bottom-up BGRA pixels. Transparent background with a compact
    // Raphael-style purple diamond/letter mark in the center.
    for y in (0..HEIGHT).rev() {
        for x in 0..WIDTH {
            let dx = x as i32 - 15;
            let dy = y as i32 - 15;
            let in_diamond = dx.abs() + dy.abs() <= 11;
            let in_cutout = dx.abs() + dy.abs() <= 4;
            let in_core = dx.abs() <= 2 && dy.abs() <= 8;
            let visible = in_diamond && (!in_cutout || in_core);

            if visible {
                // Purple in BGRA order.
                bytes.extend_from_slice(&[220, 120, 170, 255]);
            } else {
                bytes.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }

    // 1-bit AND mask: all zero because alpha is already present.
    bytes.resize(bytes.len() + AND_SIZE, 0);

    debug_assert_eq!(bytes.len(), IMAGE_OFFSET + IMAGE_SIZE);
    bytes
}
