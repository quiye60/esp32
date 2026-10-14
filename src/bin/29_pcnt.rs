// ============================================================================
// 29 · PCNT：硬件脉冲计数器（按键计数 / 编码器测速）
//
// 目标：把「数脉冲」这件小事交给硬件，CPU 不用守着：
//         Input::new(...).peripheral_input()  —— 把引脚信号直接接到 PCNT（外设间连线）
//         set_input_mode(负沿动作, 正沿动作)    —— 上升/下降沿是加还是减
//         set_filter(阈值)                     —— 硬件滤掉窄毛刺
//         counter.get()                        —— 随时读当前计数值
//       最后谈怎么用它测转速：单位时间读两次计数，差值就是频率。
//
// 参考：esp-hal/src/pcnt/mod.rs 的文档示例（正交编码器版）
//       结构照 src/bin/tasks.rs
//
// 接线：按键一端接 GPIO9，另一端接 GND（内部上拉，按下为低）
//       想验证计数精度：把 GPIO9 用跳线接到「27 课 PWM 输出（GPIO2）」，
//       PWM 跑 2 kHz 时每 500 ms 应该数到 1000 左右
// 运行：cargo run --release --bin 29_pcnt
// 预期：每 500 ms 打印一次计数值；按一下键，计数 +1（机械抖动可能多加几次）
// 坑：  ① PCNT 的硬件滤波只对付 <13 µs 的毛刺（阈值 1023 @80 MHz），
//          机械按键抖动是毫秒级，压不下去 —— 要么软件去抖，要么加 RC 电路
//       ② 计数范围是 i16（-32768..32767），超了会溢出，长时间测速要定期清零
//       ③ 想要「按一次只加一」最稳的做法还是按键中断 + 软件去抖（见第 16 课）
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Pull};
use esp_hal::pcnt::{Pcnt, channel::EdgeMode};
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

    // ---------------- 1. 建 PCNT，拿一个「计数单元」 ----------------
    let pcnt = Pcnt::new(peripherals.PCNT);
    let unit0 = pcnt.unit0;                 // S3 有 2 个单元，每单元 2 个通道

    // 计数上下限：到界就触发事件（本课不用中断，设一下更安全）
    unit0.set_low_limit(Some(-30000)).unwrap();
    unit0.set_high_limit(Some(30000)).unwrap();

    // 硬件滤波：阈值越大越能滤掉长毛刺，上限 1023（约 12.8 µs @80 MHz）
    unit0.set_filter(Some(1023)).unwrap();
    unit0.clear();                          // 计数清零

    // ---------------- 2. 把引脚信号接到通道上 ----------------
    // GPIO9 是内部上拉的按键输入；peripheral_input() 取出「外设输入信号」，
    // 这样 PCNT 在硬件层就能看到电平变化，不经过 GPIO 驱动
    let button = Input::new(peripherals.GPIO9, InputConfig::default().with_pull(Pull::Up));
    unit0.channel0.set_edge_signal(button.peripheral_input());

    // 按下（1 → 0，负沿）时 +1；松开（0 → 1）不动
    unit0.channel0.set_input_mode(EdgeMode::Increment, EdgeMode::Hold);

    // ---------------- 3. 开始计数 ----------------
    unit0.resume();
    rprintln!("PCNT 已启动，请按 GPIO9 上的按键（每 500 ms 报一次计数）");

    let mut last_value: i32 = 0;
    let mut last_time = Instant::now();

    loop {
        Timer::after(Duration::from_millis(500)).await;

        let value = unit0.counter.get() as i32;
        let elapsed_ms = last_time.elapsed().as_millis();
        let delta = value - last_value;

        // 有了「一段时间内的脉冲数」，除以时间就是频率（测转速就是这个原理）
        rprintln!(
            "计数 = {:>6}（这段 +{}，等效 {:.1} Hz）",
            value,
            delta,
            delta as f32 * 1000.0 / elapsed_ms as f32
        );

        last_value = value;
        last_time = Instant::now();
    }
}
