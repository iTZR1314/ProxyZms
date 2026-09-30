use image::{imageops::FilterType, ExtendedColorType, ImageBuffer, ImageEncoder, Rgba, RgbaImage};

/// 从主图生成 macOS Dock/bundle 共用的圆角图标 PNG。
pub fn rounded_icon_png() -> Result<Vec<u8>, String> {
    let src = image::load_from_memory(include_bytes!("../assets/fmr.png"))
        .map_err(|e| format!("读取应用图标失败:{e}"))?
        .to_rgba8();
    let canvas = 1024u32;
    let margin = 100u32;
    let content = canvas - margin * 2;
    let radius = content as f32 * 0.2237;
    let resized = image::imageops::resize(&src, content, content, FilterType::Lanczos3);
    let mut out: RgbaImage = ImageBuffer::from_pixel(canvas, canvas, Rgba([0, 0, 0, 0]));
    let half = content as f32 / 2.0;
    for y in 0..content {
        for x in 0..content {
            let px = (x as f32 + 0.5) - half;
            let py = (y as f32 + 0.5) - half;
            let qx = px.abs() - half + radius;
            let qy = py.abs() - half + radius;
            let distance = qx.max(qy).min(0.0)
                + (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt()
                - radius;
            let coverage = (0.5 - distance).clamp(0.0, 1.0);
            if coverage > 0.0 {
                let mut pixel = *resized.get_pixel(x, y);
                pixel[3] = (pixel[3] as f32 * coverage) as u8;
                out.put_pixel(x + margin, y + margin, pixel);
            }
        }
    }

    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&out, canvas, canvas, ExtendedColorType::Rgba8)
        .map_err(|e| format!("编码 macOS 图标失败:{e}"))?;
    Ok(png)
}
