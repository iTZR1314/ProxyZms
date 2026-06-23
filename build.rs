//! 构建脚本:Windows 上嵌入 requireAdministrator 清单 + 主图标
//! (不嵌 VERSION,避免与 dx 自己嵌入的 Windows 资源冲突),
//! 使双击即弹 UAC 以管理员运行(TUN/Wintun 需要管理员),且 Explorer / 任务栏 / 快捷方式有图标。
fn main() {
    #[cfg(windows)]
    {
        // proxyzms.rc 声明 RT_MANIFEST + 主 ICON,两个资源同 ID 不同 type,Win32 允许
        embed_resource::compile("proxyzms.rc", embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
}
