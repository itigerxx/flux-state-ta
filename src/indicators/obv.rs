use crate::core::indicator::Indicator;
use crate::data::candle::Candle;
use crate::utils::candle_check_context::CandleCheckContext;
use crate::window::indicator_series::IndicatorSeries;

/// ======================================================
/// OBV（On-Balance Volume，能量潮）
/// ======================================================
///
/// ✔ 核心公式：
///
/// 若 close > prev_close → OBV += volume
/// 若 close < prev_close → OBV -= volume
/// 若 close == prev_close → OBV += 0
///
/// ======================================================
/// ✔ 本实现 design 目标：
///
/// 1. 实时性：支持 websocket tick / candle 流式更新
/// 2. O(1) 更新：纯累加器结构，无窗口计算
/// 3. preview / closed 隔离：避免未完成K线污染确认状态
/// 4. 交易导向：latest() 永远返回当前资金流方向
///
/// ======================================================
/// ✔ OBV本质：
///
/// 👉 “资金流动方向的累计表达”
#[derive(Debug)]
pub struct OBV {
    /// 当前 OBV 值（已确认状态）
    obv: f64,

    /// 上一根已确认 close（用于判断方向）
    prev_close: Option<f64>,

    /// 是否已进入有效计算状态
    ready: bool,

    /// 统一K线校验上下文
    ctx: CandleCheckContext,

    /// 结果序列
    results: IndicatorSeries<f64>,

    /// 当前预览 Bar 时间戳
    preview_bar_time: Option<u64>,
}

impl OBV {
    /// ======================================================
    /// 创建 OBV 实例
    /// ======================================================
    pub fn new(results_capacity: usize) -> Self {
        assert!(results_capacity > 0);
        Self {
            obv: 0.0,
            prev_close: None,
            ready: false,
            ctx: CandleCheckContext::default(),
            results: IndicatorSeries::new(results_capacity),
            preview_bar_time: None,
        }
    }

    /// ======================================================
    /// 计算方向增量
    /// ======================================================
    #[inline]
    fn delta(&self, close: f64, volume: f64) -> f64 {
        match self.prev_close {
            None => 0.0,
            Some(prev) => {
                if close > prev {
                    volume
                } else if close < prev {
                    -volume
                } else {
                    0.0
                }
            }
        }
    }

    /// ======================================================
    /// 写入结果（preview / closed）
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

impl Indicator for OBV {
    type Output = f64;

    /// ======================================================
    /// update：OBV 实时更新逻辑
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
        // 2. preview candle：临时计算（不污染 confirmed OBV）
        // ======================================================
        if !candle.closed {
            let delta = self.delta(price, volume);
            let preview_obv = self.obv + delta;
            
            // 统一调用 write_result 维护状态机
            self.write_result(&candle, preview_obv);
            return;
        }

        // ======================================================
        // 3. closed candle：正式提交 OBV 状态
        // ======================================================
        let delta = self.delta(price, volume);
        self.obv += delta;

        // 统一调用 write_result
        self.write_result(&candle, self.obv);

        // 更新 prev_close（只在 confirmed 更新）
        self.prev_close = Some(price);
        self.ready = true;
    }

    /// ======================================================
    /// latest：获取当前 OBV
    /// ======================================================
    fn latest(&self) -> Option<Self::Output> {
        if !self.ready {
            return None;
        }

        self.results.last().copied()
    }

