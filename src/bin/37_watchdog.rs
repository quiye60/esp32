// ============================================================================
// 37 · 系统（续）：看门狗 + Systimer + 临界区
//
// 目标：补齐「系统级」的三件套，让程序在真实环境里不死不僵：
//         1) RWDT 看门狗：程序卡死时自动复位。喂狗（feed）是唯一合法操作，
//            超过超时没喂 → 芯片重启，复位原因会显示 RWDT
//         2) Systimer：esp-hal 里最精确的单调计数器（RTC 域，睡眠中也在走）
//         3) 临界区：critical_section::with 关中断保护共享变量；
//            并说清「为什么不要在临界区里干重活」（会拉高中断延迟）
//       第 26 课讲过复位原因/CPU/MAC，本课的重点是「怎么用它们做工程」
//
// 参考：esp-hal/src/rtc_cntl/mod.rs（Rwdt）、esp-hal/src/timer/systimer.rs
//       本项目 teach/16-gpio_interrupt.md（临界区）
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件
// 运行：cargo run --release --bin 37_watchdog
// 预期：每秒打印心跳 / Systimer 读数 / 临界区计数器；
//       第 11 次心跳后故意停止喂狗，3 秒内被复位，重新启动时复位原因会变
// 坑：  ① 看门狗一开就没法用调试器单步（一停就超时），调试时先别开
//       ② 喂狗必须来自「真正判断程序健康」的地方，无脑喂等于没开
//       ③ Systimer 的绝对值不要跨芯片比较，它的位宽/频率随芯片不同
//       ④ RTC 域时间能跨深睡眠：Rtc 的 current_time_us / set_current_time_us 可校准
// ============================================================================

#![no_std]
#![no_main]

use core::cell::RefCell;

use critical_section::Mutex;                 // 关中断级别的互斥
use embassy_executor::Spawner;
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::rtc_cntl::{Rtc, RwdtStage, SocResetReason, reset_reason};
use esp_hal::system::Cpu;
use esp_hal::time::{Duration, Instant};
use esp_hal::timer::systimer::{SystemTimer, Unit};
use esp_hal::timer::timg::TimerGroup;
use rtt_target::rprintln;

/// 临界区保护的共享状态：多个任务/中断都可能碰它
static SHARED: Mutex<RefCell<u32>> = Mutex::new(RefCell::new(0));

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

    // ---------------- 0. 上次是不是被看门狗打了？ ----------------
    let reason = reset_reason(Cpu::ProCpu).unwrap_or(SocResetReason::ChipPowerOn);
    rprintln!("复位原因：{:?}（上一轮演示成功的话，这里会看到 RWDT/RTC WDT 相关）", reason);

    // ---------------- 1. 打开 RWDT 看门狗 ----------------
    // Rtc 里持有 rwdt 句柄；LPWR 是「低功耗/时钟控制」外设
    // LPWR 给 LowPower 用，RTC 的计时器外设叫 RTC_TIMER
    let mut rtc = Rtc::new(peripherals.RTC_TIMER);

    // 两个阶段可以设不同超时（先报警再复位）；这里只用 stage0 = 硬复位
    rtc.rwdt.set_timeout(RwdtStage::Stage0, Duration::from_secs(3));
    rtc.rwdt.enable();
    rprintln!("RWDT 已开启：3 秒不喂狗就复位");

    // ---------------- 2. Systimer：精确的单调计时 ----------------
    // now() 返回自初始化以来的时间；unit_value(Unit::Unit0) 是原始计数值
    let start = Instant::now();
    rprintln!(
        "Systimer 起点：{} µs（Unit0 原始值 = {}）",
        // Instant 之间相减得到 Duration（esp-hal 认为绝对时间点没有「微秒数」这种含义）
        (Instant::now() - start).as_micros(),
        SystemTimer::unit_value(Unit::Unit0)
    );

    let mut heartbeats = 0u32;
    loop {
        heartbeats += 1;

        // ---------------- 3. 临界区：安全地改共享变量 ----------------
        // with 期间中断被关掉，所以这里绝对不能做耗时的事
        let value = critical_section::with(|cs| {
            let mut v = SHARED.borrow_ref_mut(cs);
            *v += 1;                     // 读-改-写整体是原子的（因为关了中断）
            *v
        });

        rprintln!(
            "心跳 {}：Systimer 走了 {} µs，共享计数器 = {}",
            heartbeats,
            (Instant::now() - start).as_micros(),
            value
        );

        // ---------------- 4. 喂狗 ----------------
        if heartbeats <= 10 {
            rtc.rwdt.feed();
        } else if heartbeats == 11 {
            rprintln!("从现在起不再喂狗，3 秒后看门狗会复位芯片（注意复位原因的变化）");
        }

        Timer::after(EmbassyDuration::from_secs(1)).await;
    }
}
