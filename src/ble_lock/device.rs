//! BLE 设备的纯数据表示与匹配策略 —— Scanner 与上层(Monitor / UI)的契约类型。
//!
//! 设计要点:
//! - [`DeviceInfo`] 是**值类型**(Clone),不持有蓝牙连接句柄,跨线程 / 跨 async 安全传递;
//! - [`DeviceMatcher`] 把"按什么识别"的决定权交给上层。本工程默认走 [`DeviceMatcher::Name`]。

/// 一次扫描中观察到的单个 BLE 外设的可读快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// btleplug 的 `PeripheralId`(转字符串)。
    /// - macOS:CoreBluetooth 分配的 UUID,**配对后**才稳定;
    /// - Linux / Windows:就是 MAC 地址。
    pub id: String,
    /// 蓝牙 BD_ADDR,字符串形式如 `"AA:BB:CC:DD:EE:FF"`。
    /// macOS 出于隐私一律返回 `"00:00:00:00:00:00"` —— 在 mac 上别用它做匹配。
    pub address: String,
    /// 广播包或 scan response 中的本地名字。许多设备只广播服务 UUID 时为 `None`。
    pub local_name: Option<String>,
    /// 接收信号强度,dBm,负值,越接近 0 越强。
    pub rssi: Option<i16>,
}

/// "如何认出我的目标设备" —— 由上层注入的识别策略。
// `Id` / `Address` 变体当前未被构造 —— 工程默认 `Name` 匹配;留着是为了
// 未来可能的 cross-platform 识别策略切换(macOS 配对后 Id 比 Name 更稳)。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum DeviceMatcher {
    /// 按 `PeripheralId` 精确匹配。macOS 上最可靠,但需先与设备配过对。
    Id(String),
    /// 按 BD_ADDR 精确匹配。macOS 拿不到真实地址,实际只在 Linux / Windows 有意义。
    Address(String),
    /// 按 `local_name` 精确匹配。跨平台通用,本工程默认走这条路径。
    Name(String),
}

impl DeviceMatcher {
    /// 判断给定的设备快照是否匹配当前策略。
    pub fn matches(&self, info: &DeviceInfo) -> bool {
        match self {
            Self::Id(s) => info.id == *s,
            Self::Address(s) => info.address == *s,
            Self::Name(s) => info.local_name.as_deref() == Some(s.as_str()),
        }
    }
}
