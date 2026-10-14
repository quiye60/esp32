// ============================================================================
// 17 · 时间 API 全景：Instant / Delay / Rate / Timer / Ticker / with_timeout
//
// 目标：分清四套时间工具各自用在哪：
//         esp_hal::time::Instant —— 阻塞场景测耗时、看开机多久
//         Delay                  —— 阻塞式延时（忙等）
//         esp_hal::time::{Rate, Duration} —— 外设配置里描述「频率 / 时长」
//         embassy_time::{Timer, Ticker, Instant, with_timeout} —— 异步定时
//
// 参考：本项目 teach/05-时间_延时_超时.md、esp-hal/src/time.rs、esp-hal/src/delay.rs
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件
// 运行：cargo run --release --bin 17_time_delay
// 预期：依次打印阻塞延时实测耗时、频率换算、异步等待、Ticker 节拍、超时结果
// 坑：  embassy 的 Timer 依赖 esp_rtos::start 提供的时基，顺序不能反
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;                 // main 宏要求的形参类型
use embassy_time::{Duration, Instant, Ticker, Timer}; // 异步时间工具
use esp_hal::clock::CpuClock;                  // CPU 频率
use esp_hal::delay::Delay;                     // 阻塞式延时
// esp-hal 的 Instant / Duration 名字和 embassy 的撞车了，所以起别名区分
use esp_hal::time::{Duration as HalDuration, Instant as HalInstant, Rate};
use esp_hal::timer::timg::TimerGroup;          // TIMG：给调度器提供时基
use rtt_target::rprintln;                      // RTT 打印

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    rtt_target::rtt_init_print!();             // 打开 RTT

    // 初始化芯片，拿到外设句柄
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // ---------------- 1. 阻塞延时 + 实测耗时 ----------------
    let delay = Delay::new();                  // 没有参数：它不占外设

    let start = HalInstant::now();             // 记下开始时刻（esp-hal 的 Instant）
    delay.delay_millis(200);                   // 忙等 200 毫秒
    // elapsed() 返回 esp-hal 的 Duration，as_micros() 取微秒
    rprintln!("1) delay_millis(200) 实测 {} us", start.elapsed().as_micros());

    // 更细的粒度：微秒、纳秒（纳秒受 240 MHz 主频限制，实际分辨率约 4 ns）
    delay.delay_micros(1500);
    delay.delay_nanos(500);
    // duration_since_epoch() = 开机到现在的时长
    rprintln!(
        "   开机至今 {} ms（Instant::duration_since_epoch）",
        HalInstant::now().duration_since_epoch().as_millis()
    );

    // ---------------- 2. 频率类型 Rate ----------------
    // esp-hal 的外设配置（LEDC / SPI / TWAI...）用 Rate 表达频率，而不是裸整数
    let freq = Rate::from_khz(24);
    rprintln!("2) Rate::from_khz(24) = {} Hz", freq.as_hz());
    // HalDuration 则用来表达「时长」，两者是外设配置里的常见搭档
    let _timeout = HalDuration::from_millis(1500); // 例如：配置里表示 1500 ms 超时
    rprintln!("   HalDuration::from_millis(1500) 在配置里表示 1500 ms 超时");

    // ---------------- 3. 异步定时 ----------------
    // 先给 embassy 装时基，Timer / Ticker 才能工作
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    rprintln!("3) Timer::after(500ms) 开始等…");
    Timer::after(Duration::from_millis(500)).await; // 睡 500 ms（让出 CPU）
    rprintln!("   等到了，开机 {} ms", Instant::now().as_millis());

    // Ticker：固定周期触发。它按「上次计划的时间」推进，不会因为处理慢而累积漂移
    let mut ticker = Ticker::every(Duration::from_millis(250));
    for n in 1..=4u32 {
        ticker.next().await;                   // 等下一拍
        rprintln!("   Ticker 第 {} 拍，开机 {} ms", n, Instant::now().as_millis());
    }

    // with_timeout：给任意 Future 套一个超时；超时返回 Err，内部 Future 被取消
    let result = embassy_time::with_timeout(
        Duration::from_millis(300),            // 最多等 300 ms
        Timer::after(Duration::from_secs(1)),  // 但这个要睡 1 s → 必然超时
    )
    .await;
    rprintln!(
        "4) with_timeout(300ms, 等1s) → {}",
        if result.is_err() { "超时 Err（预期）" } else { "Ok（不该出现）" }
    );

    // 演示完就停下来，避免刷屏
    rprintln!("17 课演示结束，程序进入空闲循环");
    loop {
        Timer::after(Duration::from_secs(10)).await; // 定期醒来一次，其余时间闲着
    }
}
