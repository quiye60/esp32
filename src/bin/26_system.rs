// ============================================================================
// 26 · 系统信息：复位原因 / CPU / 芯片版本 / eFuse MAC 地址
//
// 目标：认识「读芯片自身信息」这一类 API，它们在真实项目里非常常用：
//         reset_reason()      为什么重启的（上电？看门狗？软件复位？）
//         Cpu::current()      当前跑在哪个核上（S3 是双核）
//         chip_revision()     芯片版本（排查硬件差异）
//         interface_mac_address(Station)  Wi-Fi STA 的 MAC 地址（出厂烧在 eFuse 里）
//         software_reset()    主动重启（本课不用，注释里说明）
//
// 参考：esp-hal/src/system.rs、esp-hal/src/efuse/mod.rs 的文档示例
//       官方 qa-test/src/bin/software_reset.rs
//       结构照 src/bin/tasks.rs
//
// 说明：ESP32-S3 在 esp-hal 1.2.2 里没有 touch / tsens 驱动模块，所以原计划的
//       25/26 课换成了「随机数」和这节「系统信息」。
//
// 接线：不需要外部器件
// 运行：cargo run --release --bin 26_system
// 预期：打印复位原因、当前 CPU、芯片版本、MAC 地址；第 5 秒主动重启一次
// 坑：  ① MAC 地址是唯一的，可以拿它当设备 ID（比随机数靠谱）
//       ② 重启后 RTT 会重连，probe-rs 一般能自动接上；看到新的一轮打印就说明成功了
//       ③ reset_reason() 返回 Option：有些芯片/情况读不到就返回 None
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;                 // main 宏要求的形参类型
use embassy_time::{Duration, Timer};           // 异步定时器
use esp_hal::clock::CpuClock;                  // CPU 频率
use esp_hal::efuse::{self, InterfaceMacAddress}; // eFuse：出厂信息（含 MAC）
use esp_hal::system::{Cpu, reset_reason, software_reset}; // 系统信息 + 主动复位
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

    // 初始化芯片，拿到外设句柄（本课不用具体外设，但这行必须调）
    let _peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // 时基：下面要 await Timer
    let timg0 = TimerGroup::new(_peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, _peripherals.FROM_CPU_INTR0);

    // 1) 这次为什么启动：上电 / 软件复位 / 看门狗 / 深睡眠唤醒……
    rprintln!("复位原因：{:?}", reset_reason());

    // 2) 当前代码跑在哪个核上（0 = PRO_CPU，1 = APP_CPU）
    rprintln!("当前 CPU 编号：{}", Cpu::current() as usize);

    // 3) 芯片版本（同一型号不同批次可能不同，排查硬件问题时会问到这个）
    rprintln!("芯片版本：{:?}", efuse::chip_revision());

    // 4) MAC 地址：出厂烧录、全球唯一，可以直接当设备 ID
    let mac = efuse::interface_mac_address(InterfaceMacAddress::Station);
    rprintln!("Wi-Fi STA MAC：{}", mac);        // 带冒号的人类可读格式
    rprintln!("MAC 原始字节：{:02X?}", mac.as_bytes());

    rprintln!("5 秒后主动软件复位（用来验证复位原因会变成 SoftwareReset）");
    Timer::after(Duration::from_secs(5)).await;

    // software_reset() 永不返回（返回类型是 !），所以这里之后不需要写代码
    software_reset();
}
