// ============================================================================
// 40 · LCD_CAM —— 用 8080 并口驱动 SPI 屏之外的大屏（WT32-SC01 Plus 之类）
//
// 先说明一件容易踩的事：ESP32-S3 **没有 PARL_IO 外设**（那是 C6/P4 才有的），
// 它的并行口就是 LCD_CAM。所以本课（对应课程表里的「40 LCD_CAM + PARL_IO」）
// 在 S3 上讲 LCD_CAM 的 I8080 模式，PARL_IO 留到用 C6/P4 时再学。
//
// 目标：搞清 8080 并口屏的四类线，以及「为什么必须上 DMA」：
//         DC   —— 高=数据、低=命令（这是 8080 与普通并口的最大区别）
//         WR   —— 写时钟（LCD_CAM 负责产生）
//         D0~D7 —— 8 位数据线（也可以 16 位）
//         TE   —— 撕裂同步信号（面板刷新的节拍，用它避免画面撕裂，本课只读不判）
//       一屏 320×480×2 字节 = 300 KB，靠 CPU 一位位发根本刷不动，
//       所以 LCD_CAM 一次 DMA 把整块像素推出去（CPU 只管准备下一块）。
//
// 参考：官方 qa-test/src/bin/lcd_i8080.rs（完整的 ST7796 初始化 + 刷屏）
//       esp-hal/src/lcd_cam/lcd/i8080.rs 的文档
//       结构照 src/bin/tasks.rs
//
// 接线（WT32-SC01 Plus 默认定义，其它屏请按自己模块改）：
//       背光=GPIO45、复位=GPIO4、DC=GPIO0、WR=GPIO47、TE=GPIO48、
//       D0..D7 = GPIO9 / 46 / 3 / 8 / 18 / 17 / 16 / 15
// 运行：cargo run --release --bin 40_lcd_i8080
// 预期：没接屏幕时只打印日志；接上屏并补全面板初始化表后，屏幕会红绿蓝交替闪
// 坑：  ① 不同面板的初始化命令表完全不同（本课只发最基础的 4 条），
//          真要点亮屏幕请参考官方 qa-test 里 ST7796 的完整表，或屏厂给的 datasheet
//       ② 数据线顺序要和硬件一一对应，错一根颜色就乱
//       ③ 20 MHz 已经很快了，飞线必须短，最好用排线并加地线
//       ④ 屏的逻辑供电 3.3V，请确认模块自带 LDO/电平转换
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::dma_tx_buffer;                 // 宏：DMA 发送缓冲 + 描述符
use esp_hal::dma::DmaTxBuf;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::lcd_cam::{LcdCam, lcd::i8080::{Config as I8080Config, I8080}};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use rtt_target::rprintln;

/// 一屏像素数据：320×480 的 16 位色 = 300 KB，这里只演示一次推 4 KB
const CHUNK: usize = 4096;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

/// 把「I8080 总线 + DMA 缓冲」包在一起，避免每次发送都借来借去。
/// 真实项目里会在这里实现 fill_rectangle / draw_bitmap 之类的上层函数。
struct LcdBus<'d> {
    bus: Option<(I8080<'d, esp_hal::Blocking>, DmaTxBuf)>,
}

impl<'d> LcdBus<'d> {
    fn new(bus: I8080<'d, esp_hal::Blocking>, buf: DmaTxBuf) -> Self {
        Self { bus: Some((bus, buf)) }
    }

