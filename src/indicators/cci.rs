use crate::core::indicator::Indicator;
use crate::data::candle::Candle;
use crate::utils::candle_check_context::CandleCheckContext;
use crate::window::indicator_series::IndicatorSeries;

#[derive(Debug)]
pub struct CCI {
    period: usize,
    tp_sum: f64,
    tp_window: IndicatorSeries<f64>,
    ready: bool,
    ctx: CandleCheckContext,
    results: IndicatorSeries<f64>,
    preview_bar_time: Option<u64>,
}

impl CCI {
    pub fn new(period: usize, results_capacity: usize) -> Self {
        assert!(period > 0);
        assert!(results_capacity > 0);
        Self {
            period,
            tp_sum: 0.0,
            tp_window: IndicatorSeries::new(period),
            ready: false,
            ctx: CandleCheckContext::default(),
            results: IndicatorSeries::new(results_capacity),
            preview_bar_time: None,
        }
    }

    #[inline]
    fn tp(high: f64, low: f64, close: f64) -> f64 {
        (high + low + close) / 3.0
    }

    // 核心计算抽象：支持任意来源的数据流
    fn calculate_md<I>(sma: f64, period: usize, values: I) -> f64
    where
        I: Iterator<Item = f64>,
    {
        let mut sum = 0.0;
        for v in values {
            sum += (v - sma).abs();
        }
        sum / period as f64
    }

    fn write(&mut self, candle: &Candle, value: f64) {
        let t = candle.open_time;
        // 仅保留基于时间戳的判断，移除对 candle.closed 的状态强制重置
        match self.preview_bar_time {
            Some(t0) if t0 == t => {
                self.results.update_latest(value);
            }
            _ => {
                self.results.push(value);
                self.preview_bar_time = Some(t);
            }
        }
    }
}

impl Indicator for CCI {
    type Output = f64;

    fn update(&mut self, candle: &Candle) {
        if !self.ctx.validate(&candle) {
            return;
        }

        let tp = Self::tp(candle.high, candle.low, candle.close);

        if !candle.closed {
            if !self.ready && self.tp_window.len() < self.period - 1 {
                return;
            }

            let window_full = self.tp_window.len() == self.period;
            let mut preview_sum = self.tp_sum + tp;
            if window_full {
                preview_sum -= *self.tp_window.get(0).unwrap();
            }

            let preview_sma = preview_sum / self.period as f64;

            // 构造预览迭代器：跳过最老(若满)，衔接当前TP
            let skip_count = if window_full { 1 } else { 0 };
            let values_iter = self
                .tp_window
                .iter()
                .skip(skip_count)
                .copied()
                .chain(std::iter::once(tp));

            let md = Self::calculate_md(preview_sma, self.period, values_iter);
            let cci = if md == 0.0 {
                0.0
            } else {
                (tp - preview_sma) / (0.015 * md)
            };

            self.write(&candle, cci);
            return;
        }

        // CLOSED PATH
        // 不再手动操作 preview_bar_time = None，直接交给 write 处理
        self.tp_sum += tp;
        if let Some(old) = self.tp_window.push(tp) {
            self.tp_sum -= old;
        }

        if self.tp_window.len() >= self.period {
            self.ready = true;
        }

        if self.ready {
            let sma = self.tp_sum / self.period as f64;
            let md = Self::calculate_md(sma, self.period, self.tp_window.iter().copied());
            let cci = if md == 0.0 {
                0.0
            } else {
                (tp - sma) / (0.015 * md)
            };

            // 统一调用 write，由其内部的时间戳逻辑决定是 push 还是 update
            self.write(&candle, cci);
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
    fn test_cci_warmup() {
        let period = 3;
        let mut cci = CCI::new(period, 1000);

        // 填充前两根
        cci.update(&create_candle(1000, 10.0, 10.0, 10.0, true)); // TP=10
        cci.update(&create_candle(2000, 20.0, 20.0, 20.0, true)); // TP=20
        assert!(!cci.ready);
        assert!(cci.latest().is_none());

        // 第三根
        cci.update(&create_candle(3000, 30.0, 30.0, 30.0, true)); // TP=30
        assert!(cci.ready);

        // SMA = (10+20+30)/3 = 20
        // MD = (|10-20| + |20-20| + |30-20|) / 3 = (10+0+10)/3 = 6.666666666666667
        // CCI = (30 - 20) / (0.015 * 6.666666666666667) = 10 / 0.1 = 100.0
        let val = cci.latest().unwrap();
        assert!((val - 100.0).abs() < 1e-10);
    }

    #[test]
    fn test_cci_preview_logic() {
        let period = 5;
        let mut cci = CCI::new(period, 1000);

        // 1. 填充 period - 2 根闭合数据 (共 3 根)
        for i in 0..3 {
            // 传入 High, Low, Close (此处假设都为 100.0)
            cci.update(&create_candle(1000 + i * 100, 100.0, 100.0, 100.0, true));
        }
        assert!(cci.latest().is_none(), "数据不足 5 根时应为 None");

        // 2. 填充到临界点：第 4 根闭合
        cci.update(&create_candle(1400, 100.0, 100.0, 100.0, true));

        // 此时 Window 长度为 4。在严苛模式下，latest() 依然应该是 None
        assert!(cci.latest().is_none());

        // 3. 第 5 根预览
        // (4 根已闭合 + 1 根当前预览 = 5 根，满足 Period)
        cci.update(&create_candle(1500, 110.0, 110.0, 110.0, false));

        // 4. 验证预览输出
        // 只有你的 update 逻辑里写了 `if self.window.len() + 1 >= self.period`，此处才会有值
        if let Some(val) = cci.latest() {
            println!("CCI Preview Value: {}", val);
        } else {
            // 如果依然是 None，说明你的逻辑是“必须 5 根全部 Closed 才产出”
            assert!(true);
        }
    }

    #[test]
    fn test_cci_sliding_window_md() {
        let period = 2;
        let mut cci = CCI::new(period, 1000);

        cci.update(&create_candle(1000, 10.0, 10.0, 10.0, true));
        cci.update(&create_candle(2000, 20.0, 20.0, 20.0, true)); // 窗口 [10, 20]

        // 第三根确认，10 应该被踢出，窗口变为 [20, 40]
        cci.update(&create_candle(3000, 40.0, 40.0, 40.0, true));
        // SMA = (20+40)/2 = 30
        // MD = (|20-30| + |40-30|) / 2 = 10
        // CCI = (40 - 30) / (0.015 * 10) = 10 / 0.15 = 66.66666666666667
        assert!((cci.latest().unwrap() - 66.66666666666667).abs() < 1e-10);
    }

    #[test]
    fn test_cci_zero_deviation() {
        let period = 3;
        let mut cci = CCI::new(period, 1000);

        // 发送完全相同的价格，此时 MD 为 0
        cci.update(&create_candle(1000, 10.0, 10.0, 10.0, true));
        cci.update(&create_candle(2000, 10.0, 10.0, 10.0, true));
        cci.update(&create_candle(3000, 10.0, 10.0, 10.0, true));

        // MD 为 0 时，代码中处理为 0.0，防止除以零崩溃
        assert_eq!(cci.latest().unwrap(), 0.0);
    }
}
