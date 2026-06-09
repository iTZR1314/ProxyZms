//! 构建脚本:仅在 Windows 上嵌入 requireAdministrator 清单(只嵌清单这一种资源,
//! 不带 VERSION/图标,避免与 dx 自己嵌入的 Windows 资源冲突),
//! 使双击即弹 UAC 以管理员运行(TUN/Wintun 需要管理员)。
fn main() {
    #[cfg(windows)]
    {
        // proxyzms.rc 仅声明一条 RT_MANIFEST(id=1)指向 proxyzms.manifest
        let _ = embed_resource::compile("proxyzms.rc", embed_resource::NONE);
    }
}
