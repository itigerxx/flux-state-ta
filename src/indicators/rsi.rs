use crate::core::indicator::Indicator;
use crate::data::candle::Candle;
use crate::utils::candle_check_context::CandleCheckContext;
use crate::window::indicator_series::IndicatorSeries;

/// ======================================================
/// RSI（Relative Strength Index）
/// ======================================================
///
/// 设计目标：
/// ✔ 实时性：支持 Streaming Tick 更新，latest() 始终返回当前最新计算值。
/// ✔ 安全性：closed 才提交真实状态，preview 仅用于实时展示。
///
/// ------------------------------------------------------
/// RSI 核心公式（Wilder 平滑法）
/// ------------------------------------------------------
///
/// change = close_t - close_{t-1}
///
/// gain = max(change, 0)
/// loss = max(-change, 0)
///
/// avg_gain = (prev_avg_gain * (period - 1) + gain) / period
/// avg_loss = (prev_avg_loss * (period - 1) + loss) / period
///
/// rs  = avg_gain / avg_loss
/// rsi = 100 - 100 / (1 + rs)
///
#[derive(Debug)]
pub struct RSI {
    /// RSI周期
    period: usize,

    /// 已确认状态：平均上涨幅度
    avg_gain: Option<f64>,

    /// 已确认状态：平均下跌幅度
    avg_loss: Option<f64>,

    /// 上一根已确认 close（用于计算涨跌）
    prev_close: Option<f64>,

    /// warmup 阶段累计 gain
    init_gain_sum: f64,

    /// warmup 阶段累计 loss
    init_loss_sum: f64,

    /// 当前已累计了多少次 change（注意不是 candle 数）
    init_count: usize,

    /// 是否已完成初始化
    ready: bool,

    /// 输入数据校验器
    ctx: CandleCheckContext,

    /// 输出结果序列
    results: IndicatorSeries<f64>,

    /// 当前预览 bar 的时间戳
    preview_bar_time: Option<u64>,
}

impl RSI {
    /// ======================================================
    /// 创建 RSI 实例
    /// ======================================================
    pub fn new(period: usize, results_capacity: usize) -> Self {
        assert!(period > 0);
        assert!(results_capacity > 0);

        Self {
            period,
            avg_gain: None,
            avg_loss: None,
            prev_close: None,
            init_gain_sum: 0.0,
            init_loss_sum: 0.0,
            init_count: 0,
            ready: false,
            ctx: CandleCheckContext::default(),
            results: IndicatorSeries::new(results_capacity),
            preview_bar_time: None,
        }
    }

    /// ======================================================
    /// 根据 avg_gain / avg_loss 计算 RSI
    /// ======================================================
    fn calc_rsi(avg_gain: f64, avg_loss: f64) -> f64 {
        // 极端情况：没有波动
        if avg_gain == 0.0 && avg_loss == 0.0 {
            return 50.0;
        }

        // 只有上涨，没有下跌
        if avg_loss == 0.0 {
            return 100.0;
        }

        // 只有下跌，没有上涨
        if avg_gain == 0.0 {
            return 0.0;
        }

        let rs = avg_gain / avg_loss;
        100.0 - (100.0 / (1.0 + rs))
    }
}

impl Indicator for RSI {
    type Output = f64;

