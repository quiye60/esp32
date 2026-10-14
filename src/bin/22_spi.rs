// ============================================================================
// 22 · SPI 主机：用一根跳线做回环自测
//
// 目标：掌握 SPI 主机的最简用法：
//         Spi::new(...).with_sck(...).with_mosi(...).with_miso(...)
//         transfer(&mut buf) —— 全双工：发出去的同时把收到的写回同一块缓冲
//         write() / read()   —— 只发 / 只收
//
// 参考：官方 examples/peripheral/spi/loopback、hil-test/src/bin/spi.rs
//       结构照 src/bin/tasks.rs
//
// 接线（回环）：GPIO11(MOSI) —— 跳线 —— GPIO13(MISO)；SCK=GPIO12（不接也可）
//   回环的原理：MOSI 发出去的电平被 MISO 直接读回来，于是「发出的内容 == 收到的内容」
// 运行：cargo run --release --bin 22_spi
// 预期：打印「发出 / 收到」，两者应完全相同；拔掉跳线后收到的会变成 0x00 或 0xFF
// 坑：  ① SPI 是「主从 + 片选」总线，这里不接从机，只验证主机侧时序和收发链路
//       ② 电平是 3.3V，别把 5V 器件直接接上
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;                 // main 宏要求的形参类型
use embassy_time::{Duration, Timer};           // 异步定时器（循环节奏）
use esp_hal::clock::CpuClock;                  // CPU 频率
use esp_hal::spi::{Mode, master::{Config as SpiConfig, Spi}}; // SPI 主机 + 模式
use esp_hal::time::Rate;                       // 频率类型
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

    // 建 SPI2 主机：1 MHz、模式 0（CPOL=0/CPHA=0，绝大多数器件用它）
    let mut spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(1))
            .with_mode(Mode::_0),
    )
    .unwrap()
    .with_sck(peripherals.GPIO12)              // 时钟
    .with_mosi(peripherals.GPIO11)             // 主机输出
    .with_miso(peripherals.GPIO13);            // 主机输入

    // 时基：循环节奏用 Timer
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    let mut n: u8 = 0;
    loop {
        n = n.wrapping_add(1);                 // 每轮换一组数据，方便看变化

        // transfer 是全双工：一边把 buf 发出去，一边把 MISO 上的电平写进 buf
        let mut buf = [n, n.wrapping_add(1), n.wrapping_add(2), n.wrapping_add(3)];
        let sent = buf;                        // 留一份「发出前的样子」用来对比

        spi.transfer(&mut buf).unwrap();       // 一次收发 4 个字节

        rprintln!("发出：{:02X?}", sent);
        rprintln!("收到：{:02X?}  ← 接了 MOSI-MISO 跳线时应与上面完全相同", buf);

        // 只发不收（例如给屏幕发命令）和只收（例如读传感器）也各有一行：
        spi.write(&[0xA5]).unwrap();           // write：发出去的是常量，忽略收到的数据
        rprintln!("write(0xA5) 完成，下面这条是只收模式（没从机时读到 0x00/0xFF）");
        let mut dummy = [0u8; 2];
        spi.read(&mut dummy).unwrap();         // read：只收
        rprintln!("read 到：{:02X?}", dummy);

        Timer::after(Duration::from_secs(2)).await;
    }
}
