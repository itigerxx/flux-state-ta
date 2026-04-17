use std::mem::replace;

/// ======================================================
/// 指标序列容器（IndicatorSeries）
/// ======================================================
///
/// ======================================================
/// 核心定位
/// ======================================================
///
/// 这是一个“面向流式K线指标计算”的固定长度时间序列结构：
///
/// 👉 EMA / SMA / RSI / MACD / 自定义指标的状态存储器
///
/// 它不是：
/// - Queue（队列）
/// - RingBuffer（通用环形队列）
///
/// 它是：
///
/// 👉 “支持实时修正的流式时间序列窗口”
///
/// ======================================================
/// 设计目标
/// ======================================================
///
/// 1. 固定容量（capacity）
/// 2. 自动覆盖最旧数据（滑动窗口）
/// 3. O(1 push / update_latest / get）
/// 4. 支持 update_latest（流式K线修正）
/// 5. 无 VecDeque（避免双段内存）
/// 6. WASM 友好（连续线性内存）
///
/// ======================================================
/// 使用场景
/// ======================================================
///
/// - EMA / SMA / RSI / MACD
/// - rolling window calculations
/// - crossover 判断
/// - 策略信号计算
/// - 高频 tick / candle 混合驱动指标
///
/// ======================================================
#[derive(Debug, Clone)]
pub struct IndicatorSeries<T: Clone> {
    buf: Vec<T>,
    cap: usize,
    len: usize,
    head: usize,
}

/// ======================================================
/// 迭代器实现
/// ======================================================
pub struct IndicatorSeriesIter<'a, T: Clone> {
    series: &'a IndicatorSeries<T>,
    index: usize, // 当前逻辑索引
}

impl<'a, T: Clone> Iterator for IndicatorSeriesIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.series.len() {
            let real = self.series.logical_to_physical(self.index);
            let res = self.series.buf.get(real);
            self.index += 1;
            res
        } else {
            None
        }
    }

    // 提供准确的剩余长度，有助于优化如 collect 等操作
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.series.len();
        let remaining = len.saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

impl<'a, T: Clone> ExactSizeIterator for IndicatorSeriesIter<'a, T> {}

impl<'a, T: Clone> IntoIterator for &'a IndicatorSeries<T> {
    type Item = &'a T;
    type IntoIter = IndicatorSeriesIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T: Clone> IndicatorSeries<T> {
    /// ======================================================
    /// 创建新的指标序列
    /// ======================================================
    ///
    /// # 参数
    /// - capacity: 窗口最大长度（固定）
    ///
    /// # 行为
    /// - 预分配容量
    /// - 不预填充数据
    /// - 初始化空窗口
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);

        Self {
            buf: Vec::with_capacity(capacity),
            cap: capacity,
            len: 0,
            head: 0,
        }
    }

    /// ======================================================
    /// 返回一个迭代器（从最旧到最新）
    /// ======================================================
    pub fn iter(&self) -> IndicatorSeriesIter<'_, T> {
        IndicatorSeriesIter {
            series: self,
            index: 0,
        }
    }

    /// ======================================================
    /// 当前窗口长度
    /// ======================================================
    ///
    /// # 返回
    /// 当前有效数据数量（<= cap）
    pub fn len(&self) -> usize {
        self.len
    }

    /// ======================================================
    /// 是否已达到最大容量
    /// ======================================================
    ///
    /// # 返回
    /// true  → 窗口已满（之后 push 会覆盖旧值）
    /// false → 仍在增长阶段
    pub fn is_full(&self) -> bool {
        self.len == self.cap
    }

    /// ======================================================
    /// push（closed candle 写入）
    /// ======================================================
    ///
    /// # 语义
    /// closed K线进入指标窗口的标准入口
    ///
    /// # 行为
    /// 1. 未满：
    ///    - 直接追加到窗口尾部
    ///    - len + 1
    ///    - 不产生覆盖
    ///
    /// 2. 已满：
    ///    - 覆盖最旧数据（head 指向位置）
    ///    - head 向前移动一格（循环）
    ///    - 返回被覆盖的旧值
    ///
    /// # 返回值
    /// - Some(T) → 被淘汰的旧值（窗口滑动时有意义）
    /// - None    → 未发生覆盖
    ///
    /// # 时间复杂度
    /// O(1)
    pub fn push(&mut self, value: T) -> Option<T> {
        debug_assert!(self.cap > 0);
        debug_assert!(self.len <= self.cap);
        debug_assert!(self.head < self.cap);

        if self.len < self.cap {
            // 未满：线性追加
            self.buf.push(value);
            self.len += 1;
            return None;
        }

        // 已满：覆盖最旧数据
        let old = replace(&mut self.buf[self.head], value);

        // 移动环形写指针
        self.head = (self.head + 1) % self.cap;

        Some(old)
    }

    /// ======================================================
    /// update_latest（live candle 修正）
    /// ======================================================
    ///
    /// # 语义
    /// 用于未闭合K线的“实时修正行为”
    ///
    /// # 核心用途
    /// - 当前 candle 仍在变动（tick级更新）
    /// - EMA / SMA / RSI 等需要实时波动
    ///
    /// # 行为
    /// - 如果窗口为空 → 自动 push
    /// - 否则 → 覆盖“最新一个逻辑元素”
    ///
    /// # 关键点
    /// 不改变 len
    /// 只修改最后一个元素的值
    pub fn update_latest(&mut self, value: T) {
        debug_assert!(self.len <= self.cap);
        debug_assert!(self.cap > 0);

        if self.len == 0 {
            self.push(value);
            return;
        }

        // 找到逻辑最后一个元素位置
        let last_index = self.len - 1;

        // 转换为物理索引（环形结构）
        let real = self.logical_to_physical(last_index);

        // 覆盖最新值（实时修正）
        self.buf[real] = value;
    }

    /// ======================================================
    /// get（按逻辑索引访问）
    /// ======================================================
    ///
    /// # 索引规则
    /// 0       → 最旧数据
    /// len-1   → 最新数据
    ///
    /// # 返回
    /// - Some(&T) → 存在
    /// - None     → 越界
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }

        let real = self.logical_to_physical(index);
        self.buf.get(real)
    }

    /// ======================================================
    /// last（获取最新一个值）
    /// ======================================================
    ///
    /// # 返回
    /// 当前窗口中最新元素引用
    pub fn last(&self) -> Option<&T> {
        self.get(self.len - 1)
    }

    /// ======================================================
    /// tail（获取最近 N 个数据）
    /// ======================================================
    ///
    /// # 行为
    /// 返回按时间顺序排列的最后 N 个元素
    /// （old → new）
    ///
    /// # 示例
    /// [1,2,3,4,5].tail(3) => [3,4,5]
    pub fn tail(&self, n: usize) -> Vec<&T> {
        let n = n.min(self.len);
        let start = self.len - n;

        (start..self.len).filter_map(|i| self.get(i)).collect()
    }

    /// ======================================================
    /// head（获取最旧 N 个数据）
    /// ======================================================
    ///
    /// # 示例
    /// [1,2,3,4,5].head(2) => [1,2]
    pub fn head(&self, n: usize) -> Vec<&T> {
        let n = n.min(self.len);

        (0..n).filter_map(|i| self.get(i)).collect()
    }

    /// ======================================================
    /// 内部方法：逻辑索引 → 物理索引
    /// ======================================================
    ///
    /// # 原理
    /// ring buffer 映射公式：
    /// (head + index) % cap
    ///
    /// # 说明
    /// 保证逻辑顺序永远是：
    /// oldest → newest
    #[inline]
    fn logical_to_physical(&self, index: usize) -> usize {
        (self.head + index) % self.cap
    }
}