    /// ======================================================
    /// update：RSI 核心更新逻辑
    /// ======================================================
    fn update(&mut self, candle: Candle) {
        // ==================================================
        // 1. 输入校验（乱序 / 重复数据直接丢弃）
        // ==================================================
        if !self.ctx.validate(&candle) {
            return;
        }

        let price = candle.close;
        let bar_time = candle.open_time;

        // ==================================================
        // 2. 第一根数据：仅记录 prev_close
        // ==================================================
        if self.prev_close.is_none() {
            if candle.closed {
                self.prev_close = Some(price);
            }
            return;
        }

        let prev_close = self.prev_close.unwrap();
        let change = price - prev_close;
        let gain = change.max(0.0);
        let loss = (-change).max(0.0);

        // ==================================================
        // 3. warmup 阶段（先凑够 period 次变动）
        // ==================================================
        if !self.ready {
            // warmup 只接受 closed 数据
            if !candle.closed {
                return;
            }

            self.init_gain_sum += gain;
            self.init_loss_sum += loss;
            self.init_count += 1;
            self.prev_close = Some(price);

            // 还没达到初始化条件
            if self.init_count < self.period {
                return;
            }

            // 初始化平均涨跌幅
            let avg_gain = self.init_gain_sum / self.period as f64;
            let avg_loss = self.init_loss_sum / self.period as f64;

            self.avg_gain = Some(avg_gain);
            self.avg_loss = Some(avg_loss);
            self.ready = true;

            let rsi = Self::calc_rsi(avg_gain, avg_loss);
            self.results.push(rsi);

            return;
        }

        // ==================================================
        // 4. 正常阶段（实时 + closed 双模式）
        // ==================================================
        let prev_avg_gain = self.avg_gain.unwrap();
        let prev_avg_loss = self.avg_loss.unwrap();

        // 使用 Wilder 平滑公式计算本次结果
        let next_avg_gain =
            (prev_avg_gain * (self.period as f64 - 1.0) + gain) / self.period as f64;

        let next_avg_loss =
            (prev_avg_loss * (self.period as f64 - 1.0) + loss) / self.period as f64;

        let current_rsi = Self::calc_rsi(next_avg_gain, next_avg_loss);

        // ==================================================
        // 5. 写入结果序列（统一状态机）
        // ==================================================
        match self.preview_bar_time {
            Some(t) if t == bar_time => {
                self.results.update_latest(current_rsi);
            }
            _ => {
                self.results.push(current_rsi);
                self.preview_bar_time = Some(bar_time);
            }
        }

        // ==================================================
        // 6. closed：提交真实状态
        // ==================================================
        if candle.closed {
            self.avg_gain = Some(next_avg_gain);
            self.avg_loss = Some(next_avg_loss);
            self.prev_close = Some(price);
            // 清除预览标记，确保下一根 Bar 触发 push
            self.preview_bar_time = None;
        }
    }

    /// ======================================================
    /// latest：获取当前最新 RSI（实时值）
    /// ======================================================
    fn latest(&self) -> Option<Self::Output> {
        if !self.ready {
            return None;
        }

        self.results.last().copied()
    }

    /// ======================================================
    /// last_n：获取最近 N 个 RSI
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
    fn test_rsi_warmup_and_calculation() {
        // 使用周期 3 进行测试
        let mut rsi = RSI::new(3, 1000);

        // 1. 第 1 根：设置初始价格，无 RSI 输出
        rsi.update(create_candle(1000, 100.0, true));
        assert!(!rsi.ready);

        // 2. Warmup 阶段：需要 period(3) 次变动
        // 变动 1: +10
        rsi.update(create_candle(2000, 110.0, true));
        // 变动 2: -5
        rsi.update(create_candle(3000, 105.0, true));
        assert!(!rsi.ready);

        // 变动 3: +15 (此时达到 3 次变动)
        rsi.update(create_candle(4000, 120.0, true));
        assert!(rsi.ready);

        // 计算第一个 RSI:
        // sum_gain = 10 + 0 + 15 = 25 -> avg_gain = 25 / 3 = 8.3333...
        // sum_loss = 0 + 5 + 0 = 5  -> avg_loss = 5 / 3 = 1.6666...
        // rs = 8.333 / 1.666 = 5
        // rsi = 100 - 100/(1+5) = 83.333...
        let first_rsi = rsi.latest().unwrap();
        assert_approx(first_rsi, 83.33333333333333);
    }

