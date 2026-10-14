// ============================================================================
// 23 · 抽象层：用 embedded-hal / embedded-io 的 trait 写「不认芯片」的代码
//
// 目标：理解为什么要有 trait 层。本课写两个通用函数，函数体里**完全不出现**
//       esp32s3、GPIO2、esp-hal 这些字眼：
//         blink()  —— 只要能翻转的引脚 + 能延时，就能闪烁
//         banner() —— 只要能写字节，就能当调试串口用
//       然后把 esp-hal 的 Output / Delay / Uart 递进去，直接就能跑。
//
// 参考：esp-hal/src/gpio/embedded_hal_impls.rs、esp-hal/src/delay.rs、
//       esp-hal/src/uart/compat.rs（这些文件就是「实现 trait」的地方）
//       结构照 src/bin/tasks.rs
//
// 接线：LED→GPIO2；UART1 TX=GPIO17 可选（想用串口工具看 banner 就接 USB-TTL）
// 运行：cargo run --release --bin 23_traits
// 预期：LED 闪 3 次（通用函数干的），RTT 打印完成提示
// 坑：  trait 层只定义能力，不定义「怎么等」——延时也要作为参数传进来，
//       这样同一个函数既能用在阻塞场景，也能换成异步实现
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;                 // main 宏要求的形参类型
use embassy_time::{Duration, Timer};           // 异步定时器
use embedded_hal::delay::DelayNs;              // 通用延时接口（embedded-hal 1.0）
use embedded_hal::digital::StatefulOutputPin;  // 通用「可翻转输出引脚」接口
use embedded_io::Write;                        // 通用「能写字节」接口（embedded-io 0.7）
use esp_hal::clock::CpuClock;                  // CPU 频率
use esp_hal::delay::Delay;                     // esp-hal 的阻塞延时（它实现了 DelayNs）
use esp_hal::gpio::{Level, Output, OutputConfig}; // esp-hal 的 GPIO 输出
use esp_hal::timer::timg::TimerGroup;          // TIMG：给调度器提供时基
use esp_hal::uart::{Config as UartConfig, Uart}; // UART（它实现了 embedded-io 的 Write）
use rtt_target::rprintln;                      // RTT 打印

/// 通用函数 1：闪烁任意「可翻转的输出引脚」。
///
/// 两个泛型参数就是两种能力：P 能设电平，D 能延时。
/// 这个函数放进任何项目的任何芯片上都能用——只要那边也实现了同样的 trait。
fn blink<P, D>(pin: &mut P, delay: &mut D, times: u32)
where
    P: StatefulOutputPin,                      // 能 set_high / set_low / toggle
    D: DelayNs,                                // 能 delay_ms / delay_us ...
{
    for _ in 0..times {
        pin.set_high().ok();                   // Result 的 Err 类型是「不会发生」的，直接忽略
        delay.delay_ms(150);                   // 半周期
        pin.set_low().ok();
        delay.delay_ms(150);
    }
}

/// 通用函数 2：往任意「能写字节的东西」里写一段文本。
///
/// 返回 Result 用关联类型 W::Error，所以不管是 UART、USB 串口还是内存缓冲都能用。
fn banner<W: Write>(w: &mut W) -> Result<(), W::Error> {
    w.write_all(b"[banner] hello from a generic writer\r\n")?; // ? 把错误交给调用方
    w.flush()                                  // 等数据真正送出去
}

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

    // 时基：下面循环里要 await
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // 这里准备好的是「具体实现」：esp-hal 的 GPIO 输出和阻塞延时
    let mut led = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());
    let mut delay = Delay::new();

    rprintln!("用通用函数闪烁 LED（函数内部不知道这是 ESP32-S3）");
    blink(&mut led, &mut delay, 3);            // 具体实现 → 通用函数

    // UART 也是「具体实现」，但换了个 trait：能写字节
    let mut uart = Uart::new(
        peripherals.UART1,
        UartConfig::default().with_baudrate(115_200),
    )
    .unwrap()
    .with_tx(peripherals.GPIO17);              // 只用到发，不需要 RX

    // 同一个 banner() 换成 UART 也照样能用（打印错误就忽略，因为没接 USB-TTL 也没关系）
    let _ = banner(&mut uart);
    rprintln!("banner() 已经用通用 trait 写进 UART1（接 USB-TTL 到 GPIO17 才能看到）");

    rprintln!("23 课演示结束，程序进入空闲循环");
    loop {
        Timer::after(Duration::from_secs(10)).await;
    }
}
