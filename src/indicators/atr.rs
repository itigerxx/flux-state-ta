use crate::core::indicator::Indicator;
use crate::data::candle::Candle;
use crate::utils::candle_check_context::CandleCheckContext;
use crate::window::indicator_series::IndicatorSeries;

/// ======================================================
/// ATR（Average True Range）
/// ======================================================
///
/// ATR 是波动率指标，用于衡量价格波动幅度，不判断方向。
///
/// 输出值：
/// atr = 当前平均真实波幅
///
#[derive(Debug)]
pub struct ATR {
    /// 周期
    period: usize,

    /// 是否已可输出
    ready: bool,

    /// 输入校验器
    ctx: CandleCheckContext,

    /// 输出结果缓存
    results: IndicatorSeries<f64>,

    /// 当前预览中的 Bar 时间戳
    preview_bar_time: Option<u64>,

    // ==================================================
    // 上一根已确认K线数据（用于计算 TR）
    // ==================================================
    prev_close: Option<f64>,

    // ==================================================
    // Wilder 平滑 ATR 状态（仅保存 confirmed）
    // ==================================================
    atr: Option<f64>,

    // ==================================================
    // 初始化累计区
    // ==================================================
    warmup_count: usize,
    tr_sum: f64,
}

impl ATR {
    /// ======================================================
    /// 创建实例
    /// ======================================================
    pub fn new(period: usize, results_capacity: usize) -> Self {
        assert!(period > 0);
        assert!(results_capacity > 0);

        Self {
            period,
            ready: false,
            ctx: CandleCheckContext::default(),
            results: IndicatorSeries::new(results_capacity),
            preview_bar_time: None,

            prev_close: None,
            atr: None,

            warmup_count: 0,
            tr_sum: 0.0,
        }
    }

    /// ======================================================
    /// 计算 True Range
    /// ======================================================
    ///
    /// TR = max(
    ///     high - low,
    ///     abs(high - prev_close),
    ///     abs(low - prev_close)
    /// )
    ///
    fn calc_tr(&self, high: f64, low: f64, prev_close: f64) -> f64 {
        let tr1 = high - low;
        let tr2 = (high - prev_close).abs();
        let tr3 = (low - prev_close).abs();

        tr1.max(tr2).max(tr3)
    }

    /// ======================================================
    /// 写入结果（统一状态机）
    /// ======================================================
fn write_result(&mut self, candle: &Candle, value: f64) {
    let bar_time = candle.open_time;

    match self.preview_bar_time {
        // 身份证号没变：永远只是对当前坑位的“修正”或“完善”
        Some(t) if t == bar_time => {
            self.results.update_latest(value);
        }
        // 身份证号变了（跨 Bar）或初始化：新开一个坑位
        _ => {
            self.results.push(value);
            self.preview_bar_time = Some(bar_time);
        }
    }
}
}

impl Indicator for ATR {
    type Output = f64;

    /// ======================================================
    /// update：核心更新逻辑
    /// ======================================================
    fn update(&mut self, candle: Candle) {
        // ==================================================
        // 1. 数据校验
        // ==================================================
        if !self.ctx.validate(&candle) {
            return;
        }

        let high = candle.high;
        let low = candle.low;
        let close = candle.close;
        let is_closed = candle.closed;

        // ==================================================
        // 2. 第一根K线：仅建立 prev_close 基准
        // ==================================================
        if self.prev_close.is_none() {
            if is_closed {
                self.prev_close = Some(close);
            }
            return;
        }

        let prev_close = self.prev_close.unwrap();
        let tr = self.calc_tr(high, low, prev_close);

        // ==================================================
        // 3. 初始化阶段：累计 period 个 TR
        // ==================================================
        if self.atr.is_none() {
            if !is_closed {
                return;
            }

            self.tr_sum += tr;
            self.warmup_count += 1;
            self.prev_close = Some(close);

            if self.warmup_count < self.period {
                return;
            }

            let first_atr = self.tr_sum / self.period as f64;

            self.atr = Some(first_atr);
            self.ready = true;

            self.write_result(&candle, first_atr);
            return;
        }

        // ==================================================
        // 4. 正式阶段：实时预览 + 收盘提交
        // ==================================================
        let base_atr = self.atr.unwrap();

        // Wilder 平滑：
        // ATR = ((prev_atr * (n - 1)) + tr) / n
        let next_atr = ((base_atr * (self.period as f64 - 1.0)) + tr) / self.period as f64;

        // 实时输出
        self.write_result(&candle, next_atr);

        // 收盘提交正式状态
        if is_closed {
            self.atr = Some(next_atr);
            self.prev_close = Some(close);
        }
    }

    /// ======================================================
    /// latest：当前最新值（实时）
    /// ======================================================
    fn latest(&self) -> Option<Self::Output> {
        if !self.ready {
            return None;
        }

        self.results.last().copied()
    }

