// ============================================================================
// 36 · 深睡眠：把芯片睡到 µA 级，再定时或按键叫醒
//
// 目标：电池设备的核心技能。本课演示完整的「睡 → 醒」循环：
//         1) 醒来第一件事：查 reset reason + wakeup cause（上电？定时器？按键？）
//         2) LowPower::new(peripherals.LPWR)
//         3) 定时唤醒：set_wakeup_deadline(绝对时刻) —— 参数是时间点，不是时长
//         4) GPIO 唤醒：apply_wakeup_config + listen(Event::LowLevel)，
//            醒来后用 caused_wakeup() 判断是哪个脚把芯片叫起来的
//         5) sleep_deep() 之后代码不会再往下执行 —— 醒来等于重新启动
//       这一条要记牢：**深睡眠醒来 = 重启**，程序从头跑，RAM 内容不作数
//
// 参考：官方 qa-test/src/bin/sleep_timer.rs、sleep_timer_gpio.rs
//       结构照 src/bin/tasks.rs
//
// 接线：GPIO17 用跳线接 GND 当作「唤醒按键」（S3 上能唤醒的脚是 GPIO0~21）
// 运行：cargo run --release --bin 36_sleep
//       想看真实功耗：把 RTT/调试器拔掉再测（调试器本身就是几 mA）
// 预期：每次启动打印「为什么醒来」，然后睡 5 秒；期间按下 GPIO17 会立刻醒
// 坑：  ① 定时唤醒和 GPIO 唤醒谁先到算谁，靠 wakeup_cause() 区分
//       ② 深睡眠会关掉 Wi-Fi/BLE；想「睡眠中保持连接」要用浅睡眠/自动睡眠
//          （见 teach/10-无线-WiFi与BLE骨架.md 里 embassy_auto_light_sleep 的思路）
//       ③ RTC 域的数据能跨睡眠保留（可用于跨睡眠计数），主 RAM 别依赖
//       ④ 睡之前把 RTT 打印 flush 掉（下面用阻塞延时等一会儿）
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Event, Input, InputConfig, Pull, WakeupConfig};
use esp_hal::rtc_cntl::{
    SocResetReason,
    reset_reason,
    sleep::{LowPower, RtcSleepConfig},
    wakeup_cause,
};
use esp_hal::system::Cpu;
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

    // ---------------- 1. 搞清楚「我为什么醒来」 ----------------
    let reason = reset_reason(Cpu::ProCpu).unwrap_or(SocResetReason::ChipPowerOn);
    rprintln!("复位原因：{:?}", reason);
    rprintln!("唤醒原因：{:?}", wakeup_cause());   // ChipPowerOn / Timer / GPIO ...

    // ---------------- 2. GPIO 唤醒配置 ----------------
    // GPIO17 接按键一端，另一端接 GND；内部上拉保证平时是高电平
    let mut wake_pin = Input::new(peripherals.GPIO17, InputConfig::default().with_pull(Pull::Up));

    // 上一次睡眠是不是这个脚叫醒的？
    rprintln!("GPIO17 是否唤醒了本次启动：{}", wake_pin.caused_wakeup());

    // 低电平唤醒（按下 = 低电平）；low_power_path = 走 LP 域比较器，更省电
    let wake_cfg = WakeupConfig::default().with_low_power_path(true);
    wake_pin.apply_wakeup_config(&wake_cfg).unwrap();
    wake_pin.listen(Event::LowLevel);      // 「触发条件」就是唤醒条件
    // 想同时支持高电平唤醒，就再准备一个下拉引脚并 listen(Event::HighLevel)

    // ---------------- 3. 设置定时唤醒 ----------------
    let mut lpwr = LowPower::new(peripherals.LPWR);
    // 注意是「绝对时刻」= now + 5 秒；写成时长容易被前面的延时吃掉
    lpwr.set_wakeup_deadline(Instant::now() + Duration::from_secs(5));
    rprintln!("5 秒后自动醒来；这期间按下 GPIO17 会立刻醒（更省电）");

    // ---------------- 4. 睡下去 ----------------
    // 给 RTT 一点时间把上面的内容发出去，否则会「睡太快，什么都没打出来」
    let delay = Delay::new();
    delay.delay_millis(300);
    rprintln!("睡觉……（醒来后会从程序第一行重新开始，变量都没了）");
    delay.delay_millis(100);

    lpwr.sleep_deep(RtcSleepConfig::deep());   // 不会返回：唤醒后芯片重启

    // 只有「睡眠失败」才会走到这里
    #[allow(unreachable_code)]
    loop {}
}
