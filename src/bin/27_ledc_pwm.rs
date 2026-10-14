// ============================================================================
// 27 · LEDC：硬件 PWM，用它做呼吸灯
//
// 目标：摆脱「软件翻转 GPIO + 延时」的土办法，理解硬件 PWM 的三个概念：
//         时钟源（APB 80 MHz）→ 定时器（决定频率和分辨率）→ 通道（决定占空比 + 接哪个脚）
//       再掌握两个 API：
//         set_duty(pct)                      固定占空比（单位 %）
//         start_duty_fade(from, to, ms)      硬件自动渐变（CPU 不用管，这就是「呼吸」）
//
// 参考：esp-hal/src/ledc/mod.rs 的文档示例、esp-hal/src/ledc/channel.rs
//       结构照 src/bin/tasks.rs
//
// 接线：LED → GPIO2（引脚 → 限流电阻 → LED → GND），接示波器/逻辑分析仪能看到方波
// 运行：cargo run --release --bin 27_ledc_pwm
// 预期：LED 先 25% / 50% / 75% 各停 1 秒，然后不停「渐亮 → 渐暗」呼吸
// 坑：  ① 分辨率越高，能支持的最低频率越高（周期 = 2^分辨率 / 频率，分频器有限）
//       ② LEDC 要的是「引脚外设句柄」peripherals.GPIO2，不是 gpio::Output
//          （第 14 课那种 Output 是 GPIO 外设自己的输出，不能给 LEDC 用）
//       ③ 硬件渐变是异步的：is_duty_fade_running() 返回 false 才表示渐变结束
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::DriveMode;                       // 推挽/开漏，这里用推挽
use esp_hal::ledc::channel::{self, ChannelIFace};   // 通道 + 它实现的 trait
use esp_hal::ledc::timer::{self, TimerIFace};       // 定时器 + 它实现的 trait
use esp_hal::ledc::{LSGlobalClkSource, Ledc, LowSpeed}; // 低速通道（所有芯片都有）
use esp_hal::time::Rate;                            // 频率类型
use esp_hal::timer::timg::TimerGroup;
use rtt_target::rprintln;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    rtt_target::rtt_init_print!();

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // ---------------- 1. 拿出 LEDC 外设，选全局时钟源 ----------------
    let mut ledc = Ledc::new(peripherals.LEDC);
    // LEDC 的「低速组」是所有芯片都有的那组，时钟来自 APB（80 MHz）
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);

    // ---------------- 2. 配定时器：决定 PWM 频率与分辨率 ----------------
    let mut lstimer0 = ledc.timer::<LowSpeed>(timer::Number::Timer0);
    lstimer0
        .configure(timer::config::Config {
            // 10 位分辨率 = 占空比分成 1024 档，够细腻
            duty: timer::config::Duty::Duty10Bit,
            clock_source: timer::LSClockSource::APBClk,
            // 2 kHz：给眼睛看够了；要驱动机器/舵机就按器件手册改
            frequency: Rate::from_khz(2),
        })
        .unwrap();

    // ---------------- 3. 配通道：把定时器 + 引脚 + 初始占空比绑起来 ----------------
    // 第一个参数是通道号，S3 有 8 个低速通道，可以各带一路 PWM
    let mut channel0 = ledc.channel(channel::Number::Channel0, peripherals.GPIO2);
    channel0
        .configure(channel::config::Config {
            timer: &lstimer0,                    // 用哪个定时器决定频率
            duty_pct: 25,                        // 初始占空比 25%
            drive_mode: DriveMode::PushPull,     // 推挽输出（能主动拉高和拉低）
        })
        .unwrap();

    // ---------------- 4. 固定占空比：set_duty 的单位是百分比 ----------------
    for pct in [25u8, 50, 75] {
        channel0.set_duty(pct).unwrap();
        rprintln!("占空比 = {}%（万用表测平均电压 ≈ 3.3V × {}%）", pct, pct);
        Timer::after(Duration::from_secs(1)).await;
    }

    rprintln!("开始硬件渐变呼吸（CPU 完全不参与，只等它做完）");

    // ---------------- 5. 硬件渐变：一次配置，硬件自己跑 ----------------
    loop {
        // 从 0% 渐变到 100%，耗时 1000 ms
        channel0.start_duty_fade(0, 100, 1000).unwrap();
        // 轮询等它结束；这里必须 await 让出 CPU，否则会卡死执行器
        while channel0.is_duty_fade_running() {
            Timer::after(Duration::from_millis(20)).await;
        }

        // 再从 100% 回到 0%
        channel0.start_duty_fade(100, 0, 1000).unwrap();
        while channel0.is_duty_fade_running() {
            Timer::after(Duration::from_millis(20)).await;
        }
    }
}
