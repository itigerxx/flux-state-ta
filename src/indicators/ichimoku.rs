use crate::core::indicator::Indicator;
use crate::data::candle::Candle;
use crate::utils::candle_check_context::CandleCheckContext;
use crate::window::indicator_series::IndicatorSeries;

/// ======================================================
/// Ichimoku Output（一目均衡表输出）
/// ======================================================
///
/// 五条线：
///
/// 1. tenkan   = 转换线（9周期中点）
/// 2. kijun    = 基准线（26周期中点）
/// 3. span_a   = 先行带A
/// 4. span_b   = 先行带B
/// 5. chikou   = 迟行线（当前收盘价）
///
/// 注意：
/// 这里返回的是“当前实时计算值”，
/// 前移/后移绘图交给前端处理。
#[derive(Debug, Clone, Copy, Default)]
pub struct IchimokuOutput {
    pub tenkan: f64,
    pub kijun: f64,
    pub span_a: f64,
    pub span_b: f64,
    pub chikou: f64,
}

/// ======================================================
/// Ichimoku Cloud（一目均衡表）
/// ======================================================
///
/// 设计目标：
///
/// ✔ 实时更新（Tick / WebSocket）
/// ✔ latest() 永远拿到当前最新值
/// ✔ last_n() 获取最近结果序列
/// ✔ 不做复杂架构
/// ✔ 用户交易使用优先
///
/// 默认参数：
///
/// tenkan_period = 9
/// kijun_period  = 26
/// span_b_period = 52
///
#[derive(Debug)]
pub struct Ichimoku {
    /// 转换线周期
    tenkan_period: usize,

    /// 基准线周期
    kijun_period: usize,

    /// Span B 周期
    span_b_period: usize,

    /// K线缓存（存储历史 candle）
    candles: IndicatorSeries<Candle>,

    /// 校验上下文
    ctx: CandleCheckContext,

    /// 输出结果缓存
    results: IndicatorSeries<IchimokuOutput>,

    /// 当前预览中的 Bar 时间戳
    preview_bar_time: Option<u64>,

    /// 是否已就绪
    ready: bool,
}

impl Ichimoku {
    /// ======================================================
    /// 创建默认一目均衡表
    /// ======================================================
    pub fn new(tenkan_period: usize, kijun_period: usize, span_b_period: usize, results_capacity: usize) -> Self {
        Self::with_periods(tenkan_period, kijun_period, span_b_period, results_capacity)
    }

    pub fn default() -> Self {
        Self::with_periods(10, 30, 60, 50)
    }

    /// ======================================================
    /// 自定义周期
    /// ======================================================
    pub fn with_periods(
        tenkan_period: usize,
        kijun_period: usize,
        span_b_period: usize,
        results_capacity: usize,
    ) -> Self {
        assert!(tenkan_period > 0);
        assert!(kijun_period > 0);
        assert!(span_b_period > 0);
        assert!(results_capacity > 0);

        let max_period = span_b_period.max(kijun_period).max(tenkan_period);

        Self {
            tenkan_period,
            kijun_period,
            span_b_period,
            candles: IndicatorSeries::new(max_period + 30),
            ctx: CandleCheckContext::default(),
            results: IndicatorSeries::new(results_capacity),
            preview_bar_time: None,
            ready: false,
        }
    }

    /// ======================================================
    /// 计算最近 n 根K线区间最高价
    /// ======================================================
    fn highest_high(&self, n: usize) -> Option<f64> {
        if self.candles.len() < n {
            return None;
        }

        let start = self.candles.len() - n;
        let mut high = f64::MIN;

        for i in start..self.candles.len() {
            if let Some(c) = self.candles.get(i) {
                if c.high > high {
                    high = c.high;
                }
            }
        }

        Some(high)
    }

    /// ======================================================
    /// 计算最近 n 根K线区间最低价
    /// ======================================================
    fn lowest_low(&self, n: usize) -> Option<f64> {
        if self.candles.len() < n {
            return None;
        }

        let start = self.candles.len() - n;
        let mut low = f64::MAX;

        for i in start..self.candles.len() {
            if let Some(c) = self.candles.get(i) {
                if c.low < low {
                    low = c.low;
                }
            }
        }

        Some(low)
    }

    /// ======================================================
    /// 中点公式：(HH + LL) / 2
    /// ======================================================
    fn midpoint(&self, n: usize) -> Option<f64> {
        let hh = self.highest_high(n)?;
        let ll = self.lowest_low(n)?;
        Some((hh + ll) * 0.5)
    }

    /// ======================================================
    /// 写入结果（支持 preview / confirmed）
    /// ======================================================
    fn write_result(&mut self, candle: &Candle, value: IchimokuOutput) {
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

    /// ======================================================
    /// 使用当前缓存数据计算五条线
    /// ======================================================
    fn calculate(&self, current_close: f64) -> Option<IchimokuOutput> {
        let tenkan = self.midpoint(self.tenkan_period)?;
        let kijun = self.midpoint(self.kijun_period)?;
        let span_b = self.midpoint(self.span_b_period)?;
        let span_a = (tenkan + kijun) * 0.5;
        let chikou = current_close;

        Some(IchimokuOutput {
            tenkan,
            kijun,
            span_a,
            span_b,
            chikou,
        })
    }
}

impl Indicator for Ichimoku {
    type Output = IchimokuOutput;

