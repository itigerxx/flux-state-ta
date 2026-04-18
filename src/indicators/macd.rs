use crate::core::indicator::Indicator;
use crate::data::candle::Candle;
use crate::utils::candle_check_context::CandleCheckContext;
use crate::indicators::ema::EMA;
use crate::window::indicator_series::IndicatorSeries;

/// ======================================================
/// MACD Output (DIF, DEA, HIST)
/// ======================================================
#[derive(Debug, Clone, Copy, Default)]
pub struct MacdOutput {
    pub dif: f64,
    pub dea: f64,
    pub hist: f64,
}

/// ======================================================
/// MACD (Moving Average Convergence Divergence)
/// ======================================================
///
/// 设计目标：
/// ✔ 实时性：支持 Streaming Tick 更新，latest() 始终返回当前最新计算值。
/// ✔ 安全性：区分 Preview (未闭合) 与 Confirmed (已闭合)，保护历史数据不被污染。
/// ✔ 鲁棒性：基于 open_time 锚点，自动处理跨 Bar 异常及重复推送。
///
#[derive(Debug)]
pub struct MACD {
    // 底层指标组件
    fast_ema: EMA,
    slow_ema: EMA,
    signal_ema: EMA,

    // 输入校验上下文
    ctx: CandleCheckContext,

    // 结果序列窗口 (Hybrid: [0..n-1] 确定, [n] 预览)
    results: IndicatorSeries<MacdOutput>,

    // 当前处于预览状态的 Bar 开盘时间戳
    preview_bar_time: Option<u64>,

    // 指标是否已完成 Warmup
    ready: bool,
}

impl MACD {
    pub fn new(fast: usize, slow: usize, signal: usize, results_capacity: usize) -> Self {
        assert!(fast > 0 && slow > 0 && signal > 0);
        assert!(fast < slow, "Fast period must be less than slow period");
        assert!(results_capacity > 0);

        // 内部计算只需要 2 个槽位：1个确定的，1个预览的
        let internal_cap = 2;

        Self {
            fast_ema: EMA::new(fast, internal_cap),
            slow_ema: EMA::new(slow, internal_cap),
            signal_ema: EMA::new(signal, internal_cap),
            ctx: CandleCheckContext::default(),
            results: IndicatorSeries::new(results_capacity),
            preview_bar_time: None,
            ready: false,
        }
    }

    /// ======================================================
    /// 核心状态机：管理结果写入
    /// ======================================================
    fn write_result(&mut self, candle: &Candle, value: MacdOutput) {
        let bar_time = candle.open_time;

        match self.preview_bar_time {
            // 同一根 Bar 更新
            Some(t) if t == bar_time => {
                self.results.update_latest(value);
            }
            // 无预览态 或 跨 Bar
            _ => {
                self.results.push(value);
                self.preview_bar_time = Some(bar_time);
            }
        }
    }
}

impl Indicator for MACD {
    type Output = MacdOutput;

    /// ======================================================
    /// update：MACD 级联计算逻辑
    /// ======================================================
    fn update(&mut self, candle: &Candle) {
        // 1. 数据校验 (防止时间倒流或重复数据)
        if !self.ctx.validate(&candle) {
            return;
        }

        let is_closed = candle.closed;

        // 2. 更新基础 EMA (EMA 内部已实现预览隔离逻辑)
        self.fast_ema.update(candle);
        self.slow_ema.update(candle);

        // 3. 提取 EMA 实时值进行计算
        if let (Some(f), Some(s)) = (self.fast_ema.latest(), self.slow_ema.latest()) {
            let dif = f - s;

            // 4. 计算信号线 (信号线的输入是 DIF)
            let signal_input = Candle {
                close: dif,
                closed: is_closed,
                ..candle.clone()
            };
            self.signal_ema.update(&signal_input);

            if let Some(dea) = self.signal_ema.latest() {
                let hist = (dif - dea) * 2.0;
                let output = MacdOutput { dif, dea, hist };

                // 5. 标记 Ready (仅在首次全链路打通时触发)
                if !self.ready {
                    self.ready = true;
                }

                // 6. 写入/更新结果序列
                self.write_result(&candle, output);
            }
        }
    }

    /// ======================================================
    /// latest：获取当前最新指标值 (包含实时预览)
    /// ======================================================
    fn latest(&self) -> Option<Self::Output> {
        if !self.ready {
            return None;
        }
        self.results.last().copied()
    }

