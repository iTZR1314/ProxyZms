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
    /// Locked 后允许的连续"Nearby"次数上限,达到即 should_rearm。
    pub rearm_limit: u32,
}

/// 监视器本体。持有配置 + 累积状态。
#[derive(Debug)]
pub struct Monitor {
    config: MonitorConfig,
    missing_count: u32,
    present_count: u32,
    /// "Watching 进入后是否至少扫到过一次 Nearby"。
    /// 未 armed 时 Weak/Missing 都不入 missing_count、不会触发 should_lock ——
    /// 防止"手机不在身边时启动保护,瞬间被自己锁出去"。
    armed: bool,
    should_lock: bool,
}

impl Monitor {
    pub fn new(config: MonitorConfig) -> Self {
        Self {
            config,
            missing_count: 0,
            present_count: 0,
            armed: false,
            should_lock: false,
        }
    }

    /// 喂入本轮扫描结果,推进内部状态,并返回这一轮的分类结果。
    pub fn update(&mut self, rssi: Option<i16>) -> PhoneStatus {
        match rssi {
            Some(value) if value >= self.config.lock_rssi => {
                // 信号够强 —— 锁定"已 armed",清耐心计数器、推进"已回归"计数。
                self.armed = true;
                self.missing_count = 0;
                self.present_count = self.present_count.saturating_add(1);
                PhoneStatus::Nearby
            }
            Some(_) => {
                // 扫到了但太弱:只有 armed 状态下才计入 missing。
                if self.armed {
                    self.missing_count += 1;
                    self.check_lock();
                }
                self.present_count = 0;
                PhoneStatus::WeakSignal
            }
            None => {
                // 没扫到:同 WeakSignal 处理。armed 之前的 Missing 不计入。
                if self.armed {
                    self.missing_count += 1;
                    self.check_lock();
                }
                self.present_count = 0;
                PhoneStatus::Missing
            }
        }
    }

    /// 是否已经满足锁屏条件。锁存语义:一旦为 true 不会自动回落。
    pub fn should_lock(&self) -> bool {
        self.should_lock
    }

    /// 是否满足"信号回归"条件:必须先经历过锁屏(should_lock 置位),
    /// 且随后连续 `rearm_limit` 拍都是 Nearby。
    pub fn should_rearm(&self) -> bool {
        self.should_lock && self.present_count >= self.config.rearm_limit
    }

    pub fn missing_count(&self) -> u32 {
        self.missing_count
    }

    /// 是否"已见过手机一次"。Watching 进入后到首次 Nearby 之间为 false,
    /// 此期间 Monitor 不会触发 should_lock。
    pub fn is_armed(&self) -> bool {
        self.armed
    }

    /// 显式完全重置 —— 计数 + armed + 锁存全清。用于"用户取消锁屏并继续保护"。
    /// 取消后需要重新被看到一次才会重新进入计数,语义合理(刚刚正是因为没看到才差点锁屏)。
    pub fn reset(&mut self) {
        self.missing_count = 0;
        self.present_count = 0;
        self.armed = false;
        self.should_lock = false;
    }

    /// **信号回归专用** —— 清计数与 should_lock 锁存,但 `armed` 保持 true。
    /// 走这条路时刚刚连续 `rearm_limit` 次 Nearby,身份已确认,无须重新 arm。
    pub fn rearm(&mut self) {
        self.missing_count = 0;
        self.present_count = 0;
        self.should_lock = false;
        // armed 故意不动:保持 true,Watching 立即恢复正常计数
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
