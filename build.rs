//! 构建脚本:
//! - Windows:只嵌入 requireAdministrator 清单,使双击即弹 UAC 以管理员运行
//!   (TUN/Wintun 需要管理员)。
//!
//!   **ICON 与 VERSION 都不要在这里嵌** —— dx bundle 自 0.7.10 起会生成
//!   `.winres/resource.lib` 并嵌入它们,重复嵌会在链接期报
//!   `CVT1100: duplicate resource` / `LNK1123`。详见 proxyzms.rc 的注释。
fn main() {
    #[cfg(windows)]
    {
        // proxyzms.rc 现在只声明 RT_MANIFEST(id 1, type 24)
        embed_resource::compile("proxyzms.rc", embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
}
