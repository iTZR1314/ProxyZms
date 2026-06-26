//! BLE 扫描器 —— 把 btleplug 的扫描动作封装成三种能力:
//!   1. [`Scanner::scan_all`][]:在指定时长内扫遍周围所有 BLE 外设;
//!   2. [`Scanner::find`][]:扫描并按 [`DeviceMatcher`] 查找一台具体设备(一次性);
//!   3. [`Scanner::drive_watch_stream`][]:**持续**按指定节奏扫描,把每轮结果通过
//!      `mpsc::Sender` 推给下游 —— 解耦"采集"与"消费"。
//!
//! 设计边界:扫描器**不知道**锁屏阈值、不维护连续丢失计数、不引用 Monitor。
//! 它的输出永远是"这一窗口扫到了什么",而非"应不应该锁屏"。

use std::time::Duration;

use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager};
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};

use super::device::{DeviceInfo, DeviceMatcher};

/// 扫描过程中可能出错的几种情形。
#[derive(Debug)]
pub enum ScannerError {
    /// 系统找不到任何蓝牙适配器(蓝牙被关 / 权限没给 / 硬件缺失)。
    NoAdapter,
    /// btleplug 抛出的底层错误,原样透传以便诊断。
    Btle(btleplug::Error),
}

impl std::fmt::Display for ScannerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAdapter => write!(f, "找不到蓝牙适配器(是否未启用蓝牙?)"),
            Self::Btle(e) => write!(f, "蓝牙底层错误: {e}"),
        }
    }
}

impl std::error::Error for ScannerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoAdapter => None,
            Self::Btle(e) => Some(e),
        }
    }
}

impl From<btleplug::Error> for ScannerError {
    fn from(e: btleplug::Error) -> Self {
        Self::Btle(e)
    }
}

/// BLE 扫描器。持有系统蓝牙适配器,可被多次复用,无需每次扫描都重建。
pub struct Scanner {
    adapter: Adapter,
}

impl Scanner {
    /// 初始化:接上系统蓝牙栈,取第一个可用适配器。
    /// macOS 首次调用会触发"允许蓝牙访问"权限弹窗。
    pub async fn new() -> Result<Self, ScannerError> {
        let manager = Manager::new().await?;
        let adapter = manager
            .adapters()
            .await?
            .into_iter()
            .next()
            .ok_or(ScannerError::NoAdapter)?;
        Ok(Self { adapter })
    }

    /// 扫描指定时长,返回这段窗口内累计看到的所有可读外设。
    pub async fn scan_all(&self, duration: Duration) -> Result<Vec<DeviceInfo>, ScannerError> {
        self.adapter.start_scan(ScanFilter::default()).await?;
        tokio::time::sleep(duration).await;

        let peripherals = self.adapter.peripherals().await?;
        let _ = self.adapter.stop_scan().await;

        let mut out = Vec::with_capacity(peripherals.len());
        for p in peripherals {
            if let Ok(Some(props)) = p.properties().await {
                out.push(DeviceInfo {
                    id: p.id().to_string(),
                    address: props.address.to_string(),
                    local_name: props.local_name,
                    rssi: props.rssi,
                });
            }
        }
        Ok(out)
    }

    /// 扫描并按 `matcher` 查找一台具体设备。
    ///
    /// 三种返回都不是错,上层要区别对待:
    /// - `Ok(Some(info))` —— 扫到了目标;
    /// - `Ok(None)`       —— 扫描成功但没看到目标 —— 对应 `Monitor::update(None)`;
    /// - `Err(_)`         —— 扫描本身出问题,**不要**当成"丢失"喂给 Monitor。
    pub async fn find(
        &self,
        matcher: &DeviceMatcher,
        duration: Duration,
    ) -> Result<Option<DeviceInfo>, ScannerError> {
        let all = self.scan_all(duration).await?;
        Ok(all.into_iter().find(|info| matcher.matches(info)))
    }

    /// **持续生产者** —— 消费 `Scanner` 本身,以 `tick_interval` 为节拍反复调 `find()`,
    /// 把每轮结果推进 `tx`。
    ///
    /// 参数关系:`scan_window` **必须 < tick_interval**,否则 tokio interval 会因
    /// `MissedTickBehavior::Skip` 丢拍。
    ///
    /// 终止条件:`tx` 关闭(消费者 drop receiver)时立即返回 —— 协作式取消。
    pub async fn drive_watch_stream(
        self,
        matcher: DeviceMatcher,
        tick_interval: Duration,
        scan_window: Duration,
        tx: mpsc::Sender<Result<Option<DeviceInfo>, ScannerError>>,
    ) {
        let mut ticker = interval(tick_interval);
        // Skip 而非 Burst:落后了直接对齐到下一拍,不补帧。
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            let result = self.find(&matcher, scan_window).await;
            if tx.send(result).await.is_err() {
                return;
            }
        }
    }
}
