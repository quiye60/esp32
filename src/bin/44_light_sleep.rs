// ============================================================================
// 44 · 浅睡眠：省电，但醒来后接着往下跑（不是重启）
//
// 关于课程表里的「LP 核 + ETM」，先说实话（对 S3 这两个都需要额外条件）：
//   · LP 核（ULP-RISC-V）：要另外写一个跑在 LP 核上的程序、单独编译成固件再塞进
//     主程序（官方 lp_blinky 需要改 build.rs + 第二个 crate）。本项目 build.rs 是
//     所有课共用的，硬塞会拖慢每次编译、没装 RISC-V 工具链时还会直接报错，
//     所以留到专门做低功耗项目时再补（讲义 teach/44-lp_core与etm.md）。
//   · ETM（事件任务矩阵）：ESP32-S3 的 esp-hal 里 `etm_driver_supported = false`
//     —— 这颗芯片根本没有 ETM 外设（C6/H2/P4 才有），所以写不出来。
// 本课改讲 S3 上真正实用、且和第 36 课（深睡眠）互补的主题：**浅睡眠**。
//
// 深睡眠 vs 浅睡眠（本课的核心对比）：
//   深睡眠：几乎关掉一切（µA 级）→ 醒来 = 复位重启，RAM 里的东西全丢
//   浅睡眠：关掉 CPU 和大部分外设，RAM/寄存器保留 → 醒来 = **从 sleep 那一行继续跑**
//           （省电没深睡眠那么极致，但代码逻辑简单得多）
//
// 目标：
//   1) LowPower::new(peripherals.LPWR)
//   2) set_wakeup_deadline(绝对时刻) —— 定时唤醒
//   3) gpio.listen(Event::LowLevel) + apply_wakeup_config —— 引脚唤醒
//   4) sleep_light(...) 返回后，用一个递增的计数器证明「我们回到了原来的现场」
//
// 参考：官方 qa-test/src/bin/sleep_timer.rs、sleep_timer_gpio.rs
//       esp-hal/src/rtc_cntl/sleep/mod.rs
//       结构照 src/bin/tasks.rs
//
// 接线：GPIO17 用跳线接 GND 当作「唤醒按键」
// 运行：cargo run --release --bin 44_light_sleep
// 预期：每轮打印「第 N 次醒来，计数器 = N」——**计数器在涨**说明 RAM 没丢、
//       程序没有重启；按 GPIO17 可以立刻叫醒，观察 wakeup_cause
// 坑：  ① 浅睡眠会停掉大部分外设时钟，醒来后某些外设需要重新初始化
//       ② 调试器连着时睡眠可能被拒绝（sleep_light 内部会检查并拒绝），
//          看不到现象就先把 probe-rs 的 RTT 窗口关掉再看
//       ③ 定时唤醒用「绝对时刻」，和深睡眠一样是 now + 时长
//       ④ 想省电又保持 Wi-Fi 连接，用自动浅睡眠（embassy_auto_light_sleep 例程）
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Event, Input, InputConfig, Pull, WakeupConfig};
use esp_hal::rtc_cntl::{
    sleep::{LowPower, RtcSleepConfig},
    wakeup_cause,
};
use esp_hal::time::{Duration, Instant};
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

    // 唤醒按键：GPIO17 接 GND 就醒（内部上拉，平时高电平）
    let mut wake_pin = Input::new(peripherals.GPIO17, InputConfig::default().with_pull(Pull::Up));
    wake_pin.apply_wakeup_config(&WakeupConfig::default().with_low_power_path(true)).unwrap();
    wake_pin.listen(Event::LowLevel);

    let mut lpwr = LowPower::new(peripherals.LPWR);

    // 关键证据：这个计数器活在 RAM 里。每轮醒来 +1 就说明「没有重启」
    let mut wake_count = 0u32;

    rprintln!("浅睡眠实验开始：每 3 秒自动醒一次，按 GPIO17 可立即唤醒");

    loop {
        wake_count += 1;
        rprintln!("第 {} 轮：准备睡觉（计数器 = {}）", wake_count, wake_count);
        rprintln!("  上次唤醒原因：{:?}", wakeup_cause());

        // 睡 3 秒（绝对时刻：now + 3s）
        lpwr.set_wakeup_deadline(Instant::now() + Duration::from_secs(3));

        // 给 RTT 一点时间把日志发出去，然后进入浅睡眠
        Timer::after(EmbassyDuration::from_millis(200)).await;
        lpwr.sleep_light(RtcSleepConfig::default());   // ← 醒来后从这里继续执行！

        // 走到这里说明醒来了：CPU 继续用同一个栈、同一个局部变量
        rprintln!(
            "第 {} 轮醒来：唤醒原因 = {:?}，GPIO17 是否触发 = {}",
            wake_count,
            wakeup_cause(),
            wake_pin.caused_wakeup()
        );

        // 对比第 36 课的深睡眠：那边每次醒来都是「复位原因 + 计数器从 0 开始」，
        // 这边计数器一直在累加 —— 这就是浅睡眠与深睡眠最本质的区别
        Timer::after(EmbassyDuration::from_millis(500)).await;
    }
}
