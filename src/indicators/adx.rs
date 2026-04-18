use crate::core::indicator::Indicator;
use crate::data::candle::Candle;
use crate::utils::candle_check_context::CandleCheckContext;
use crate::window::indicator_series::IndicatorSeries;

/// ======================================================
/// ADX Output
/// ======================================================
///
/// adx     : 趋势强度
/// plus_di : +DI 多头方向指标
/// minus_di: -DI 空头方向指标
///
#[derive(Debug, Clone, Copy, Default)]
pub struct AdxOutput {
    pub adx: f64,
    pub plus_di: f64,
    pub minus_di: f64,
}

/// ======================================================
/// ADX（Average Directional Index）
/// ======================================================
///
/// 设计目标：
/// ✔ 实时性：latest() 始终返回当前 Tick 对应最新值
/// ✔ 简洁性：仅保留实时交易必要状态
/// ✔ 可用性：last_n() 可直接给前端绘图
///
/// ======================================================
/// 计算说明（Wilder体系）
/// ======================================================
///
/// TR   = True Range
/// +DM  = Positive Directional Movement
/// -DM  = Negative Directional Movement
///
/// 平滑后：
/// smoothed_tr
/// smoothed_plus_dm
/// smoothed_minus_dm
///
/// +DI = 100 * smoothed_plus_dm / smoothed_tr
/// -DI = 100 * smoothed_minus_dm / smoothed_tr
///
/// DX  = 100 * abs(+DI - -DI) / (+DI + -DI)
///
/// ADX = DX 的 Wilder 平滑均值
///
#[derive(Debug)]
pub struct ADX {
    /// 周期
    period: usize,

    /// 是否就绪
    ready: bool,

    /// 输入校验器
    ctx: CandleCheckContext,

    /// 输出结果
    results: IndicatorSeries<AdxOutput>,

    /// 当前预览 Bar 时间
    preview_bar_time: Option<u64>,

    // ==================================================
    // 上一根K线（用于计算 TR / DM）
    // ==================================================
    prev_high: Option<f64>,
    prev_low: Option<f64>,
    prev_close: Option<f64>,

    // ==================================================
    // Wilder 平滑状态（仅保存 confirmed 状态）
    // ==================================================
    smoothed_tr: Option<f64>,
    smoothed_plus_dm: Option<f64>,
    smoothed_minus_dm: Option<f64>,
    adx_value: Option<f64>,

    // ==================================================
    // 初始化累计区
    // ==================================================
    warmup_count: usize,
    tr_sum: f64,
    plus_dm_sum: f64,
    minus_dm_sum: f64,

    // DX 初始化（用于首个 ADX）
    dx_window: IndicatorSeries<f64>,
}

impl ADX {
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

            prev_high: None,
            prev_low: None,
            prev_close: None,

            smoothed_tr: None,
            smoothed_plus_dm: None,
            smoothed_minus_dm: None,
            adx_value: None,

            warmup_count: 0,
            tr_sum: 0.0,
            plus_dm_sum: 0.0,
            minus_dm_sum: 0.0,

            dx_window: IndicatorSeries::new(period),
        }
    }

    /// ======================================================
    /// 计算单次 TR / +DM / -DM
    /// ======================================================
    fn calc_raw(
        &self,
        high: f64,
        low: f64,
        close: f64,
        prev_high: f64,
        prev_low: f64,
        prev_close: f64,
    ) -> (f64, f64, f64) {
        let up_move = high - prev_high;
        let down_move = prev_low - low;

        let plus_dm = if up_move > down_move && up_move > 0.0 {
            up_move
        } else {
            0.0
        };

        let minus_dm = if down_move > up_move && down_move > 0.0 {
            down_move
        } else {
            0.0
        };

        let tr1 = high - low;
        let tr2 = (high - prev_close).abs();
        let tr3 = (low - prev_close).abs();

        let tr = tr1.max(tr2).max(tr3);

        let _ = close; // 保留参数语义

        (tr, plus_dm, minus_dm)
    }

    /// ======================================================
    /// 由平滑值计算输出
    /// ======================================================
    fn build_output(&self, tr: f64, plus_dm: f64, minus_dm: f64, adx: f64) -> AdxOutput {
        // 关键：如果 tr 太小（近乎 0），说明根本没波动，DI 必须是 0
        if tr < 1e-9 {
            return AdxOutput {
                adx,
                plus_di: 0.0,
                minus_di: 0.0,
            };
        }

        let plus_di = 100.0 * plus_dm / tr;
        let minus_di = 100.0 * minus_dm / tr;

        AdxOutput {
            adx,
            plus_di,
            minus_di,
        }
    }

    /// ======================================================
    /// 写入结果（与你 MACD 风格一致）
    /// ======================================================
    fn write_result(&mut self, candle: &Candle, value: AdxOutput) {
        let bar_time = candle.open_time;

        match self.preview_bar_time {
            // A. 身份证号一致：永远只更新当前槽位
            Some(t) if t == bar_time => {
                self.results.update_latest(value);
            }
            // B. 身份证号变了（跨 Bar）或初始化：开新坑
            _ => {
                self.results.push(value);
                self.preview_bar_time = Some(bar_time);
            }
        }
    }
}

