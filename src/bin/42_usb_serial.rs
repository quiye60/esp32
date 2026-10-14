// ============================================================================
// 42 · USB OTG：把板子变成「真 USB 串口」（CDC-ACM），插上电脑就能收数据
//
// 和第 20 课（USB-Serial-JTAG）的区别，先分清楚：
//   第 20 课：用芯片**内置**的 USB-Serial-JTAG 外设，不需要任何额外库，
//             适合当调试串口用（和 RTT 并列）。
//   本课：用 USB OTG 控制器自己实现一个标准 USB 设备（CDC-ACM 虚拟串口），
//         电脑装好驱动后会多出一个 COM 口 / ttyACM，适合做「产品对外接口」。
//
// 目标：完整走一遍 USB 设备栈的分工：
//         esp-hal  → Driver：管物理层（端点、DMA、SOF）
//         embassy-usb → Builder/Device：管描述符与标准控制请求（GET_DESCRIPTOR…）
//         CdcAcmClass → 管「USB 串口」这个类：收发 packet、断线检测
//         你 → 只要 read_packet / write_packet
//       本课把收到的字符转成大写再发回去（经典的 echo + 变换）。
//
// 参考：官方 examples/async/embassy_usb_serial（本项目按它改写，打印换成 RTT）
//       结构照 src/bin/tasks.rs
//
// 接线：S3 的 USB 引脚是固定的：DP = GPIO20、DM = GPIO19
//       （注意：很多 S3 开发板这两个脚同时接到了板载 USB 座和 USB-Serial-JTAG 上，
//         要用 OTG 功能时确认接线是接到 USB 座而不是外部器件）
// 运行：cargo run --release --bin 42_usb_serial
//      然后电脑上出现一个新的串口设备：
//        Linux: /dev/ttyACM0     macOS: /dev/tty.usbmodemXXXX     Windows: 新的 COM 口
//      用串口工具（minicom/screen/PuTTY）打开，波特率随便填（CDC 不看波特率），
//      输入小写字母，会收到大写回显
// 预期：RTT 打印 "USB 已连接"，串口工具里能看到回显
// 坑：  ① 一定要 --release：debug 下 USB 时序容易不满足（官方示例也这么要求）
//       ② 插进电脑后如果只认出一个「USB 串行设备」但没有新串口：多半是描述符里的
//          类信息不对，或系统还需要驱动（Windows 一般自动装 CDC 驱动）
//       ③ ep_out_buffer 要够大（下面给 1 KB）；太小会丢包
//       ④ 同时用板载 USB-Serial-JTAG 和 OTG 会互相干扰，做 USB 功能时，
//          调试打印建议走 RTT（本项目默认就是 RTT）
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use embassy_usb::{Builder, Config as UsbConfig};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::usb::otg::{
    Usb,
    embassy_usb_device::{Config as DriverConfig, Driver},
};
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

    // ---------------- 1. 拿出 USB 外设（全速模式，DP/DM 固定引脚） ----------------
    // new_fs = Full Speed（12 Mbps）；S3 的 USB0 只能全速
    let usb = Usb::new_fs(peripherals.USB_FS, peripherals.GPIO20, peripherals.GPIO19);

    // 端点 OUT 的接收缓冲：主机发来的数据先落在这里
    let mut ep_out_buffer = [0u8; 1024];
    let driver = Driver::new(usb, &mut ep_out_buffer, DriverConfig::default());

    // ---------------- 2. USB 设备描述符 ----------------
    // VID/PID 用 Espressif 的测试 ID（0x303A）；自己做产品要申请或用自己的
    let mut config = UsbConfig::new(0x303A, 0x3001);
    config.manufacturer = Some("esp-s3-rust");
    config.product = Some("ESP32-S3 教学板 USB 串口");
    config.serial_number = Some("s3-0001");

    // 这些缓冲是给「控制传输」用的：标准请求（取描述符、设置配置）都走它
    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buf = [0; 64];
    let mut state = State::new();

    let mut builder = Builder::new(
        driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [],            // 不用 MS OS 描述符（那是给 Windows 特殊驱动用的）
        &mut control_buf,
    );

    // ---------------- 3. 挂一个 CDC-ACM 类（就是「虚拟串口」） ----------------
    // 64 = 端点包大小（全速 USB 的常见值）
    let mut class = CdcAcmClass::new(&mut builder, &mut state, 64);

    // 生成 Device：它的 run() 负责所有 USB 事件（枚举、挂起、复位…）
    let mut usb_device = builder.build();
    let usb_fut = usb_device.run();

    rprintln!("USB 设备已启动，等电脑枚举……");

    // ---------------- 4. 业务逻辑：把收到的小写字母转大写回显 ----------------
    let echo_fut = async {
        loop {
            class.wait_connection().await;      // 用另一个 future 等连接，不阻塞 USB 事件处理
            rprintln!("USB 已连接（电脑认得这个串口了）");

            match echo_uppercase(&mut class).await {
                Ok(()) => rprintln!("USB 断开"),
                Err(_) => rprintln!("USB 断开（端点被禁用）"),
            }
        }
    };

    // 两件事都必须在跑：USB 事件处理 + 收发逻辑
    join(usb_fut, echo_fut).await;

    loop {}
}

/// 读一个 packet → 转成大写 → 发回去，直到断开。
async fn echo_uppercase<'d>(
    class: &mut CdcAcmClass<'d, Driver<'d>>,
) -> Result<(), EndpointError> {
    let mut buf = [0u8; 64];
    loop {
        let n = class.read_packet(&mut buf).await?;
        for byte in buf[..n].iter_mut() {
            // ASCII 小写转大写：清掉 0x20 这一位
            if byte.is_ascii_lowercase() {
                *byte -= 32;
            }
        }
        class.write_packet(&buf[..n]).await?;
    }
}
