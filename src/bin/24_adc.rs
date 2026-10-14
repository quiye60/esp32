// ============================================================================
// 24 · ADC：把模拟电压读成数字
//
// 目标：掌握 ADC 的三步：
//         AdcConfig::new()                    建配置
//         config.enable_pin(pin, 衰减)         把某个 GPIO 交给 ADC（返回 AdcPin）
//         Adc::new(ADC1, config)              建 ADC，然后 read_oneshot(&mut pin)
//       并理解「衰减」为什么决定了量程。
//
// 参考：官方 qa-test/src/bin/embassy_adc.rs、esp-hal/src/analog/adc/xtensa.rs
//       结构照 src/bin/tasks.rs
//
// 接线：电位器两端接 3V3 和 GND，中间抽头接 GPIO1（= ADC1 通道 0）
//       没有电位器时：把 GPIO1 用杜邦线分别碰 3V3 / GND，看读数是不是接近最大 / 接近 0
// 运行：cargo run --release --bin 24_adc
// 预期：每 500 ms 打印一次原始值（0~4095）和按量程换算的估算电压
// 坑：  ① ADC2 与 Wi-Fi 冲突，能不用就不用；本课用 ADC1
//       ② 悬空引脚读数会乱跳，这是正常的（高阻态）
//       ③ 11 dB 衰减约对应 0~3.1 V 量程，超量程会一直读到最大值
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;                 // main 宏要求的形参类型
use embassy_time::{Duration, Timer};           // 异步定时器（采样节奏）
use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation}; // ADC 三件套
use esp_hal::clock::CpuClock;                  // CPU 频率
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

    // 时基：下面用 Timer 控制采样间隔
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // 第一步：建 ADC 配置
    let mut adc1_config = AdcConfig::new();

    // 第二步：把 GPIO1 交给 ADC1，并选定 11 dB 衰减（≈0~3.1 V 量程）
    // enable_pin 返回 AdcPin：它就是「ADC 眼里的那个引脚」，读的时候要把它传进去
    let mut pot_pin = adc1_config.enable_pin(peripherals.GPIO1, Attenuation::_11dB);

    // 第三步：建 ADC1 驱动（这里用阻塞/单次模式；想用异步可接 .into_async()）
    let mut adc1 = Adc::new(peripherals.ADC1, adc1_config);

    // 提示：想要「直接读毫伏」而不是原始值，可以把上面 enable_pin 换成
    //   adc1_config.enable_pin_with_cal::<_, esp_hal::analog::adc::AdcCalLine<_>>(pin, Attenuation::_11dB)
    // 这样 adc1.read_oneshot(&mut pot_pin) 的返回值会经过出厂校准。
    rprintln!("24 课：每 500 ms 采一次 ADC1_CH0（GPIO1）");

    loop {
        // read_blocking：采一次就返回，S3 上是 12 位 → 0~4095
        // （另有非阻塞的 read_oneshot，返回 nb::Result，配合 nb 库做轮询用）
        let raw: u16 = adc1.read_blocking(&mut pot_pin);

        // 粗略换算：raw / 4095 * 量程(约 3.1 V) —— 只是为了让数字有物理意义
        let mv = (raw as u32 * 3100) / 4095;
        rprintln!("原始值 {} / 4095  ≈ {} mV", raw, mv);

        Timer::after(Duration::from_millis(500)).await;
    }
}
