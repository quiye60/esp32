//! 板级引脚约定：**要改接线，只改这个文件**，所有课程都会跟着变。
//!
//! 默认约定（沿用本项目 teach/03、08、09 的写法）：
//!
//! | 器件 | GPIO | 电路 | 说明 |
//! | --- | --- | --- | --- |
//! | LED | GPIO2 | 引脚 → 限流电阻 → LED → GND | 高电平点亮（改成板载 LED 也是这里改） |
//! | 按键 | GPIO9 | 引脚 → 按键 → GND | 用内部上拉，按下为低电平 |
//! | 电位器 | GPIO1 | 两端接 3V3/GND，中间抽头接 GPIO1 | ADC1 通道 0，用于 ADC 课 |
//!
//! 三个常量是给讲义和代码注释对照用的；真正拿句柄时写
//! `peripherals.GPIO2` 之类，编号与这里的常量保持一致即可。

use esp_hal::gpio::{Input, InputConfig, InputPin, Level, Output, OutputConfig, OutputPin, Pull};

/// LED 所在的 GPIO 编号（默认 2）。
pub const LED_GPIO: u8 = 2;
/// 按键所在的 GPIO 编号（默认 9）。
pub const BUTTON_GPIO: u8 = 9;
/// 电位器所在的 GPIO 编号（默认 1，属于 ADC1）。
pub const POT_GPIO: u8 = 1;

/// 按项目约定创建一个 LED 输出：初始灭、推挽、20 mA 驱动能力。
///
/// 用法：`let mut led = pins::led(peripherals.GPIO2);`
pub fn led(pin: impl OutputPin + 'static) -> Output<'static> {
    // Level::Low = 初始低电平（LED 灭）；推挽 = 能主动输出高/低，不靠外部上拉
    Output::new(pin, Level::Low, OutputConfig::default())
}

/// 按项目约定创建一个按键输入：打开内部上拉，所以按下（对地）读到低电平。
///
/// 用法：`let mut button = pins::button(peripherals.GPIO9);`
pub fn button(pin: impl InputPin + 'static) -> Input<'static> {
    // with_pull(Pull::Up) = 打开芯片内部上拉电阻，省掉外部上拉
    Input::new(pin, InputConfig::default().with_pull(Pull::Up))
}
