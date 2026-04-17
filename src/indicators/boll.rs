use crate::core::indicator::Indicator;
use crate::data::candle::Candle;
use crate::utils::candle_check_context::CandleCheckContext;
use crate::window::indicator_series::IndicatorSeries;

/// ======================================================
/// Bollinger Bands Output
/// ======================================================
///
/// upper : 上轨
/// mid   : 中轨（SMA）
/// lower : 下轨
///
#[derive(Debug, Clone, Copy, Default)]
pub struct BollOutput {
    pub upper: f64,
    pub mid: f64,
    pub lower: f64,
}

/// ======================================================
/// Bollinger Bands（布林带）
/// ======================================================
///
/// 设计目标：
/// ✔ 实时性：latest() 始终返回当前 Tick 最新值
/// ✔ 简洁性：仅保留实时交易所需状态
/// ✔ 易使用：last_n() 可直接绘图
///
/// ======================================================
/// 公式
/// ======================================================
///
/// MID   = SMA(period)
/// STD   = 标准差
/// UPPER = MID + k * STD
/// LOWER = MID - k * STD
///
/// 默认 k = 2.0
///
#[derive(Debug)]
pub struct BOLL {
    /// 周期
    period: usize,

    /// 标准差倍数
    multiplier: f64,

    /// 是否已可输出
    ready: bool,

    /// 已确认窗口价格
    window: IndicatorSeries<f64>,

    /// 已确认价格总和
    sum: f64,

    /// 已确认平方和（用于快速算方差）
    sum_sq: f64,

    /// 输入校验器
    ctx: CandleCheckContext,

    /// 输出结果
    results: IndicatorSeries<BollOutput>,

    /// 当前预览 Bar 时间
    preview_bar_time: Option<u64>,
}

impl BOLL {
    /// ======================================================
    /// 创建实例（默认2倍标准差）
    /// ======================================================
    pub fn new(period: usize, multiplier: f64, results_capacity: usize) -> Self {
        assert!(period > 0);
        assert!(results_capacity > 0);

        Self {
            period,
            multiplier,
            ready: false,
            window: IndicatorSeries::new(period),
            sum: 0.0,
            sum_sq: 0.0,
            ctx: CandleCheckContext::default(),
            results: IndicatorSeries::new(results_capacity),
            preview_bar_time: None,
        }
    }

    /// ======================================================
    /// 根据 sum / sum_sq / n 计算布林带
    /// ======================================================
    fn calc_band(&self, sum: f64, sum_sq: f64, n: usize) -> BollOutput {
        let mean = sum / n as f64;

        // 方差 = E[x²] - E[x]²
        let variance = (sum_sq / n as f64) - (mean * mean);

        // 浮点误差保护
        let std = variance.max(0.0).sqrt();

        let upper = mean + self.multiplier * std;
        let lower = mean - self.multiplier * std;

        BollOutput {
            upper,
            mid: mean,
            lower,
        }
    }

