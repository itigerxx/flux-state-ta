use crate::core::indicator::Indicator;
use crate::data::candle::Candle;
use crate::utils::candle_check_context::CandleCheckContext;
use crate::window::indicator_series::IndicatorSeries;

/// ======================================================
/// VWAP（Volume Weighted Average Price）
/// ======================================================
///
/// ✔ 核心公式：
///
/// VWAP = Σ(Price × Volume) / Σ(Volume)
///
/// ======================================================
/// ✔ 本实现设计目标：
///
/// 1. 实时性：支持 websocket tick / candle 流式更新
/// 2. 简洁性：纯状态累加，不使用复杂窗口结构
/// 3. 正确性：严格区分 closed / preview
/// 4. 工程实用：latest() 始终返回 latest VWAP
///
/// ======================================================
/// ✔ 适用场景：
///
/// - 日内交易基准线
/// - 机构均价参考
/// - 趋势过滤（price > VWAP / price < VWAP）
///
/// ======================================================
#[derive(Debug)]
pub struct VWAP {
    /// 累计成交金额：Σ(price * volume)
    sum_price_volume: f64,

    /// 累计成交量：Σ(volume)
    sum_volume: f64,

    /// 是否已进入有效计算状态
    ready: bool,

    /// 统一K线校验上下文
    ctx: CandleCheckContext,

    /// 结果序列（支持 latest / last_n）
    results: IndicatorSeries<f64>,

    /// 当前预览 Bar 时间戳（用于隔离 preview / closed）
    preview_bar_time: Option<u64>,
}

impl VWAP {
    /// ======================================================
    /// 创建 VWAP 实例
    /// ======================================================
    pub fn new(results_capacity: usize) -> Self {
        assert!(results_capacity > 0);
        Self {
            sum_price_volume: 0.0,
            sum_volume: 0.0,
            ready: false,
            ctx: CandleCheckContext::default(),
            results: IndicatorSeries::new(results_capacity),
            preview_bar_time: None,
        }
    }

