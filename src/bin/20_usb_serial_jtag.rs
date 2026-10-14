// ============================================================================
// 20 · USB-Serial-JTAG：ESP32-S3 内置 USB 当串口用
//
// 目标：不接任何外设，用板子自己的 USB 口收发数据。
//       S3 的那个 USB 口同时提供两条通道：JTAG（probe-rs 烧录/调试用）
//       和 USB-Serial-JTAG（就是本课的串口），互不干扰。
//
// 参考：官方 examples/async/embassy_usb_serial_jtag、esp-hal/src/usb/usb_serial_jtag.rs
//       结构照 src/bin/tasks.rs
//
// 接线：不用接线，把板子的 USB 口插到电脑上即可（RTT 也走这个口）
// 运行：cargo run --release --bin 20_usb_serial_jtag
// 预期：RTT 里看到提示后，用串口工具（如 `screen /dev/ttyACM0 115200`、
//       Windows 上 COM 口）连上去，就能看到板子发来的问候语；
//       你在串口里敲字符，板子会把它回显（echo）出来。
// 坑：  ① 串口工具要独占这个口；如果打不开，先确认没有别的程序占着它
//       ② read 是阻塞的：没人发数据时它会一直等，这是预期行为
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;                 // main 宏要求的形参类型
use embassy_time::{Duration, Timer};           // 异步定时器（心跳用）
use embedded_io::{Read as _, Write as _};      // 阻塞版读写 trait（esp-hal 已实现 0.7 版）
use esp_hal::clock::CpuClock;                  // CPU 频率
use esp_hal::timer::timg::TimerGroup;          // TIMG：给调度器提供时基
use esp_hal::usb::usb_serial_jtag::UsbSerialJtag; // 内置 USB 串口驱动
use rtt_target::rprintln;                      // RTT 打印（调试信息走 JTAG，不走 USB 串口）

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

    // USB-Serial-JTAG 不用指定引脚：S3 上它固定接在 GPIO19/20（就是那个 USB 口）
    let mut usb = UsbSerialJtag::new(peripherals.USB_DEVICE);

    // 时基：下面心跳要用 Timer
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // 往 USB 串口写：字符串通过 embedded-io 的 Write trait 发出去
    usb.write_all(b"hello from USB-Serial-JTAG!\r\n").unwrap();
    usb.write_all(b"type something, I will echo it back\r\n").unwrap();
    usb.flush().unwrap();                      // 等数据真的发完
    rprintln!("已通过 USB 串口发出问候语（用串口工具连上就能看到）");

    // 把 USB 串口拆成「只发」和「只收」两半，两边可以放进不同任务
    let (mut rx, mut tx) = usb.split();

    let mut buf = [0u8; 32];                   // 接收缓冲：一次最多 32 字节
    let mut rounds = 0u32;                   // 心跳计数

    loop {
        // 阻塞读：有数据就读到 buf 里（等待期间 CPU 空转，这课只演示用法）
        let got = rx.read(&mut buf).unwrap();
        if got > 0 {
            rounds += 1;
            rprintln!("收到 {} 字节：{:?}", got, &buf[..got]);

            // 原样回显：把收到的字节写回去
            tx.write_all(&buf[..got]).unwrap();
            tx.flush().unwrap();
        }

        // 顺手报个数：证明循环还在跑
        if rounds.is_multiple_of(10) {          // clippy 推荐的写法：每 10 条数据报一次
            tx.write_all(b"[still here]\r\n").ok(); // 失败也无所谓，忽略结果
        }

        Timer::after(Duration::from_millis(10)).await;
    }
}