    /// ======================================================
    /// last_n：最近 N 个值
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
    use crate::data::candle::Candle;

    fn create_candle(open_time: u64, high: f64, low: f64, close: f64, closed: bool) -> Candle {
        Candle {
            open_time,
            high,
            low,
            close,
            closed,
            ..Default::default()
        }
    }

    #[test]
    fn test_atr_warmup_and_first_value() {
        let period = 3;
        let mut atr = ATR::new(period, 1000);

        // 1. 第一根仅作为 prev_close 基准
        atr.update(create_candle(1000, 10.0, 5.0, 8.0, true));
        assert!(atr.latest().is_none());

        // 2. 累积 TR
        // TR1: 12-7=5 (H-L > H-PC & L-PC)
        atr.update(create_candle(2000, 12.0, 7.0, 9.0, true));
        // TR2: 15-8=7 (H-PC=15-9=6, L-PC=8-9=1, H-L=15-8=7)
        atr.update(create_candle(3000, 15.0, 8.0, 14.0, true));
        // TR3: 20-13=7 (H-PC=20-14=6, L-PC=13-14=1, H-L=20-13=7)
        atr.update(create_candle(4000, 20.0, 13.0, 19.0, true));

        // 第一个 ATR = (5 + 7 + 7) / 3 = 6.3333...
        assert!(atr.ready);
        let val = atr.latest().unwrap();
        assert!((val - 6.333333333333333).abs() < 1e-10);
    }

    #[test]
    fn test_atr_wilder_smoothing() {
        let period = 3;
        let mut atr = ATR::new(period, 1000);

        // 预热并获取第一个 ATR (6.333...)
        atr.update(create_candle(1000, 10.0, 10.0, 10.0, true));
        atr.update(create_candle(2000, 15.0, 10.0, 15.0, true)); // TR=5
        atr.update(create_candle(3000, 20.0, 15.0, 20.0, true)); // TR=5
        atr.update(create_candle(4000, 25.0, 20.0, 25.0, true)); // TR=5

        let first_atr = 5.0; // (5+5+5)/3
        assert_eq!(atr.latest().unwrap(), first_atr);

        // 第 5 根确认：TR = 30 - 25 = 5
        // Next_ATR = (5.0 * 2 + 5.0) / 3 = 5.0
        atr.update(create_candle(5000, 30.0, 25.0, 30.0, true));
        assert_eq!(atr.latest().unwrap(), 5.0);

        // 第 6 根确认：出现波动放大 TR = 41 - 30 = 11
        // Next_ATR = (5.0 * 2 + 11.0) / 3 = 21 / 3 = 7.0
        atr.update(create_candle(6000, 41.0, 30.0, 40.0, true));
        assert_eq!(atr.latest().unwrap(), 7.0);
    }

    #[test]
    fn test_atr_preview_isolation() {
        let period = 3;
        let mut atr = ATR::new(period, 1000);

        // 预热
        for i in 1..=4 {
            atr.update(create_candle(
                i * 1000,
                10.0 + i as f64,
                10.0,
                10.0 + i as f64,
                true,
            ));
        }
        let confirmed_atr = atr.latest().unwrap();

        // 预览一根巨大的波动 (closed: false)
        atr.update(create_candle(5000, 100.0, 10.0, 50.0, false));
        let preview_atr = atr.latest().unwrap();
        assert!(preview_atr > confirmed_atr);

        // 再次更新同一根预览 Bar，波动缩小
        atr.update(create_candle(5000, 20.0, 10.0, 15.0, false));
        assert!(atr.latest().unwrap() < preview_atr);

        // 此时内部保存的 confirmed atr 不应改变
        // 我们可以通过发送下一根 closed Bar 来验证
        atr.update(create_candle(5000, 15.0, 15.0, 15.0, true)); // 实际这根 TR = 15-14=1 (假设上一根 Close 是 14)
        assert!((atr.latest().unwrap() - 2.3333333333333335).abs() < 1e-10);
    }

    #[test]
    fn test_atr_price_gap() {
        let mut atr = ATR::new(14, 1000);

        // 预热 15 根使就绪
        for i in 1..=15 {
            atr.update(create_candle(i * 1000, 100.0, 95.0, 98.0, true));
        }

        let before_gap = atr.latest().unwrap();

        // 模拟一个巨大的向上跳空（Gap Up）
        // Prev Close = 98.0, Current High = 150.0, Low = 145.0
        // TR = max(150-145, 150-98, 145-98) = 52.0
        atr.update(create_candle(16000, 150.0, 145.0, 148.0, true));

        let after_gap = atr.latest().unwrap();
        assert!(after_gap > before_gap);
        println!("ATR Before Gap: {}, After Gap: {}", before_gap, after_gap);
    }
}
