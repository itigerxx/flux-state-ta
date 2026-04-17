use crate::core::indicator::Indicator;
use crate::data::candle::Candle;
use crate::utils::candle_check_context::CandleCheckContext;
use crate::window::indicator_series::IndicatorSeries;

/// ======================================================
/// EMA（Exponential Moving Average）
/// ======================================================
///
/// ✔ 核心思想：
/// EMA 是一种“递归加权平均”，越新的数据权重越大
///
/// ======================================================
/// 数学公式：
/// ======================================================
///
/// alpha = 2 / (period + 1)
///
/// EMA_t = alpha * price_t + (1.0 - alpha) * EMA_{t-1}
#[derive(Debug)]
pub struct EMA {
    /// EMA周期
    period: usize,

    /// 平滑系数 alpha
    alpha: f64,

    /// 当前 EMA 状态值（核心状态 - ❗严格仅存储已确认收盘的值，严禁被 preview 污染）
    ema: Option<f64>,

    /// 是否已经完成 warmup
    ready: bool,

    /// 用于 warmup 期间累积初始均值（SMA初始化）
    init_window: IndicatorSeries<f64>,

    /// 统一K线校验上下文
    ctx: CandleCheckContext,

    /// 结果窗口（Hybrid Series: [0..n-1] 是确认历史, [n] 是实时预览）
    results: IndicatorSeries<f64>,

    /// ❗新字段：用于跟踪预览状态的时间戳，实现 Slot 分离
    preview_bar_time: Option<u64>,
}

impl EMA {
    /// ======================================================
    /// 创建 EMA 实例
    /// ======================================================
    pub fn new(period: usize, results_capacity: usize) -> Self {
        assert!(period > 0);
        assert!(results_capacity > 0);

        Self {
            period,
            alpha: 2.0 / (period as f64 + 1.0),
            ema: None,
            ready: false,
            init_window: IndicatorSeries::new(period),
            ctx: CandleCheckContext::default(),
            results: IndicatorSeries::new(results_capacity),
            preview_bar_time: None,
        }
    }

    /// 统一写入逻辑：基于时间戳幂等占位
    fn write_result(&mut self, bar_time: u64, value: f64) {
        match self.preview_bar_time {
            Some(t) if t == bar_time => {
                self.results.update_latest(value);
            }
            _ => {
                self.results.push(value);
                self.preview_bar_time = Some(bar_time);
            }
        }
    }
}

impl Indicator for EMA {
    type Output = f64;

    /// ======================================================
    /// update：EMA 核心更新逻辑
    /// ======================================================
    fn update(&mut self, candle: Candle) {
        // =========================
        // 1. 统一数据校验
        // =========================
        if !self.ctx.validate(&candle) {
            return;
        }

        let price = candle.close;
        let bar_time = candle.open_time;

        // ======================================================
        // 2. warmup 阶段（SMA 初始化 EMA）
        // ======================================================
        if !self.ready {
            if !candle.closed {
                return;
            }

            self.init_window.push(price);

            let len = self.init_window.len();

            if len < self.period {
                return;
            }

            let mut sum = 0.0;
            for i in 0..len {
                if let Some(v) = self.init_window.get(i) {
                    sum += *v;
                }
            }

            let sma = sum / len as f64;

            self.ema = Some(sma);
            self.ready = true;
            
            // 使用统一写入逻辑
            self.write_result(bar_time, sma);

            self.init_window = IndicatorSeries::new(self.period);

            return;
        }

        // ======================================================
        // 3. EMA 正常递推阶段（核心）
        // ======================================================
        if let Some(prev_confirmed_ema) = self.ema {

            // ======================================================
            // 【分流一：CLOSED PATH】
            // ======================================================
            if candle.closed {
                let new_confirmed_ema =
                    self.alpha * price + (1.0 - self.alpha) * prev_confirmed_ema;

                // 更新核心状态
                self.ema = Some(new_confirmed_ema);

                // 使用统一写入逻辑（不显式置 None，交给时间戳判断）
                self.write_result(bar_time, new_confirmed_ema);

                return;
            }

            // ======================================================
            // 【分流二：PREVIEW PATH】
            // ======================================================
            let preview_ema =
                self.alpha * price + (1.0 - self.alpha) * prev_confirmed_ema;

            // 使用统一写入逻辑
            self.write_result(bar_time, preview_ema);
        }
    }

