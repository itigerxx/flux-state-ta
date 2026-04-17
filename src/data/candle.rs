#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candle {
    /// K线起始时间戳（毫秒或秒单位），通常作为该 Bar 的唯一标识
    pub open_time: u64,
    /// K线发生的时间
    pub event_time: u64,
    /// 开盘价
    pub open: f64,
    /// 最高价
    pub high: f64,
    /// 最低价
    pub low: f64,
    /// 收盘价（如果是未收盘的 K线，则为当前最新价）
    pub close: f64,
    /// 成交量
    pub volume: f64,
    /// 该 K线是否已完成（true 表示已收盘，false 表示该 Bar 仍在变动中）
    pub closed: bool,
}

impl Default for Candle {
    fn default() -> Self {
        Self {
            open_time: 0,
            event_time: 0,
            open: 0.0,
            high: 0.0,
            low: 0.0,
            close: 0.0,
            volume: 0.0,
            closed: false,
        }
    }
}
