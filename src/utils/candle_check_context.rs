use crate::data::candle::Candle;

/// ======================================================
/// CandleCheckContext
/// ======================================================
///
/// ✔ 负责所有 Candle 一致性校验状态
/// ✔ 不参与任何指标计算
/// ✔ 可被所有指标复用（SMA/EMA/RSI）
///
/// ======================================================
/// 职责边界：
/// ======================================================
///
/// 只做三件事：
///
/// 1. 保证 open_time 顺序
/// 2. 保证 event_time 顺序
/// 3. 防止重复 closed candle（幂等）
#[derive(Debug, Default)]
pub struct CandleCheckContext {
    /// 上一次 open_time（保证顺序）
    pub last_open_time: Option<u64>,

    /// 上一次 event_time（保证顺序）
    pub last_event_time: Option<u64>,

    /// 已处理的 closed candle（幂等控制）
    pub last_closed_open_time: Option<u64>,
}

impl CandleCheckContext {
    /// ======================================================
    /// validate：统一 Candle 校验入口
    /// ======================================================
    ///
    /// 返回：
    /// - true  = 允许进入指标计算
    /// - false = 丢弃（乱序 / 重复）
    #[inline]
    pub fn validate(&mut self, candle: &Candle) -> bool {
        // =========================
        // 1. 顺序校验（open_time）
        // =========================
        if let Some(last) = self.last_open_time {
            if candle.open_time < last {
                return false;
            }
        }

        // =========================
        // 2. 顺序校验（event_time）
        // =========================
        if let Some(last) = self.last_event_time {
            if candle.event_time < last {
                return false;
            }
        }

        // 更新顺序状态
        self.last_open_time = Some(candle.open_time);
        self.last_event_time = Some(candle.event_time);

        // =========================
        // 3. closed 幂等性
        // =========================
        if candle.closed {
            if self.last_closed_open_time == Some(candle.open_time) {
                return false;
            }

            self.last_closed_open_time = Some(candle.open_time);
        }

        true
    }
}