    fn latest(&self) -> Option<Self::Output> {
        if !self.ready {
            return None;
        }

        self.results.last().copied()
    }

    fn last_n(&self, n: usize) -> Vec<Self::Output> {
        self.results.tail(n).into_iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::candle::Candle;

    fn create_candle(open_time: u64, close: f64, closed: bool) -> Candle {
        Candle {
            open_time,
            close,
            closed,
            ..Default::default()
        }
    }

    #[test]
    fn test_ema_warmup_and_first_value() {
        let period = 3;
        let mut ema = EMA::new(period, 1000);

        // 1. 前两根收盘，不应 ready
        ema.update(create_candle(1000, 10.0, true));
        ema.update(create_candle(2000, 20.0, true));
        assert!(!ema.ready);
        assert!(ema.latest().is_none());

        // 2. 第三根收盘，SMA 初始化：(10 + 20 + 30) / 3 = 20.0
        ema.update(create_candle(3000, 30.0, true));
        assert!(ema.ready);
        assert_eq!(ema.latest().unwrap(), 20.0);
    }

    #[test]
    fn test_ema_recursion_accuracy() {
        let period = 3; // alpha = 2 / (3 + 1) = 0.5
        let mut ema = EMA::new(period, 1000);

        // 初始化到 20.0
        ema.update(create_candle(1000, 10.0, true));
        ema.update(create_candle(2000, 20.0, true));
        ema.update(create_candle(3000, 30.0, true));

        // 第四根收盘价格为 40.0
        // EMA = 0.5 * 40.0 + (1 - 0.5) * 20.0 = 20.0 + 10.0 = 30.0
        ema.update(create_candle(4000, 40.0, true));
        assert_eq!(ema.latest().unwrap(), 30.0);

        // 第五根收盘价格为 10.0
        // EMA = 0.5 * 10.0 + (1 - 0.5) * 30.0 = 5.0 + 15.0 = 20.0
        ema.update(create_candle(5000, 10.0, true));
        assert_eq!(ema.latest().unwrap(), 20.0);
    }

    #[test]
    fn test_ema_preview_isolation() {
        let period = 3; // alpha = 0.5
        let mut ema = EMA::new(period, 1000);

        // 初始化到 20.0
        for i in 1..=3 {
            ema.update(create_candle(i * 1000, i as f64 * 10.0, true));
        }
        let confirmed_ema = ema.latest().unwrap(); // 20.0

        // 预览一根极其夸张的价格 (1000.0)
        ema.update(create_candle(4000, 1000.0, false));
        let preview_val = ema.latest().unwrap();
        assert!(preview_val > confirmed_ema); // 预览值应该反映 1000.0 的影响

        // 再次发送同一根 Bar 的预览，价格变动
        ema.update(create_candle(4000, 40.0, false));
        let second_preview = ema.latest().unwrap();
        // 应基于原始 20.0 计算：0.5 * 40 + 0.5 * 20 = 30.0
        assert_eq!(second_preview, 30.0);

        // 重点：此时发送一根全新的 Bar (5000) 且直接收盘，价格 10.0
        ema.update(create_candle(5000, 10.0, true));
        // EMA_5000 = 0.5 * 10.0 + 0.5 * 20.0 = 15.0
        assert_eq!(ema.latest().unwrap(), 15.0);
    }

    #[test]
    fn test_ema_last_n() {
        let mut ema = EMA::new(3, 1000);
        for i in 1..=5 {
            ema.update(create_candle(i * 1000, i as f64 * 10.0, true));
        }
        
        let tail = ema.last_n(2);
        assert_eq!(tail.len(), 2);
        // 基于递推：EMA3=20.0, EMA4=30.0, EMA5=40.0
        assert_eq!(tail[0], 30.0);
        assert_eq!(tail[1], 40.0);
    }
}