impl Indicator for ADX {
    type Output = AdxOutput;

    /// ======================================================
    /// update：核心更新逻辑
    /// ======================================================
    fn update(&mut self, candle: &Candle) {
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
        // 2. 第一根K线：仅记录基准
        // ==================================================
        if self.prev_close.is_none() {
            if is_closed {
                self.prev_high = Some(high);
                self.prev_low = Some(low);
                self.prev_close = Some(close);
            }
            return;
        }

        let prev_high = self.prev_high.unwrap();
        let prev_low = self.prev_low.unwrap();
        let prev_close = self.prev_close.unwrap();

        let (tr, plus_dm, minus_dm) =
            self.calc_raw(high, low, close, prev_high, prev_low, prev_close);

        // ==================================================
        // 3. 初始化阶段：累计 period 个样本
        // ==================================================
        if self.smoothed_tr.is_none() {
            if !is_closed {
                return;
            }

            self.tr_sum += tr;
            self.plus_dm_sum += plus_dm;
            self.minus_dm_sum += minus_dm;
            self.warmup_count += 1;

            self.prev_high = Some(high);
            self.prev_low = Some(low);
            self.prev_close = Some(close);

            if self.warmup_count < self.period {
                return;
            }

            self.smoothed_tr = Some(self.tr_sum);
            self.smoothed_plus_dm = Some(self.plus_dm_sum);
            self.smoothed_minus_dm = Some(self.minus_dm_sum);

            let plus_di = 100.0 * self.plus_dm_sum / self.tr_sum;
            let minus_di = 100.0 * self.minus_dm_sum / self.tr_sum;

            let dx = if plus_di + minus_di == 0.0 {
                0.0
            } else {
                100.0 * (plus_di - minus_di).abs() / (plus_di + minus_di)
            };

            self.dx_window.push(dx);
            return;
        }

        // ==================================================
        // 4. Wilder 平滑计算（preview / confirmed 都可实时算）
        // ==================================================
        let base_tr = self.smoothed_tr.unwrap();
        let base_plus_dm = self.smoothed_plus_dm.unwrap();
        let base_minus_dm = self.smoothed_minus_dm.unwrap();

        let next_tr = base_tr - (base_tr / self.period as f64) + tr;
        let next_plus_dm = base_plus_dm - (base_plus_dm / self.period as f64) + plus_dm;
        let next_minus_dm = base_minus_dm - (base_minus_dm / self.period as f64) + minus_dm;

        let plus_di = if next_tr == 0.0 {
            0.0
        } else {
            100.0 * next_plus_dm / next_tr
        };

        let minus_di = if next_tr == 0.0 {
            0.0
        } else {
            100.0 * next_minus_dm / next_tr
        };

        let dx = if plus_di + minus_di == 0.0 {
            0.0
        } else {
            100.0 * (plus_di - minus_di).abs() / (plus_di + minus_di)
        };

        // ==================================================
        // 5. 首个 ADX：先收集 period 个 DX
        // ==================================================
        if self.adx_value.is_none() {
            if is_closed {
                self.dx_window.push(dx);

                self.smoothed_tr = Some(next_tr);
                self.smoothed_plus_dm = Some(next_plus_dm);
                self.smoothed_minus_dm = Some(next_minus_dm);

                if self.dx_window.len() == self.period {
                    let mut sum = 0.0;
                    for i in 0..self.dx_window.len() {
                        if let Some(v) = self.dx_window.get(i) {
                            sum += *v;
                        }
                    }

                    let first_adx = sum / self.period as f64;
                    self.adx_value = Some(first_adx);
                    self.ready = true;

                    let output = self.build_output(next_tr, next_plus_dm, next_minus_dm, first_adx);

                    self.write_result(&candle, output);
                }
            } else {
                // 未就绪且非收盘，不进行预览计算
                return;
            }
        } else {
            // ==================================================
            // 6. 已进入正式实时阶段
            // ==================================================
            let base_adx = self.adx_value.unwrap();
            let next_adx = ((base_adx * (self.period as f64 - 1.0)) + dx) / self.period as f64;

            let output = self.build_output(next_tr, next_plus_dm, next_minus_dm, next_adx);

            self.write_result(&candle, output);

            // ==================================================
            // 7. 收盘后提交真实状态
            // ==================================================
            if is_closed {
                self.smoothed_tr = Some(next_tr);
                self.smoothed_plus_dm = Some(next_plus_dm);
                self.smoothed_minus_dm = Some(next_minus_dm);
                self.adx_value = Some(next_adx);
            }
        }

        // 重要修复点：无论处于哪个阶段，收盘时必须更新基准价格，
        // 且由于删除了步骤 5 的 return，初始化完成的那一根能走到这里。
        if is_closed {
            self.prev_high = Some(high);
            self.prev_low = Some(low);
            self.prev_close = Some(close);
        }
    }

