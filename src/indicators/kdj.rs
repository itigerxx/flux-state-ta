use crate::core::indicator::Indicator;
use crate::data::candle::Candle;
use crate::utils::candle_check_context::CandleCheckContext;
use crate::window::indicator_series::IndicatorSeries;

/// ======================================================
/// KDJ Output
/// ======================================================
///
/// k : 快线
/// d : 慢线
/// j : 放大线（3K - 2D）
///
#[derive(Debug, Clone, Copy, Default)]
pub struct KdjOutput {
    pub k: f64,
    pub d: f64,
    pub j: f64,
}

/// ======================================================
/// KDJ（随机指标）
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
/// RSV = (close - LLV) / (HHV - LLV) * 100
///
/// K = (2/3) * prev_k + (1/3) * RSV
/// D = (2/3) * prev_d + (1/3) * K
/// J = 3 * K - 2 * D
///
/// 默认初始值：K=50, D=50
///
#[derive(Debug)]
pub struct KDJ {
    /// RSV窗口周期
    period: usize,

    /// 是否可输出
    ready: bool,

    /// 输入校验器
    ctx: CandleCheckContext,

    /// 最近已确认 high 序列
    highs: IndicatorSeries<f64>,

    /// 最近已确认 low 序列
    lows: IndicatorSeries<f64>,

    /// 输出结果缓存
    results: IndicatorSeries<KdjOutput>,

    /// 当前预览中的 Bar 时间戳
    preview_bar_time: Option<u64>,

    /// 已确认 K 状态
    k: f64,

    /// 已确认 D 状态
    d: f64,
}