#[cfg(test)]
mod series_physical_tests {
    use super::*;

    #[test]
    fn test_physical_wrap_around_logic() {
        // 1. 创建容量只有 2 的极小容器
        let mut series = IndicatorSeries::new(2);

        // 2. 填满它
        series.push(10.0); // 物理 [10.0], head=0, len=1
        series.push(20.0); // 物理 [10.0, 20.0], head=0, len=2
        
        assert_eq!(series.len(), 2);
        assert_eq!(*series.get(0).unwrap(), 10.0); // 逻辑最旧是 10
        assert_eq!(*series.get(1).unwrap(), 20.0); // 逻辑最新是 20

        // 3. 触发第一次回绕：覆盖物理索引 0 (10.0)
        let old1 = series.push(30.0); // 物理 [30.0, 20.0], head=1, len=2
        
        assert_eq!(old1, Some(10.0)); // 确认吐出来的是被覆盖的 10
        assert_eq!(series.len(), 2);  // 长度不准增加
        assert_eq!(*series.get(0).unwrap(), 20.0); // 逻辑最旧现在变成了 20
        assert_eq!(*series.get(1).unwrap(), 30.0); // 逻辑最新变成了 30

        // 4. 触发第二次回绕：覆盖物理索引 1 (20.0)
        let old2 = series.push(40.0); // 物理 [30.0, 40.0], head=0, len=2
        
        assert_eq!(old2, Some(20.0));
        assert_eq!(*series.get(0).unwrap(), 30.0); // 逻辑最旧是 30
        assert_eq!(*series.get(1).unwrap(), 40.0); // 逻辑最新是 40
    }

    #[test]
    fn test_update_latest_during_wrap_around() {
        let mut series = IndicatorSeries::new(2);
        series.push(1.0);
        series.push(2.0);
        series.push(3.0); // 此时物理是 [3.0, 2.0], head=1, 逻辑最旧是 2.0, 最新是 3.0

        // 修正当前最新的 3.0 为 99.0
        series.update_latest(99.0);

        assert_eq!(*series.get(1).unwrap(), 99.0);
        assert_eq!(*series.get(0).unwrap(), 2.0); // 确保没改错位置
        assert_eq!(series.len(), 2);
    }

    #[test]
    fn test_iterator_after_wrap_around() {
        let mut series = IndicatorSeries::new(3);
        for i in 1..=5 {
            series.push(i as f64);
        }
        
        // 容量 3，推了 5 个数，剩下的应该是 [3.0, 4.0, 5.0]
        let values: Vec<f64> = series.iter().copied().collect();
        assert_eq!(values, vec![3.0, 4.0, 5.0]);
    }

    #[test]
    fn test_tail_and_head_logic() {
        let mut series = IndicatorSeries::new(10);
        for i in 1..=5 {
            series.push(i as f64);
        }

        // 测试 tail：拿最后 2 个
        let tail: Vec<f64> = series.tail(2).into_iter().copied().collect();
        assert_eq!(tail, vec![4.0, 5.0]);

        // 测试 head：拿最前 2 个
        let head: Vec<f64> = series.head(2).into_iter().copied().collect();
        assert_eq!(head, vec![1.0, 2.0]);
    }
}