//! 监视器(Monitor):承接扫描器产出的目标设备 RSSI,把"一次次原始扫描结果"
//! 折叠成"是否应该锁屏"这一个判断。
//!
//! 设计要点:
//! - 不直接执行锁屏,只产出状态与标志位,由上层(locker)决定动作;
//! - `should_lock` 是**单向锁存**:一旦置 true 不自动回落 —— 抗抖动。
//!   想撤回锁屏决策走 [`Monitor::reset`];想热替换配置走 [`Monitor::set_config`]。

/// 目标设备相对于本机的状态分类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhoneStatus {
    /// 信号强度 ≥ 阈值,认为设备就在附近。
    Nearby,
    /// 本次扫到了设备,但 RSSI 低于阈值 —— 人正在走远。
    WeakSignal,
    /// 本次扫描没看到设备 —— 可能已经走出蓝牙范围。
    Missing,
}

impl PhoneStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Nearby => "在身边",
            Self::WeakSignal => "信号弱",
            Self::Missing => "丢失",
        }
    }
}

/// 监视器的运行参数。
#[derive(Debug, Clone)]
pub struct MonitorConfig {
    /// 触发"远离"判定的 RSSI 阈值(dBm,负值)。
    pub lock_rssi: i16,
    /// 允许的连续"非 Nearby"次数上限,达到即置位 should_lock。
    pub missing_limit: u32,
}

/// 监视器本体。持有配置 + 累积状态。
#[derive(Debug)]
pub struct Monitor {
    config: MonitorConfig,
    missing_count: u32,
    should_lock: bool,
}

impl Monitor {
    pub fn new(config: MonitorConfig) -> Self {
        Self {
            config,
            missing_count: 0,
            should_lock: false,
        }
    }

    /// 喂入本轮扫描结果,推进内部状态,并返回这一轮的分类结果。
    pub fn update(&mut self, rssi: Option<i16>) -> PhoneStatus {
        match rssi {
            Some(value) if value >= self.config.lock_rssi => {
                // 信号够强 —— 重置耐心计数器。
                self.missing_count = 0;
                PhoneStatus::Nearby
            }
            Some(_) => {
                // 扫到了但太弱。
                self.missing_count += 1;
                self.check_lock();
                PhoneStatus::WeakSignal
            }
            None => {
                // 没扫到。Missing 与 WeakSignal 共用同一计数器,对"该不该锁"等价。
                self.missing_count += 1;
                self.check_lock();
                PhoneStatus::Missing
            }
        }
    }

    /// 是否已经满足锁屏条件。锁存语义:一旦为 true 不会自动回落。
    pub fn should_lock(&self) -> bool {
        self.should_lock
    }

    pub fn missing_count(&self) -> u32 {
        self.missing_count
    }

    /// 显式重置 —— 把 `missing_count` 清零、把锁存的 `should_lock` 复位。
    /// 用于"用户取消锁屏并继续保护"。
    pub fn reset(&mut self) {
        self.missing_count = 0;
        self.should_lock = false;
    }

    /// **热替换配置** —— 不重置计数,但立即重跑一次锁屏判定。
    /// `should_lock` 锁存**不会**被撤回(放宽阈值跳过决定的攻击)。
    pub fn set_config(&mut self, config: MonitorConfig) {
        self.config = config;
        self.check_lock();
    }

    fn check_lock(&mut self) {
        if self.missing_count >= self.config.missing_limit {
            self.should_lock = true;
        }
    }
}