    /// ======================================================
    /// last_n：获取历史指标序列 (末尾值可能随 Tick 变动)
    /// ======================================================
    fn last_n(&self, n: usize) -> Vec<Self::Output> {
        if !self.ready {
            return Vec::new();
        }
        self.results.tail(n).into_iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 辅助工具：快速创建 Candle
    fn create_candle(time: u64, price: f64, closed: bool) -> Candle {
        Candle {
            open_time: time,
            close: price,
            high: price,
            low: price,
            closed,
            ..Default::default()
        }
    }

    /// 辅助工具：浮点数近似相等判断
    fn assert_approx(actual: f64, expected: f64) {
        let precision = 1e-10;
        assert!(
            (actual - expected).abs() <= precision,
            "Expected {}, got {}",
            expected,
            actual
        );
    }

    #[test]
    fn test_macd_initialization_and_warmup() {
        // 设置一个极短的周期以便测试: Fast(2), Slow(3), Signal(2)
        let mut macd = MACD::new(2, 3, 2, 1000);

        // 第 1-2 根 Bar
        macd.update(&create_candle(1000, 10.0, true));
        macd.update(&create_candle(2000, 11.0, true));
        assert!(!macd.ready, "MACD should not be ready during initial EMA warmup");

        // 第 3 根 Bar: Slow EMA(3) 此时会有值，DIF 产生，Signal EMA 开始 warmup
        macd.update(&create_candle(3000, 12.0, true));
        
        // 第 4 根 Bar: Signal EMA(2) 此时达到周期，全链路打通
        macd.update(&create_candle(4000, 13.0, true));
        assert!(macd.ready, "MACD should be ready after all cascaded EMAs warmup");
        assert!(macd.latest().is_some());
    }

    #[test]
    fn test_macd_streaming_preview_logic() {
        let mut macd = MACD::new(12, 26, 9, 1000);
        
        // 1. 快速填充数据使指标进入 Ready 状态
        for i in 0..50 {
            macd.update(&create_candle(i * 60, 100.0 + i as f64, true));
        }
        assert!(macd.ready);
        let initial_len = macd.last_n(1000).len();

        // 2. 模拟新 Bar 的第一个实时 Tick (Preview)
        let bar_time = 50 * 60;
        macd.update(&create_candle(bar_time, 160.0, false));
        
        assert_eq!(macd.last_n(1000).len(), initial_len + 1, "Should push a new position for preview");
        let p1 = macd.latest().unwrap();

        // 3. 模拟同一根 Bar 的第二个实时 Tick (价格剧烈波动)
        macd.update(&create_candle(bar_time, 180.0, false));
        assert_eq!(macd.last_n(1000).len(), initial_len + 1, "Should NOT push new position for same bar");
        
        let p2 = macd.latest().unwrap();
        assert_ne!(p1.dif, p2.dif, "DIF should update in real-time");
        assert_ne!(p1.dea, p2.dea, "DEA should update in real-time");
        assert_ne!(p1.hist, p2.hist, "HIST should update in real-time");

        // 4. 模拟该 Bar 正式收盘 (Confirmed)
        macd.update(&create_candle(bar_time, 180.0, true));
        assert_eq!(macd.last_n(1000).len(), initial_len + 1, "Should maintain same position after confirmed");
        
        // 收盘后的值应与最后一个预览值一致 (在价格一致的前提下)
        let confirmed = macd.latest().unwrap();
        assert_approx(confirmed.dif, p2.dif);

        // 5. 跨 Bar 验证：下一根 Bar 的第一个 Tick 应该再次触发 push
        macd.update(&create_candle(bar_time + 60, 185.0, false));
        assert_eq!(macd.last_n(1000).len(), initial_len + 2, "New bar should push a new preview position");
    }

    #[test]
    fn test_macd_state_isolation() {
        let mut macd = MACD::new(12, 26, 9, 1000);
        for i in 0..40 {
            macd.update(&create_candle(i * 60, 100.0, true));
        }

        // 记录收盘状态下的内部 DEA
        let confirmed_dea = macd.signal_ema.latest().unwrap();

        // 注入一个极端的预览值
        macd.update(&create_candle(10000, 9999.0, false));
        assert_ne!(macd.signal_ema.latest().unwrap(), confirmed_dea);

        // 再次注入另一个预览值（同一根 Bar），验证它是基于 confirmed_dea 还是上一个预览值
        // EMA 的特性是基于前值计算，如果预览污染了状态，连续两次 update(false) 会导致错误累积
        macd.update(&create_candle(10000, 100.0, false));
        
        // 此时价格回到了收盘价，预览值应该重新回到接近 confirmed_dea 的水平
        // 如果内部状态被污染了，这里会算出一个基于 9999.0 的错误值
        assert_approx(macd.signal_ema.latest().unwrap(), confirmed_dea);
    }

    #[test]
    fn test_macd_idempotency_and_robustness() {
        let mut macd = MACD::new(12, 26, 9, 1000);
        
        // 模拟完全持平的市场
        for i in 0..50 {
            macd.update(&create_candle(i * 60, 100.0, true));
        }
        
        let out = macd.latest().unwrap();
        // 持平市场下，DIF 和 DEA 应该趋近于 0，HIST 也为 0
        assert_approx(out.dif, 0.0);
        assert_approx(out.dea, 0.0);
        assert_approx(out.hist, 0.0);

        // 模拟重复的收盘信号 (由网络重试等引起)
        let last_time = 49 * 60;
        let len_before = macd.last_n(1000).len();
        macd.update(&create_candle(last_time, 100.0, true));
        
        assert_eq!(macd.last_n(1000).len(), len_before, "Duplicate closed signal should be idempotent");
    }
}