    #[test]
    fn test_rsi_streaming_preview_and_isolation() {
        let mut rsi = RSI::new(3, 1000);
        
        // 先填充数据完成初始化 (100 -> 110 -> 105 -> 120)
        rsi.update(create_candle(1000, 100.0, true));
        rsi.update(create_candle(2000, 110.0, true));
        rsi.update(create_candle(3000, 105.0, true));
        rsi.update(create_candle(4000, 120.0, true));
        
        let base_avg_gain = rsi.avg_gain.unwrap(); // 8.333...

        // --------------------------------------------------
        // 3. 预览逻辑验证 (Preview)
        // --------------------------------------------------
        let bar_time = 5000;
        
        // Tick 1: 价格继续上涨 (Preview)
        // change = 130 - 120 = 10
        // next_avg_gain = (8.333 * 2 + 10) / 3 = 8.888...
        // next_avg_loss = (1.666 * 2 + 0) / 3 = 1.111...
        rsi.update(create_candle(bar_time, 130.0, false));
        let p1 = rsi.latest().unwrap();
        assert_eq!(rsi.last_n(10).len(), 2); // 1个确认的 + 1个预览的
        assert!(p1 > 83.33);

        // Tick 2: 价格瞬间跳水 (同一根 Bar, Preview)
        // change = 90 - 120 = -30 -> loss = 30
        // next_avg_gain = (8.333 * 2 + 0) / 3 = 5.555...
        // next_avg_loss = (1.666 * 2 + 30) / 3 = 11.111...
        rsi.update(create_candle(bar_time, 90.0, false));
        let p2 = rsi.latest().unwrap();
        assert_eq!(rsi.last_n(10).len(), 2); // 长度不应增加
        assert!(p2 < p1);

        // 核心验证：预览不应污染内部状态
        // 即使 Tick 2 计算时使用了 p1 之前的 base 状态，它也不应该被 p1 的计算所影响
        assert_approx(rsi.avg_gain.unwrap(), base_avg_gain);

        // --------------------------------------------------
        // 4. 正式确认验证 (Confirmed)
        // --------------------------------------------------
        rsi.update(create_candle(bar_time, 90.0, true));
        assert_eq!(rsi.last_n(10).len(), 2);
        assert_approx(rsi.latest().unwrap(), p2);
        
        // 内部状态更新
        assert_ne!(rsi.avg_gain.unwrap(), base_avg_gain);
        assert!(rsi.preview_bar_time.is_none());
    }

    #[test]
    fn test_rsi_extreme_scenarios() {
        let mut rsi = RSI::new(3, 1000);
        rsi.update(create_candle(1000, 100.0, true));
        
        // 场景 A: 全力上涨 (avg_loss 为 0)
        rsi.update(create_candle(2000, 110.0, true));
        rsi.update(create_candle(3000, 120.0, true));
        rsi.update(create_candle(4000, 130.0, true));
        assert_approx(rsi.latest().unwrap(), 100.0);

        // 场景 B: 横盘不动 (avg_gain & avg_loss 均为 0)
        let mut rsi_flat = RSI::new(3, 1000);
        rsi_flat.update(create_candle(1000, 100.0, true));
        rsi_flat.update(create_candle(2000, 100.0, true));
        rsi_flat.update(create_candle(3000, 100.0, true));
        rsi_flat.update(create_candle(4000, 100.0, true));
        assert_approx(rsi_flat.latest().unwrap(), 50.0);
    }

    #[test]
    fn test_rsi_idempotency_check() {
        let mut rsi = RSI::new(3, 1000);
        rsi.update(create_candle(1000, 100.0, true));
        rsi.update(create_candle(2000, 110.0, true));
        rsi.update(create_candle(3000, 105.0, true));
        rsi.update(create_candle(4000, 120.0, true));
        
        let initial_len = rsi.last_n(100).len();
        
        // 模拟重复发送已确认的相同 K 线
        rsi.update(create_candle(4000, 120.0, true));
        assert_eq!(rsi.last_n(100).len(), initial_len, "Duplicate candle should be ignored");
    }
}