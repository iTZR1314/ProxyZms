#[path = "../src/macos_icon.rs"]
mod macos_icon;

fn main() {
    let output = std::env::args_os()
        .nth(1)
        .unwrap_or_else(|| "assets/fmr-macos.png".into());
    let png = macos_icon::rounded_icon_png().expect("生成 macOS 图标失败");
    std::fs::write(&output, png).expect("写入 macOS 图标失败");
    println!("已生成 {}", std::path::PathBuf::from(output).display());
}
