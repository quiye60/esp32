//! 每课通用的导入清单：`use esp_s3::prelude::*;` 就够了。
//!
//! 这里只做「转发」：把常用类型从各个 crate 里搬到一个地方，
//! 避免每个课文件都写一长串 `use`。

pub use crate::boot;
pub use crate::pins;

// embassy：任务、时间、执行器句柄
pub use embassy_executor::{Spawner, task};
pub use embassy_time::{Duration, Instant, Ticker, Timer};

// esp-hal：阻塞延时、GPIO、定时器组
pub use esp_hal::delay::Delay;
pub use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
pub use esp_hal::timer::timg::TimerGroup;

// RTT 打印（probe-rs 看输出），`rtt_init_print!` 已被 boot::init() 调用
pub use rtt_target::{rprintln, rtt_init_print};