impl KDJ {
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
            highs: IndicatorSeries::new(period),
            lows: IndicatorSeries::new(period),
            results: IndicatorSeries::new(results_capacity),
            preview_bar_time: None,
            k: 50.0,
            d: 50.0,
        }
    }

    /// ======================================================
    /// 单次遍历：同时计算窗口最高价和最低价
    /// ======================================================
    fn find_hhv_llv(highs: &IndicatorSeries<f64>, lows: &IndicatorSeries<f64>) -> (f64, f64) {
        let mut max_v = f64::MIN;
        let mut min_v = f64::MAX;

        // 假设 highs 和 lows 长度一致
        for i in 0..highs.len() {
            if let Some(h) = highs.get(i) {
                if *h > max_v {
                    max_v = *h;
                }
            }
            if let Some(l) = lows.get(i) {
                if *l < min_v {
                    min_v = *l;
                }
            }
        }

        (max_v, min_v)
    }

    /// ======================================================
    /// 根据 RSV 推导 KDJ
    /// ======================================================
    fn calc_from(&self, prev_k: f64, prev_d: f64, rsv: f64) -> KdjOutput {
        let k = (2.0 / 3.0) * prev_k + (1.0 / 3.0) * rsv;
        let d = (2.0 / 3.0) * prev_d + (1.0 / 3.0) * k;
        let j = 3.0 * k - 2.0 * d;

        KdjOutput { k, d, j }
    }

    /// ======================================================
    /// 写入结果（统一状态机）
    /// ======================================================
    fn write_result(&mut self, candle: &Candle, value: KdjOutput) {
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

impl Indicator for KDJ {
    type Output = KdjOutput;

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
        // 2. Preview 路径（实时计算，不污染正式状态）
        // ==================================================
        if !is_closed {
            if !self.ready && self.highs.len() < self.period - 1 {
                return;
            }

            // 使用新函数
            let (mut hhv, mut llv) = if self.highs.len() == 0 {
                (high, low)
            } else {
                Self::find_hhv_llv(&self.highs, &self.lows)
            };

            // 将当前 Tick 加入对比
            hhv = hhv.max(high);
            llv = llv.min(low);

            let rsv = if (hhv - llv).abs() < f64::EPSILON {
                50.0
            } else {
                (close - llv) / (hhv - llv) * 100.0
            };

            let output = self.calc_from(self.k, self.d, rsv);
            self.write_result(&candle, output);
            return;
        }

        // ==================================================
        // 3. Closed 路径：正式提交窗口
        // ==================================================
        self.highs.push(high);
        self.lows.push(low);

        if self.highs.len() >= self.period {
            self.ready = true;
        }

        if !self.ready {
            return;
        }

        let (hhv, llv) = Self::find_hhv_llv(&self.highs, &self.lows);

        let rsv = if (hhv - llv).abs() < f64::EPSILON {
            50.0
        } else {
            (close - llv) / (hhv - llv) * 100.0
        };

        let output = self.calc_from(self.k, self.d, rsv);

        // 收盘提交正式状态
        self.k = output.k;
        self.d = output.d;

        self.write_result(&candle, output);
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

    /// 辅助工具：快速创建 Candle
    fn create_candle(time: u64, close: f64, high: f64, low: f64, closed: bool) -> Candle {
        Candle {
            open_time: time,
            close,
            high,
            low,
            closed,
            ..Default::default()
        }
    }

    /// 辅助工具：浮点数近似相等判断
    fn assert_approx(actual: f64, expected: f64, precision: f64) {
        assert!(
            (actual - expected).abs() <= precision,
            "Expected {}, got {}",
            expected,
            actual
        );
    }

    #[test]
    fn test_kdj_full_lifecycle() {
        // 设定周期为 3
        let mut kdj = KDJ::new(3, 1000);

        // --------------------------------------------------
        // 1. Warmup 阶段：未达到周期不应有输出
        // --------------------------------------------------
        kdj.update(create_candle(1000, 10.0, 10.0, 10.0, true));
        kdj.update(create_candle(2000, 10.0, 10.0, 10.0, true));
        assert!(!kdj.ready);
        assert!(kdj.latest().is_none());

        // --------------------------------------------------
        // 2. Ready 阶段：达到周期产生第一个值
        // --------------------------------------------------
        // RSV = (10-10)/(10-10) -> 50.0 (平市处理)
        // K = 2/3*50 + 1/3*50 = 50.0
        kdj.update(create_candle(3000, 10.0, 10.0, 10.0, true));
        assert!(kdj.ready);
        let first = kdj.latest().unwrap();
        assert_approx(first.k, 50.0, 1e-10);

        // --------------------------------------------------
        // 3. Preview 实时性测试：同一根 Bar 的多次 Tick
        // --------------------------------------------------
        // 第一次 Tick (Preview)
        kdj.update(create_candle(4000, 12.0, 15.0, 10.0, false));
        let len_after_first_tick = kdj.last_n(10).len();
        assert_eq!(len_after_first_tick, 2); // 之前 1 个确认的 + 1 个预览的
        let p1 = kdj.latest().unwrap();

        // 第二次 Tick (同一根 Bar，价格上涨)
        kdj.update(create_candle(4000, 14.0, 15.0, 10.0, false));
        assert_eq!(kdj.last_n(10).len(), 2); // 长度不应增加（幂等）
        let p2 = kdj.latest().unwrap();
        assert!(p2.k > p1.k, "K should increase when close price increases in preview");

        // --------------------------------------------------
        // 4. 状态隔离测试：Preview 不应影响持久化状态
        // --------------------------------------------------
        let k_before_close = kdj.k;
        let d_before_close = kdj.d;
        // 虽然此时最新的结果是 p2，但 KDJ 内部存储的已确认 K/D 必须还是上一个 Closed Bar 的
        assert_approx(k_before_close, 50.0, 1e-10);
        assert_approx(d_before_close, 50.0, 1e-10);

        // --------------------------------------------------
        // 5. 闭合测试：Preview 转为正式数据
        // --------------------------------------------------
        kdj.update(create_candle(4000, 14.0, 15.0, 10.0, true));
        assert_eq!(kdj.last_n(10).len(), 2); // 长度依然是 2
        assert_ne!(kdj.k, 50.0); // 内部状态现在应该更新了
        assert_approx(kdj.latest().unwrap().k, p2.k, 1e-10);

        // --------------------------------------------------
        // 6. 跨 Bar 清理测试：新 Bar 应该开启新位置
        // --------------------------------------------------
        kdj.update(create_candle(5000, 15.0, 16.0, 14.0, false));
        assert_eq!(kdj.last_n(10).len(), 3); // 确认 2 个 + 预览 1 个
    }

    #[test]
    fn test_kdj_flat_market_robustness() {
        let mut kdj = KDJ::new(3, 1000);
        // 模拟价格完全不动（HHV == LLV）
        for i in 0..5 {
            kdj.update(create_candle(i * 1000, 100.0, 100.0, 100.0, true));
        }
        let out = kdj.latest().unwrap();
        // 结果应稳定在 50 附近，且不应发生除零错误或 NaN
        assert_approx(out.k, 50.0, 1e-10);
        assert_approx(out.d, 50.0, 1e-10);
        assert_approx(out.j, 50.0, 1e-10);
        assert!(!out.k.is_nan());
    }

    #[test]
    fn test_kdj_extreme_volatility() {
        let mut kdj = KDJ::new(3, 1000);
        // 快速拉升后快速回落
        kdj.update(create_candle(1000, 10.0, 10.0, 10.0, true));
        kdj.update(create_candle(2000, 20.0, 20.0, 10.0, true));
        kdj.update(create_candle(3000, 30.0, 30.0, 10.0, true));
        
        let first_k = kdj.latest().unwrap().k;
        
        // 预览一个巨大的下跌
        kdj.update(create_candle(4000, 5.0, 30.0, 5.0, false));
        let crash_k = kdj.latest().unwrap().k;
        
        assert!(crash_k < first_k, "K should drop significantly on price crash");
    }

    #[test]
    fn test_kdj_idempotent_closed_signals() {
        let mut kdj = KDJ::new(3, 1000);
        kdj.update(create_candle(1000, 10.0, 11.0, 9.0, true));
        kdj.update(create_candle(2000, 10.0, 11.0, 9.0, true));
        kdj.update(create_candle(3000, 10.0, 11.0, 9.0, true));
        
        let len_initial = kdj.last_n(100).len();

        // 模拟同一个 Bar 的 Closed 信号由于某种原因发送了两次
        kdj.update(create_candle(3000, 10.0, 11.0, 9.0, true));
        
        // 如果 context 校验正常，长度不应改变（由 CandleCheckContext 保证）
        // 如果 context 没过滤掉，这里即便进入逻辑，结果也应保持一致
        assert_eq!(kdj.last_n(100).len(), len_initial);
    }
}