    /// ======================================================
    /// 写入结果（统一处理 preview / closed）
    /// ======================================================
    fn write_result(&mut self, candle: &Candle, value: f64) {
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

impl Indicator for VWAP {
    type Output = f64;

    /// ======================================================
    /// update：VWAP 实时更新逻辑
    /// ======================================================
    fn update(&mut self, candle: Candle) {
        // =========================
        // 1. 数据校验
        // =========================
        if !self.ctx.validate(&candle) {
            return;
        }

        let price = candle.close;
        let volume = candle.volume;

        // ======================================================
        // 2. preview / closed 都允许参与 VWAP 更新
        //    （VWAP本质就是流式累加）
        // ======================================================

        // ------------------------------------------------------
        // closed candle：正式提交状态
        // ------------------------------------------------------
        if candle.closed {
            self.sum_price_volume += price * volume;
            self.sum_volume += volume;

            if self.sum_volume > 0.0 {
                self.ready = true; // 只有产生过有效成交量，才是真正的 Ready
                let vwap = self.sum_price_volume / self.sum_volume;
                self.write_result(&candle, vwap);
            }

            return;
        }

        // ------------------------------------------------------
        // preview candle：临时计算，不污染最终统计
        // ------------------------------------------------------
        let temp_sum_pv = self.sum_price_volume + price * volume;
        let temp_sum_v = self.sum_volume + volume;

        let vwap = if temp_sum_v > 0.0 {
            temp_sum_pv / temp_sum_v
        } else {
            0.0
        };

        // 统一使用 write_result，修复之前重复判断 preview_bar_time 的问题
        self.write_result(&candle, vwap);
    }

    /// ======================================================
    /// latest：获取当前 VWAP
    /// ======================================================
    fn latest(&self) -> Option<Self::Output> {
        if !self.ready {
            return None;
        }

        self.results.last().copied()
    }

    /// ======================================================
    /// last_n：获取历史 VWAP
    /// ======================================================
    fn last_n(&self, n: usize) -> Vec<Self::Output> {
        self.results.tail(n).into_iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 辅助工具：快速创建 Candle
    fn create_candle(time: u64, close: f64, volume: f64, closed: bool) -> Candle {
        Candle {
            open_time: time,
            close,
            volume,
            high: close,
            low: close,
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
    fn test_vwap_basic_accumulation() {
        let mut vwap = VWAP::new(1000);

        // 1. 第一根 K 线 (Closed)
        // PV = 100 * 10 = 1000, V = 10, VWAP = 100.0
        vwap.update(create_candle(1000, 100.0, 10.0, true));
        assert!(vwap.ready);
        assert_approx(vwap.latest().unwrap(), 100.0);

        // 2. 第二根 K 线 (Closed)
        // PV = 1000 + (110 * 20) = 3200, V = 10 + 20 = 30, VWAP = 106.666...
        vwap.update(create_candle(2000, 110.0, 20.0, true));
        assert_approx(vwap.latest().unwrap(), 106.66666666666667);
    }

    #[test]
    fn test_vwap_streaming_preview_logic() {
        let mut vwap = VWAP::new(1000);
        
        // 初始数据
        vwap.update(create_candle(1000, 100.0, 10.0, true));

        // --------------------------------------------------
        // 预览测试：同一根 Bar 内价格/成交量跳动
        // --------------------------------------------------
        let bar_time = 2000;
        
        // Tick 1: 预览 (110 * 10)
        // Temp VWAP = (1000 + 1100) / (10 + 10) = 105.0
        vwap.update(create_candle(bar_time, 110.0, 10.0, false));
        assert_eq!(vwap.last_n(10).len(), 2);
        assert_approx(vwap.latest().unwrap(), 105.0);

        // Tick 2: 价格剧烈跳动 (150 * 40)
        // Temp VWAP = (1000 + 6000) / (10 + 40) = 140.0
        vwap.update(create_candle(bar_time, 150.0, 40.0, false));
        assert_eq!(vwap.last_n(10).len(), 2); // 长度不应增加
        assert_approx(vwap.latest().unwrap(), 140.0);

        // Tick 3: 最终收盘
        vwap.update(create_candle(bar_time, 150.0, 40.0, true));
        assert_eq!(vwap.last_n(10).len(), 2);
        assert_approx(vwap.latest().unwrap(), 140.0);

        // --------------------------------------------------
        // 跨 Bar 验证
        // --------------------------------------------------
        vwap.update(create_candle(3000, 100.0, 10.0, false));
        assert_eq!(vwap.last_n(10).len(), 3);
    }

    #[test]
    fn test_vwap_state_isolation() {
        let mut vwap = VWAP::new(1000);
        vwap.update(create_candle(1000, 100.0, 10.0, true));
        
        let confirmed_pv = vwap.sum_price_volume;
        let confirmed_v = vwap.sum_volume;

        // 注入预览 Tick
        vwap.update(create_candle(2000, 500.0, 1000.0, false));
        
        // 核心验证：持久化字段不能被预览 Tick 修改
        assert_eq!(vwap.sum_price_volume, confirmed_pv, "Preview must not pollute sum_price_volume");
        assert_eq!(vwap.sum_volume, confirmed_v, "Preview must not pollute sum_volume");

        // 再次预览（同一根 Bar），验证它是基于 confirmed 状态计算，而不是上一个预览状态
        vwap.update(create_candle(2000, 110.0, 10.0, false));
        // (1000 + 110*10) / (10 + 10) = 105.0
        assert_approx(vwap.latest().unwrap(), 105.0);
    }

    #[test]
    fn test_vwap_zero_volume_robustness() {
        let mut vwap = VWAP::new(1000);

        // 场景 1: 第一根就是零成交量 (不应 ready)
        vwap.update(create_candle(1000, 100.0, 0.0, true));
        assert!(!vwap.ready);
        assert!(vwap.latest().is_none());

        // 场景 2: 预览零成交量
        vwap.update(create_candle(2000, 100.0, 0.0, false));
        assert!(vwap.latest().is_none());

        // 场景 3: 正常数据后跟零成交量确认 (VWAP 不应变)
        vwap.update(create_candle(3000, 100.0, 10.0, true));
        let last_val = vwap.latest().unwrap();
        vwap.update(create_candle(4000, 200.0, 0.0, true));
        assert_approx(vwap.latest().unwrap(), last_val);
    }

    #[test]
    fn test_vwap_idempotency() {
        let mut vwap = VWAP::new(1000);
        
        // 正常写入
        vwap.update(create_candle(1000, 100.0, 10.0, true));
        let len_initial = vwap.last_n(100).len();
        let val_initial = vwap.latest().unwrap();

        // 模拟重复发送已确认的 Bar (同一 open_time)
        vwap.update(create_candle(1000, 100.0, 10.0, true));
        
        // Context 应该拦截，或者内部逻辑识别出 Bar 时间已处理
        assert_eq!(vwap.last_n(100).len(), len_initial, "Duplicate bar should not increase results length");
        assert_approx(vwap.latest().unwrap(), val_initial);
    }
}