    /// ======================================================
    /// last_n：获取历史 OBV
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
            closed,
            ..Default::default()
        }
    }

    #[test]
    fn test_obv_basic_accumulation() {
        let mut obv = OBV::new(1000);

        // 1. 第一根 K 线：OBV 初始通常不累加（因为没有 prev_close）
        obv.update(create_candle(1000, 100.0, 1000.0, true));
        assert_eq!(obv.latest().unwrap(), 0.0);
        assert!(obv.ready);

        // 2. 价格上涨：OBV += volume
        obv.update(create_candle(2000, 105.0, 500.0, true));
        assert_eq!(obv.latest().unwrap(), 500.0);

        // 3. 价格下跌：OBV -= volume
        obv.update(create_candle(3000, 102.0, 300.0, true));
        assert_eq!(obv.latest().unwrap(), 200.0); // 500 - 300

        // 4. 价格持平：OBV 不变
        obv.update(create_candle(4000, 102.0, 800.0, true));
        assert_eq!(obv.latest().unwrap(), 200.0);
    }

    #[test]
    fn test_obv_streaming_preview_logic() {
        let mut obv = OBV::new(1000);
        
        // 初始确认一根基准 K 线
        obv.update(create_candle(1000, 100.0, 1000.0, true));
        let base_obv = obv.latest().unwrap(); // 0.0

        // --------------------------------------------------
        // 预览测试：同一根 Bar 内价格跳动
        // --------------------------------------------------
        let bar_time = 2000;
        
        // Tick 1: 价格上涨 (Preview)
        obv.update(create_candle(bar_time, 110.0, 500.0, false));
        assert_eq!(obv.last_n(10).len(), 2);
        assert_eq!(obv.latest().unwrap(), base_obv + 500.0);

        // Tick 2: 价格跌破基准 (同一根 Bar, Preview)
        // 注意：OBV 的方向是基于上一个 *Confirmed* Close 计算的
        obv.update(create_candle(bar_time, 90.0, 500.0, false));
        assert_eq!(obv.last_n(10).len(), 2); // 长度不应增加
        assert_eq!(obv.latest().unwrap(), base_obv - 500.0); // 更新为负向

        // Tick 3: 最终收盘 (Confirmed)
        obv.update(create_candle(bar_time, 110.0, 500.0, true));
        assert_eq!(obv.last_n(10).len(), 2);
        assert_eq!(obv.latest().unwrap(), 500.0);
        
        // --------------------------------------------------
        // 跨 Bar 验证
        // --------------------------------------------------
        obv.update(create_candle(3000, 120.0, 200.0, false));
        assert_eq!(obv.last_n(10).len(), 3);
        assert_eq!(obv.latest().unwrap(), 500.0 + 200.0);
    }

    #[test]
    fn test_obv_state_isolation() {
        let mut obv = OBV::new(1000);
        obv.update(create_candle(1000, 100.0, 1000.0, true));
        
        let confirmed_obv_before = obv.obv;

        // 注入一个极其夸张的预览 Tick
        obv.update(create_candle(2000, 999.0, 999999.0, false));
        assert_ne!(obv.latest().unwrap(), confirmed_obv_before);

        // 核心验证：内部持久化的 obv 字段是否未被修改
        assert_eq!(obv.obv, confirmed_obv_before, "Preview should not pollute the inner accumulator state");

        // 注入另一个预览 Tick（同一根 Bar），验证计算基准是否依然是上一个 Confirmed Close
        obv.update(create_candle(2000, 101.0, 100.0, false));
        assert_eq!(obv.latest().unwrap(), confirmed_obv_before + 100.0);
    }

    #[test]
    fn test_obv_idempotency_robustness() {
        let mut obv = OBV::new(1000);
        
        // 模拟重复的收盘信号
        obv.update(create_candle(1000, 100.0, 1000.0, true));
        obv.update(create_candle(1000, 100.0, 1000.0, true)); // 重复发送
        
        assert_eq!(obv.last_n(100).len(), 1, "Should be idempotent for duplicate closed candles");
        
        // 模拟价格虽然没动，但成交量变化的预览（实时 Tick 更新）
        obv.update(create_candle(2000, 100.0, 500.0, false));
        let p1 = obv.latest().unwrap();
        
        obv.update(create_candle(2000, 100.0, 800.0, false)); // 成交量增加，但价格没变
        let p2 = obv.latest().unwrap();
        
        assert_eq!(p1, p2, "OBV should not change if price is equal to prev_close, regardless of volume");
    }

    #[test]
    fn test_obv_negative_accumulation() {
        let mut obv = OBV::new(1000);
        obv.update(create_candle(1000, 100.0, 1000.0, true));
        
        // 连续大幅度缩量下跌
        obv.update(create_candle(2000, 90.0, 5000.0, true));
        obv.update(create_candle(3000, 80.0, 5000.0, true));
        
        assert_eq!(obv.latest().unwrap(), -10000.0, "OBV can and should be able to go negative");
    }
}