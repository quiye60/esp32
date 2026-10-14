// ============================================================================
// 28 · MCPWM：互补 PWM + 死区时间（电机驱动的核心）
//
// 目标：LEDC 只会输出「一路方波」，而驱动 H 桥要「两路严格互补的信号」，
//       这就是 MCPWM 的用武之地。本课搞清三个概念：
//         定时器（timer0）—— 决定 PWM 周期
//         操作器（operator）—— 绑定定时器和一个引脚对（A/B）
//         死区时间（dead time）—— A 关闭到 B 打开之间的强制空档，
//           防止上下管同时导通（直通 = 炸管）
//       最后用 set_deadtime_cfg 在「桥臂关断（都低）」和「桥臂工作（互补）」之间切换。
//
// 参考：官方 qa-test/src/bin/mcpwm.rs、esp-hal/src/mcpwm/operator.rs 的文档示例
//       结构照 src/bin/tasks.rs
//
// 接线：GPIO5 = PWMA，GPIO6 = PWMB（最好接逻辑分析仪看两路互补关系）
//       只接 LED 也能看出亮度变化（A 路在动）
// 运行：cargo run --release --bin 28_mcpwm
// 预期：先 50% 固定占空比 2 秒（看两路互补 + 死区），然后占空比 0→90% 循环扫
// 坑：  ① MCPWM 时钟频率不能超过外设上限，with_frequency 会帮你校验
//       ② 定时器时间戳范围是 0..=周期值（这里 0..=99），不是微秒
//       ③ 要把 B 路配成 PwmPinConfig::EMPTY，再由死区发生器「补」出互补波形
//       ④ 死区时间单位是「MCPWM 时钟周期」，10 在 32 MHz 下约 0.31 µs
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::mcpwm::operator::{DeadTimeCfg, PwmPinConfig};
use esp_hal::mcpwm::timer::PwmWorkingMode;
use esp_hal::mcpwm::{McPwm, PeripheralClockConfig};
use esp_hal::time::Rate;
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

    // ---------------- 1. 给 MCPWM 选时钟 ----------------
    let clock_cfg = PeripheralClockConfig::with_frequency(Rate::from_mhz(32)).unwrap();
    let mut mcpwm = McPwm::new(peripherals.MCPWM0, clock_cfg);

    // ---------------- 2. 操作器绑定定时器 ----------------
    // operator0 用 timer0 作为时间基准（相当于 LEDC 里「通道用哪个定时器」）
    mcpwm.operator0.set_timer(&mcpwm.timer0);

    // ---------------- 3. 一次配置两路引脚 + 死区发生器 ----------------
    // DeadTimeCfg::new_ahc() = AHC 模式（Active High Complementary）：
    //   B 自动成为 A 的互补信号，翻转瞬间留出死区，两路不会同时为高
    let bridge_active = DeadTimeCfg::new_ahc();
    let mut pins = mcpwm.operator0.with_linked_pins(
        peripherals.GPIO5,                  // A 路（主信号）
        PwmPinConfig::UP_ACTIVE_HIGH,       // A：计数到比较值后拉高
        peripherals.GPIO6,                  // B 路
        PwmPinConfig::EMPTY,                // B：不用自己设动作，交给死区发生器
        bridge_active,
    );

    // 上升沿/下降沿死区长度（单位：MCPWM 时钟周期；32 MHz 下 10 拍 ≈ 0.31 µs）
    pins.set_rising_edge_deadtime(10);
    pins.set_falling_edge_deadtime(10);

    // ---------------- 4. 启动定时器：周期 99，频率 20 kHz ----------------
    // 时间戳范围 0..=99，所以写 50 就是 50% 占空比
    let timer_clock_cfg = clock_cfg
        .timer_clock_with_frequency(99, PwmWorkingMode::Increase, Rate::from_khz(20))
        .unwrap();
    mcpwm.timer0.start(timer_clock_cfg);

    // ---------------- 5. 先关断桥臂再上电：电机驱动的安全顺序 ----------------
    rprintln!("桥臂关断（A/B 都低），此时电机不会动");
    pins.set_deadtime_cfg(DeadTimeCfg::new_bypass());   // bypass = 不输出互补波形
    pins.set_timestamp_a(0);
    Timer::after(Duration::from_secs(1)).await;

    // 切到互补输出：A 有波形，B 是它的互补（带死区）
    rprintln!("桥臂工作：A 输出 50%，B 为其互补（带 0.3 µs 死区）");
    pins.set_deadtime_cfg(bridge_active);
    pins.set_timestamp_a(50);
    Timer::after(Duration::from_secs(2)).await;

    // ---------------- 6. 占空比扫描：模拟「慢慢加速」 ----------------
    rprintln!("开始占空比扫描 0% → 90% → 0%");
    loop {
        for duty in (0..=90).step_by(10) {
            pins.set_timestamp_a(duty);
            rprintln!("A 路占空比 = {}%", duty);
            Timer::after(Duration::from_millis(300)).await;
        }
        for duty in (0..=90).rev().step_by(10) {
            pins.set_timestamp_a(duty);
            Timer::after(Duration::from_millis(300)).await;
        }
    }
}
