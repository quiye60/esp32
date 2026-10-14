// ============================================================================
// 19 · UART：阻塞收发 + 转异步
//
// 目标：掌握 UART 的两种用法，以及「同一套外设、两种执行模式」的切换：
//         Uart<'_, Blocking>  —— uart.write() / uart.read()，简单直接
//         uart.into_async()   —— 变成 Uart<'_, Async>，配 embedded-io-async 用
//
// 参考：官方 examples/async/embassy_serial（异步版）、hil-test/src/bin/uart.rs
//       结构照 src/bin/tasks.rs
//
// 接线（回环自测，不需要外部器件）：GPIO17(UART1 TX) —— 跳线 —— GPIO18(UART1 RX)
//   这样板子发出去的数据会立刻被自己收到，方便验证读写都通。
//   想看电脑上的串口输出，把 USB-TTL 的 RXD 接 GPIO17、GND 接板子 GND 即可（波特率 115200）。
//
// 运行：cargo run --release --bin 19_uart
// 预期：先打印「阻塞」字样并显示回环读回的内容；之后每秒一段「异步」回环内容
// 坑：  UART0 被 ROM/bootloader 用于启动日志，本项目里建议用 UART1 练手
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;                 // main 宏要求的形参类型
use embassy_time::{Duration, Timer};           // 异步定时器
use esp_hal::clock::CpuClock;                  // CPU 频率
use esp_hal::timer::timg::TimerGroup;          // TIMG：给调度器提供时基
use esp_hal::uart::{Config as UartConfig, Uart}; // UART：配置 + 驱动
use rtt_target::rprintln;                      // RTT 打印（调试信息走这里，不走 UART）

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

    // 建 UART1：Config 用默认值（8N1），只改波特率
    // 返回 Result：配置非法（比如波特率算不出来）才会失败，所以 unwrap 没问题
    let mut uart = Uart::new(
        peripherals.UART1,
        UartConfig::default().with_baudrate(115_200),
    )
    .unwrap()
    // S3 的 UART 引脚可以任选（走 GPIO 交换矩阵），这里显式指定 TX / RX
    .with_tx(peripherals.GPIO17)
    .with_rx(peripherals.GPIO18);

    // ---------------- 1. 阻塞收发 ----------------
    rprintln!("1) 阻塞模式：发 8 字节，再把回环读回来");
    // write 返回实际写出的字节数（一般等于数组长度）
    let sent = uart.write(b"BLOCKING").unwrap();
    rprintln!("   已发送 {} 字节", sent);

    // 从 RX 读：因为 TX/RX 用跳线短接，这里能读回刚才发的内容
    let mut buf = [0u8; 16];
    let got = uart.read(&mut buf).unwrap();
    rprintln!("   收到 {} 字节：{}", got, core::str::from_utf8(&buf[..got]).unwrap_or("(非 UTF-8)"));

    // ---------------- 2. 转异步 ----------------
    // 时基：异步模式要用到中断和 embassy 的定时器
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // into_async() 把驱动换成「中断驱动」的实现，类型变成 Uart<'_, Async>
    let mut uart = uart.into_async();
    rprintln!("2) 异步模式：每隔 1 秒回环一次");

    loop {
        // 异步写：等待期间让出 CPU（写 FIFO 满了才会真的等）
        // 这里写全路径 embedded_io_async::Write::...，避免和同名的阻塞方法混淆
        embedded_io_async::Write::write_all(&mut uart, b"ASYNC\r\n").await.unwrap();
        embedded_io_async::Write::flush(&mut uart).await.unwrap(); // 等数据真的发完

        // 异步读：没数据时挂起，不占 CPU（这里因为回环，马上就有数据）
        let mut buf = [0u8; 16];
        let got = embedded_io_async::Read::read(&mut uart, &mut buf).await.unwrap();
        rprintln!("   异步收到 {} 字节：{:?}", got, &buf[..got]);

        Timer::after(Duration::from_secs(1)).await;
    }
}