    /// ======================================================
    /// update：实时更新入口
    /// ======================================================
    fn update(&mut self, candle: &Candle) {
        // 1. 数据校验
        if !self.ctx.validate(&candle) {
            return;
        }

        let bar_time = candle.open_time;

        // ==================================================
        // Preview 模式（当前 Bar 未闭合）
        // ==================================================
        match self.preview_bar_time {
            Some(t) if t == bar_time => {
                self.candles.update_latest(candle.clone());
            }

            _ => {
                self.candles.push(candle.clone());
                self.preview_bar_time = Some(bar_time);
            }
        }
        // ==================================================
        // 计算指标
        // ==================================================
        if let Some(output) = self.calculate(candle.close) {
            if !self.ready {
                self.ready = true;
            }

            self.write_result(&candle, output);
        }
    }

    /// ======================================================
    /// latest：获取最新值
    /// ======================================================
    fn latest(&self) -> Option<Self::Output> {
        if !self.ready {
            return None;
        }

        self.results.last().copied()
    }

    /// ======================================================
    /// last_n：获取最近N个值
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
    fn test_ichimoku_warmup_and_basic_calc() {
        // 使用缩短的周期便于测试: Tenkan=2, Kijun=4, SpanB=6
        let mut ichi = Ichimoku::with_periods(2, 4, 6, 1000);

        // 填充 5 根数据 (不足以计算 Span B)
        for i in 1..=5 {
            ichi.update(&create_candle(
                i * 1000,
                i as f64 * 10.0,
                i as f64 * 5.0,
                i as f64 * 7.0,
                true,
            ));
        }

        // 此时由于 candles.len() < 6, calculate 会返回 None
        assert!(!ichi.ready);
        assert!(ichi.latest().is_none());

        // 第 6 根收盘: High=60, Low=30, Close=42
        // 此时 candles 包含:
        // H: [10, 20, 30, 40, 50, 60]
        // L: [ 5, 10, 15, 20, 25, 30]
        ichi.update(&create_candle(6000, 60.0, 30.0, 42.0, true));

        assert!(ichi.ready);
        let out = ichi.latest().unwrap();

        // 1. Tenkan (2周期): HH(50,60)=60, LL(25,30)=25 => (60+25)/2 = 42.5
        assert_eq!(out.tenkan, 42.5);
        // 2. Kijun (4周期): HH(30,40,50,60)=60, LL(15,20,25,30)=15 => (60+15)/2 = 37.5
        assert_eq!(out.kijun, 37.5);
        // 3. Span A: (Tenkan + Kijun)/2 = (42.5 + 37.5)/2 = 40.0
        assert_eq!(out.span_a, 40.0);
        // 4. Span B (6周期): HH(10..60)=60, LL(5..30)=5 => (60+5)/2 = 32.5
        assert_eq!(out.span_b, 32.5);
        // 5. Chikou: Current Close = 42.0
        assert_eq!(out.chikou, 42.0);
    }

    #[test]
    fn test_ichimoku_preview_refinement() {
        let mut ichi = Ichimoku::with_periods(2, 2, 2, 1000);

        // 初始化 2 根
        ichi.update(&create_candle(1000, 20.0, 10.0, 15.0, true));
        ichi.update(&create_candle(2000, 40.0, 20.0, 30.0, true));

        // 预览第 3 根: 此时 Tenkan 范围是 [Bar2, Bar3]
        // Bar2: H=40, L=20
        // Bar3 (Preview): H=50, L=30, C=35
        ichi.update(&create_candle(3000, 50.0, 30.0, 35.0, false));

        // Tenkan: HH(40,50)=50, LL(20,30)=20 => (50+20)/2 = 35.0
        assert_eq!(ichi.latest().unwrap().tenkan, 35.0);

        // 预览更新: Bar3 突破了更高的 High 和 更低的 Low
        // Bar3 (Preview): H=100, L=10, C=50
        ichi.update(&create_candle(3000, 100.0, 10.0, 50.0, false));

        // Tenkan: HH(40,100)=100, LL(20,10)=10 => (100+10)/2 = 55.0
        assert_eq!(ichi.latest().unwrap().tenkan, 55.0);

        // 确认收盘
        ichi.update(&create_candle(3000, 100.0, 10.0, 50.0, true));
        assert_eq!(ichi.latest().unwrap().tenkan, 55.0);
    }

    #[test]
    fn test_ichimoku_data_isolation() {
        let mut ichi = Ichimoku::with_periods(2, 2, 2, 1000);
        ichi.update(&create_candle(1000, 20.0, 10.0, 15.0, true));
        ichi.update(&create_candle(2000, 20.0, 10.0, 15.0, true));

        // 预览态产生了一个极端值 1000.0，推入了序列
        ichi.update(&create_candle(3000, 1000.0, 1.0, 500.0, false));

        // 直接跳到下一根 Bar 收盘
        ichi.update(&create_candle(4000, 20.0, 10.0, 15.0, true));

        // 按照“占位优先”逻辑，Bar 3 的预览值 1000.0 会被保留
        // Tenkan (2周期) = (HH(1000, 20) + LL(1, 10)) / 2
        //               = (1000 + 1) / 2 = 500.5
        assert_eq!(ichi.latest().unwrap().tenkan, 500.5);
    }
}