    /// ======================================================
    /// 写入结果（与你 MACD / ADX 风格一致）
    /// ======================================================
    fn write_result(&mut self, candle: &Candle, value: BollOutput) {
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

impl Indicator for BOLL {
    type Output = BollOutput;

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

        let price = candle.close;

        // ==================================================
        // 2. Preview 实时路径（不污染正式状态）
        // ==================================================
        if !candle.closed {
            // 尚未 ready，且确认窗口不足 period-1，无法形成实时值
            if !self.ready && self.window.len() < self.period - 1 {
                return;
            }

            let mut temp_sum = self.sum + price;
            let mut temp_sum_sq = self.sum_sq + price * price;

            // 如果窗口已满，预览时需要模拟滑窗踢出最旧值
            if self.window.len() == self.period {
                if let Some(oldest) = self.window.get(0) {
                    temp_sum -= oldest;
                    temp_sum_sq -= oldest * oldest;
                }
            }

            let output = self.calc_band(temp_sum, temp_sum_sq, self.period);
            self.write_result(&candle, output);
            return;
        }

        // ==================================================
        // 3. Closed 正式路径
        // ==================================================
        self.sum += price;
        self.sum_sq += price * price;

        if let Some(old) = self.window.push(price) {
            self.sum -= old;
            self.sum_sq -= old * old;
        }

        if self.window.len() >= self.period {
            self.ready = true;
        }

        if self.ready {
            let output = self.calc_band(self.sum, self.sum_sq, self.period);
            self.write_result(&candle, output);
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
    fn test_boll_warmup() {
        let period = 3;
        let mut boll = BOLL::new(period, 2.0, 1000);

        // 第 1-2 根收盘，不应 ready
        boll.update(create_candle(1000, 10.0, true));
        boll.update(create_candle(2000, 20.0, true));
        assert!(!boll.ready);
        assert!(boll.latest().is_none());

        // 第 3 根收盘，ready
        boll.update(create_candle(3000, 30.0, true));
        assert!(boll.ready);

        let out = boll.latest().unwrap();
        // Mean = (10+20+30)/3 = 20
        // Var = (100+400+900)/3 - 20^2 = 1400/3 - 400 = 466.666... - 400 = 66.666...
        // Std = sqrt(66.666...) ≈ 8.1649658
        assert_eq!(out.mid, 20.0);
        assert!((out.upper - (20.0 + 2.0 * 8.16496580927726)).abs() < 1e-10);
    }

    #[test]
    fn test_boll_sliding_window() {
        let period = 2;
        let mut boll = BOLL::new(period, 2.0, 1000);

        boll.update(create_candle(1000, 10.0, true));
        boll.update(create_candle(2000, 20.0, true)); // 窗口: [10, 20]

        let out1 = boll.latest().unwrap();
        assert_eq!(out1.mid, 15.0); // (10+20)/2

        boll.update(create_candle(3000, 40.0, true)); // 窗口: [20, 40], 10被弹出
        let out2 = boll.latest().unwrap();
        assert_eq!(out2.mid, 30.0); // (20+40)/2

        // 验证标准差计算是否同步更新 (窗口 [20, 40])
        // Mean = 30, Var = (400 + 1600)/2 - 30^2 = 1000 - 900 = 100, Std = 10
        assert_eq!(out2.upper, 30.0 + 2.0 * 10.0);
        assert_eq!(out2.lower, 30.0 - 2.0 * 10.0);
    }

    #[test]
    fn test_boll_preview_isolation() {
        let period = 2;
        let mut boll = BOLL::new(period, 2.0, 1000);

        boll.update(create_candle(1000, 10.0, true));
        boll.update(create_candle(2000, 20.0, true));

        // 预览一根极大值 (未收盘)
        boll.update(create_candle(3000, 100.0, false));
        let preview_mid = boll.latest().unwrap().mid;
        assert_eq!(preview_mid, 60.0); // (20 + 100) / 2

        // 预览同一根 Bar，价格变动
        boll.update(create_candle(3000, 40.0, false));
        assert_eq!(boll.latest().unwrap().mid, 30.0); // (20 + 40) / 2

        // 此时确认收盘，价格为 40
        boll.update(create_candle(3000, 40.0, true));

        // 关键点：发送第 4 根 Bar 验证之前的 sum/sum_sq 没有被 100.0 污染
        boll.update(create_candle(4000, 60.0, true)); // 窗口: [40, 60]
        let final_mid = boll.latest().unwrap().mid;
        assert_eq!(final_mid, 50.0); // (40 + 60) / 2
    }

    #[test]
    fn test_boll_floating_point_safety() {
        let mut boll = BOLL::new(5, 2.0, 1000);
        // 发送完全相同的价格，测试 variance 是否会因为浮点误差变成微小的负数导致 sqrt 崩溃
        for i in 1..=10 {
            boll.update(create_candle(i * 1000, 1.2345678, true));
        }

        let out = boll.latest().unwrap();
        assert_eq!(out.mid, 1.2345678);
        assert_eq!(out.upper, 1.2345678);
        assert_eq!(out.lower, 1.2345678);
    }
}