    /// ======================================================
    /// latest：返回当前最新值（实时）
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

    // 辅助工具：创建一个基础 Candle
    fn create_canle(open_time: u64, high: f64, low: f64, close: f64, closed: bool) -> Candle {
        Candle {
            open_time,
            high,
            low,
            close,
            volume: 1000.0, // ADX 不使用成交量，但设为默认值
            closed,
            ..Default::default()
        }
    }

    #[test]
    fn test_adx_warmup_flow() {
        let period = 3;
        let mut adx = ADX::new(period,1000);

        // 1. 第一根 K 线：仅作为 prev 基准，不产生任何计算
        adx.update(&create_canle(1000, 100.0, 90.0, 95.0, true));
        assert!(adx.latest().is_none());

        // 2. 进入 TR/DM 累加阶段 (需 period = 3 个样本)
        // 此时已收 1 根，还需 2 根
        adx.update(&create_canle(2000, 110.0, 105.0, 108.0, true)); // Warmup Count = 1
        adx.update(&create_canle(3000, 115.0, 112.0, 114.0, true)); // Warmup Count = 2
        adx.update(&create_canle(4000, 120.0, 118.0, 119.0, true)); // Warmup Count = 3 (初始化 TR/DM 平滑种子)

        // 此时 adx_value 仍为 None，因为正在收集 dx_window 里的 DX
        assert!(adx.latest().is_none());

        // 3. 进入 DX 收集阶段 (需再收集 period = 3 个 DX 才能产生第一个 ADX)
        adx.update(&create_canle(5000, 125.0, 123.0, 124.0, true)); // DX 1
        adx.update(&create_canle(6000, 130.0, 128.0, 129.0, true)); // DX 2
        adx.update(&create_canle(7000, 135.0, 133.0, 134.0, true)); // DX 3 -> 第一个 ADX 产生

        assert!(adx.ready);
        let first_result = adx.latest().unwrap();
        assert!(first_result.adx > 0.0);
        println!("First ADX: {:?}", first_result);
    }

    #[test]
    fn test_adx_preview_isolation() {
        let period = 14;
        let mut adx = ADX::new(period, 1000);
        let mut time = 1000;

        // 1. 稳定预热：温和上涨
        for i in 0..40 {
            let p = 100.0 + i as f64;
            adx.update(&create_canle(time, p + 1.0, p, p + 0.5, true));
            time += 1000;
        }

        let confirmed_val = adx.latest().unwrap();

        // 2. 构造一个“纯净”的预览上涨
        // 关键：High 显著上移，但 Low 不能下移（防止产生 Minus_DM），且 Close 维持在高位
        let last_c = 100.0 + 39.0 + 0.5;
        adx.update(&create_canle(
            time,
            last_c + 10.0,
            last_c - 0.1,
            last_c + 9.0,
            false,
        ));

        let preview_val = adx.latest().unwrap();

        println!(
            "Confirmed Plus_DI: {}, Preview Plus_DI: {}",
            confirmed_val.plus_di, preview_val.plus_di
        );

        // 验证预览值生效了且不等于确认值
        assert!(preview_val.plus_di != confirmed_val.plus_di);
        // 在这种纯上涨构造下，Plus_DI 必须增加
        assert!(preview_val.plus_di > confirmed_val.plus_di);
    }

    #[test]
    fn test_adx_extreme_swing() {
        let period = 14;
        let mut adx = ADX::new(period, 1000);

        // 预热
        for i in 0..30 {
            adx.update(&create_canle(
                i * 1000,
                100.0 + i as f64,
                90.0 + i as f64,
                95.0 + i as f64,
                true,
            ));
        }

        // 突然出现一个巨大的向下跳空
        adx.update(&create_canle(31000, 50.0, 40.0, 45.0, true));

        let res = adx.latest().unwrap();
        // 巨大的下跌应导致 minus_di 暴增
        assert!(res.minus_di > res.plus_di);
        println!("Extreme Downswing: {:?}", res);
    }

    #[test]
    fn test_adx_idempotency_via_context() {
        let mut adx = ADX::new(14, 1000);
        let c1 = &create_canle(1000, 100.0, 90.0, 95.0, true);
        let c2 = &create_canle(2000, 110.0, 100.0, 105.0, true);

        adx.update(c1);
        adx.update(c2);

        let state_after_c2 = adx.smoothed_tr;

        // 重复发送 c2（ closed 数据）
        adx.update(c2);

        // 校验器 ctx 应拦截重复数据，状态不应改变
        assert_eq!(adx.smoothed_tr, state_after_c2);
    }
}
