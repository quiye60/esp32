// ============================================================================
// 25 · 随机数：RNG 外设
//
// 目标：掌握 esp-hal 的随机数接口，并知道它的两个用途：
//         1) 业务需要随机（随机延时、随机闪烁、抽奖都一样）
//         2) 协议栈需要随机种子（Wi-Fi / BLE / 网络栈初始化都要 seed）
//
// 参考：官方 examples/wifi/embassy_dhcp（用 Rng 生成网络栈的 seed）
//       esp-hal/src/rng/mod.rs、esp-hal/src/rng/trng.rs
//       结构照 src/bin/tasks.rs
//
// 说明：ESP32-S3 在 esp-hal 1.2.2 里**没有**触摸（touch）和温度传感器（tsens）
//       驱动模块（它们在别的芯片上才有），所以原计划的 25/26 课换成了
//       「随机数」和「系统信息」，两课都能在 S3 上真跑。
//
// 接线：不需要外部器件
// 运行：cargo run --release --bin 25_rng
// 预期：每次打印一个 32 位随机数，并用它决定下次等待 200~1200 ms
// 坑：  ① Rng 是「伪随机」：种子来自硬件噪声，但序列可复现，够普通业务用
//       ② 要「真随机」用 esp_hal::rng::Trng（熵源 = RNG 外设 + ADC1 的噪声），
//          代价是初始化更慢、更耗电；本课不展开
//       ③ 别用它做密码学（密钥、token）——那要用 Trng
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;                 // main 宏要求的形参类型
use embassy_time::{Duration, Timer};           // 异步定时器（随机延时要靠它）
use esp_hal::clock::CpuClock;                  // CPU 频率
use esp_hal::rng::Rng;                         // 随机数驱动
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

    // 时基：下面用 Timer 做随机延时
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // 建随机数驱动：不需要传外设（它直接用芯片内部的噪声源）
    let rng = Rng::new();

    // 第一件事先演示「网络栈 seed」的用法（esp-radio / embassy-net 都要一个 u64 种子）
    let seed = ((rng.random() as u64) << 32) | rng.random() as u64;
    rprintln!("给网络栈用的 64 位随机种子：{:#018X}", seed);

    let mut n: u32 = 0;
    loop {
        n += 1;

        // random() 返回一个 u32；打印成 16 进制和十进制各一次，方便看
        let value = rng.random();
        rprintln!("第 {} 个随机数：{}（0x{:08X}）", n, value, value);

        // 用随机数决定这次的等待时长：200 ~ 1200 ms
        // % 1000 取余得到 0~999，再加 200 就落在这个区间
        let wait_ms = 200 + (value % 1000);
        rprintln!("   这次等 {} ms", wait_ms);
        Timer::after(Duration::from_millis(wait_ms as u64)).await;
    }
}