    /// 发一条命令（DC 拉低）+ 紧跟一段数据（DC 拉高）
    fn send(&mut self, cmd: u8, data: &[u8]) -> Result<(), ()> {
        let (bus, mut buf) = self.bus.take().unwrap();
        buf.fill(data);
        let result = match bus.send(cmd, 0, buf) {
            Ok(transfer) => {
                // wait() 返回 (传输结果, 总线, 缓冲) —— 后两个会把所有权还回来
                let (_res, bus, buf) = transfer.wait();
                self.bus = Some((bus, buf));
                Ok(())
            }
            Err((_err, bus, buf)) => {
                self.bus = Some((bus, buf));
                Err(())
            }
        };
        result
    }
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    rtt_target::rtt_init_print!();

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // 背光与复位：普通 GPIO 就能控制
    let mut backlight = Output::new(peripherals.GPIO45, Level::Low, OutputConfig::default());
    let mut reset = Output::new(peripherals.GPIO4, Level::Low, OutputConfig::default());

    // ---------------- 1. 建 8080 总线 ----------------
    let dma_buf = dma_tx_buffer!(CHUNK).unwrap();
    let lcd_cam = LcdCam::new(peripherals.LCD_CAM);
    let i8080 = I8080::new(
        lcd_cam.lcd,
        peripherals.DMA_CH0,
        I8080Config::default().with_frequency(Rate::from_mhz(20)),  // WR 时钟 20 MHz
    )
    .unwrap()
    .with_dc(peripherals.GPIO0)          // 命令/数据选择（最重要的一根）
    .with_wrx(peripherals.GPIO47)        // 写时钟
    .with_data0(peripherals.GPIO9)
    .with_data1(peripherals.GPIO46)
    .with_data2(peripherals.GPIO3)
    .with_data3(peripherals.GPIO8)
    .with_data4(peripherals.GPIO18)
    .with_data5(peripherals.GPIO17)
    .with_data6(peripherals.GPIO16)
    .with_data7(peripherals.GPIO15);

    let mut bus = LcdBus::new(i8080, dma_buf);

    // ---------------- 2. 硬件复位序列 ----------------
    // 复位脚先拉低再拉高，然后等 120 ms（各面板手册都会有这段）
    reset.set_low();
    Timer::after(Duration::from_millis(10)).await;
    reset.set_high();
    Timer::after(Duration::from_millis(120)).await;

    // ---------------- 3. 最基础的三条 8080 命令 ----------------
    // 0x01 = 软复位，0x11 = 退出睡眠，0x29 = 开显示（ST77xx 系列通用）
    // 真实面板还需要几十条初始化命令（伽马、方向、供电…），见官方 qa-test
    rprintln!("发 0x01 软复位");
    let _ = bus.send(0x01, &[]);
    Timer::after(Duration::from_millis(150)).await;

    rprintln!("发 0x11 退出睡眠");
    let _ = bus.send(0x11, &[]);
    Timer::after(Duration::from_millis(120)).await;

    rprintln!("发 0x29 开显示");
    let _ = bus.send(0x29, &[]);

    // 打开背光
    backlight.set_high();
    rprintln!("背光已打开（没接屏时看不到现象，日志能证明总线配置没报错）");

    // ---------------- 4. 推一整块像素（演示 DMA 刷屏的思路） ----------------
    // 0x2C = 写显存；数据是 RGB565，这里用红(0xF800) / 绿(0x07E0) / 蓝(0x001F) 轮换。
    // 每个像素 2 字节，低字节先发（面板通常按大端写入，具体看屏的手册）
    let colors: [(u8, u8); 3] = [(0x00, 0xF8), (0xE0, 0x07), (0x1F, 0x00)];
    let names = ["红", "绿", "蓝"];
    let mut step = 0;

    loop {
        let (lo, hi) = colors[step % colors.len()];
        // 用同一对字节填满 CHUNK，模拟「一整屏同色」
        let fill = [lo, hi];
        rprintln!("推 {} KB 的 {} 色像素（DMA 直接送，CPU 不参与）", CHUNK / 1024, names[step % names.len()]);
        // 一次 send 只能带一种 cmd + 一段数据，屏幕实际刷满要循环很多次
        if bus.send(0x2C, &fill).is_err() {
            rprintln!("发送失败（检查接线/频率）");
        }

        step += 1;
        Timer::after(Duration::from_secs(1)).await;
    }
}
