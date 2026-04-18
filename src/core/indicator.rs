use crate::data::candle::Candle;

pub trait Indicator {
    type Output;

    /// 输入数据，更新指标状态
    fn update(&mut self, candle: &Candle);

    /// 获取当前最新指标值
    fn latest(&self) -> Option<Self::Output>;

    /// 获取最近 N 个指标值（按时间顺序）
    fn last_n(&self, n: usize) -> Vec<Self::Output>;
}
