// ============================================================================
// 21 · I2C 主机：总线扫描 + 寄存器读写
//
// 目标：掌握 I2C 的三件套：
//         I2c::new(...).with_sda(...).with_scl(...)  建总线
//         write(addr, &[..])                         写
//         write_read(addr, &[reg], &mut buf)         按「寄存器地址 + 读」的套路读
//       并学会用「空写」去探测总线上有哪些器件（扫描）。
//
// 参考：官方 hil-test/src/bin/i2c.rs、esp-hal/src/i2c/master/mod.rs 的文档示例
//       结构照 src/bin/tasks.rs
//
// 接线：SDA=GPIO4、SCL=GPIO5（都要接 4.7kΩ 上拉到 3V3；很多模块板载已有上拉）
//       不接任何器件也能跑：扫描结果会是「没发现器件」，这是正常的
// 运行：cargo run --release --bin 21_i2c
// 预期：打印扫描到的地址（有器件时），然后对第一个器件尝试读寄存器并打印结果
// 坑：  ① 地址是 7 位的（0x08~0x77）
//       ② 没有上拉电阻时总线读全是 0xFF/超时 —— 先检查上拉
//       ③ 同一条总线上多个器件地址不能冲突
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;                 // main 宏要求的形参类型
use embassy_time::{Duration, Timer};           // 异步定时器（扫描间隔）
use esp_hal::clock::CpuClock;                  // CPU 频率
use esp_hal::i2c::master::{Config as I2cConfig, I2c, I2cAddress}; // I2C 主机
use esp_hal::time::Rate;                       // 频率类型（I2C 速率用它表示）
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

    // 建 I2C0：标准速率 100 kHz（长线/弱上拉时先降速更稳）
    let mut i2c = I2c::new(
        peripherals.I2C0,
        I2cConfig::default().with_frequency(Rate::from_khz(100)),
    )
    .unwrap()
    .with_sda(peripherals.GPIO4)               // 数据线
    .with_scl(peripherals.GPIO5);              // 时钟线

    // 时基：下面用 Timer 控制扫描节奏
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    loop {
        rprintln!("开始扫描 I2C 总线…");
        let mut found: Option<u8> = None;      // 记住第一个发现的地址（后面拿它演示读寄存器）

        // 7 位地址范围是 0x08~0x77（0x00~0x07 和 0x78~0x7F 保留）
        for addr in 0x08u8..=0x77 {
            // 空写 = 只发地址、不发数据：器件应答（ACK）就说明它在线
            let address = I2cAddress::SevenBit(addr); // 7 位地址（右对齐，0x08~0x77）
            if i2c.write(address, &[]).is_ok() {
                rprintln!("  发现器件：0x{:02X}", addr);
                if found.is_none() {
                    found = Some(addr);        // 记下第一个
                }
            }
        }

        match found {
            // 有器件：演示「写寄存器地址 + 读数据」这个最常见的读法
            Some(addr) => {
                let address = I2cAddress::SevenBit(addr);
                let mut reg = [0u8; 4];        // 读回 4 个字节
                // write_read：先写 &[0x00]（寄存器地址），紧接着读 reg.len() 个字节
                match i2c.write_read(address, &[0x00], &mut reg) {
                    Ok(()) => rprintln!("  读 0x{:02X} 寄存器 0x00 → {:02X?}", addr, reg),
                    // 有些器件不支持从寄存器 0 读，报错是正常的，别慌
                    Err(e) => rprintln!("  读寄存器失败（该器件可能不支持）：{:?}", e),
                }
            }
            // 没器件：不是错误，只是线上没东西（或不带上拉）
            None => rprintln!("  没有发现任何器件（检查 SDA/SCL 接线和上拉电阻）"),
        }

        Timer::after(Duration::from_secs(3)).await; // 每 3 秒扫一次
    }
}
