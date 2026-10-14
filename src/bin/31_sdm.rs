// ============================================================================
// 31 · SDM（Sigma-Delta 调制）：用 1 位输出「做」出模拟电压
//
// 目标：理解一种有趣的「软件 DAC」：
//         SDM 只输出 0/1 方波，但通过改变 1 的比例（脉冲密度），
//         经过外部 RC 低通滤波后，平均电压就是你要的模拟值。
//       两个入口要分清：
//         set_duty(0..=255)              —— 直接给占空比（0 = 全低，255 = 全高）
//         set_pulse_density(-128..=127)  —— 相对中间电平的偏移，0 表示 50%
//
// 参考：esp-hal/src/sdm.rs 的文档示例、hil-test/src/bin/sdm.rs
//       结构照 src/bin/tasks.rs
//
// 接线：GPIO7 → RC 低通（例如 1 kΩ + 1 µF）→ 万用表/示波器看平均电压
//       不接滤波也行：用逻辑分析仪看「1 的密度」变化
// 运行：cargo run --release --bin 31_sdm
// 预期：打印目标占空比；接 RC 后用万用表能看到电压平滑地来回爬
// 坑：  ① 频率不能乱选：太低滤波跟不上（纹波大），太高对 RC 要求更苛刻
//       ② 输出幅度是 3.3 V，但驱动能力很弱，接负载前先确认
//       ③ duty 与 pulse_density 是同一目标的两个视角，先后写会互相覆盖
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::sdm::{Channel, ChannelConfig};
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

    // ---------------- 1. 配置并建通道 ----------------
    // 载波 500 kHz：既够高（滤波后纹波小），又不至于太快
    // with_frequency 会挑一个最接近的硬件分频值，选不出来会返回 Err
    let config = ChannelConfig::new()
        .with_frequency(Rate::from_khz(500))
        .unwrap()
        .with_duty(128);                    // 初值：50% 占空比

    // SDM_CH0 是外设句柄，第二个参数是输出引脚
    let mut channel = Channel::new(peripherals.SDM_CH0, peripherals.GPIO7, config);

    rprintln!("SDM 已启动（GPIO7 输出 500 kHz 方波，占空比可编程）");

    loop {
        // ---------------- 2. 用 duty（0..=255）扫描 ----------------
        // duty/255 就是「高电平比例」，滤波后平均电压 = 3.3 V × duty/255
        for duty in (0..=255).step_by(51) {
            channel.set_duty(duty);
            let millivolts = 3300u32 * duty as u32 / 255;
            rprintln!(
                "duty = {:>3} → 高电平占 {:.0}%，滤波后约 {} mV",
                duty,
                duty as f32 / 255.0 * 100.0,
                millivolts
            );
            Timer::after(Duration::from_millis(400)).await;
        }

        // ---------------- 3. 用 pulse_density（-128..=127）扫描 ----------------
        rprintln!("改用 set_pulse_density（-128..127，0 = 50% 占空比）");
        for density in [-128i8, -64, 0, 64, 127] {
            channel.set_pulse_density(density);
            rprintln!(
                "pulse_density = {:>4} → 约占空比 {:.1}%",
                density,
                50.0 + density as f32 / 256.0 * 100.0
            );
            Timer::after(Duration::from_millis(400)).await;
        }
    }
}
