//! 芯片初始化与（可选的）调度器启动。
//!
//! 所有课程的第一句基本都是 `let peripherals = esp_s3::boot::init();`：
//! 它一次性完成「打开 RTT 打印 → 配好 CPU 频率 → 初始化芯片」。

use esp_hal::clock::CpuClock;
use esp_hal::peripherals::Peripherals;

/// 初始化芯片，并把所有外设句柄交给你。
///
/// 这个函数在固件的整个生命周期里**只能调用一次**：`esp_hal::init` 会
/// 真正去配时钟、中断、看门狗等，调第二次会 panic。
///
/// 注意：它**不**启动 esp-rtos 调度器。要在 async main 里用
/// `Timer` / `Ticker` / 多任务，还得按各课样板区那样再补两行：
///
/// ```ignore
/// let timg0 = TimerGroup::new(peripherals.TIMG0);
/// esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);
/// ```
pub fn init() -> Peripherals {
    // 先打开 RTT：越早越好，这样后面任何打印都能被 probe-rs 收到
    rtt_target::rtt_init_print!();

    // CPU 跑最高频（ESP32-S3 是 240 MHz），并让协议栈/外设都用默认配置
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());

    // 初始化芯片：返回 Peripherals，里面每个字段就是一个外设的「钥匙」
    esp_hal::init(config)
}
