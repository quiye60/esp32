// ============================================================================
// 32 · TWAI（CAN）：自检模式下发一帧再收回自己
//
// 目标：CAN 是车上/工业里最常见的总线，本课把最关键的几步走一遍：
//         1) TwaiConfiguration::new_no_transceiver(...) —— 不用收发器芯片，
//            TX 直接用开漏模式，两块板可以一线对接（或单板自检）
//         2) set_filter(SingleStandardFilter::new(...)) —— 硬件验收滤波器，
//            用 "x" 当通配符只关心特定 ID，减少 CPU 负担
//         3) 标准帧 StandardId（11 位）vs 扩展帧 ExtendedId（29 位）
//         4) 自检模式（TwaiMode::SelfTest）+ new_self_reception：
//            单板就能把自己发的帧收回来，不需要第二块板
//
// 参考：官方 examples/async/embassy_twai、examples/peripheral/twai
//       结构照 src/bin/tasks.rs
//
// 接线：只用一块板 → 用默认的自检模式即可
//       两块板对通：GND 对接、GPIO4 互接，两块都烧这份代码并把 SELF_TEST 改成 false，
//       其中一块把 IS_FIRST_SENDER 改成 false
//       接真实 CAN 总线：用 new() + CAN 收发器（TJA1050 / SN65HVD230），
//       总线两端各接 120 Ω 终端电阻
// 运行：cargo run --release --bin 32_twai
// 预期：每 500 ms 发一帧 [1,2,3]，紧接着收到同一帧（自检回环）
// 坑：  ① 波特率两端必须一致（这里 125 kbps），否则报总线错误
//       ② 没接第二块板又没开自检 → 发送会一直等 ACK，最终进入错误被动状态
//       ③ 要发扩展 ID 帧用 ExtendedId，接口在 EspTwaiFrame 的构造函数里
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::twai::{self, EspTwaiFrame, StandardId, TwaiMode, filter::SingleStandardFilter};
use rtt_target::rprintln;

/// 自检模式：单块板就能收发自己的帧（不用第二块板、不用收发器）
const SELF_TEST: bool = true;
/// 两块板对通时，把其中一块改成 false，就能看出「谁先发」的差别
const IS_FIRST_SENDER: bool = true;

/// 总线速率：两端必须一致
const TWAI_BAUDRATE: twai::BaudRate = twai::BaudRate::B125K;

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

    // 不用收发器时，TX/RX 可以共用一根线：split() 把 GPIO4 拆成输入/输出两个信号
    // （SAFETY：只在确定这根线没有被别的外设驱动时才能这么做）
    let (rx_pin, tx_pin) = unsafe { peripherals.GPIO4.split() };
    // 用外部收发器时改成这样（并换成 TwaiConfiguration::new）：
    //   let tx_pin = peripherals.GPIO4;
    //   let rx_pin = peripherals.GPIO5;

    // ---------------- 1. 建配置（此时外设处于复位态：能配置、不能收发） ----------------
    let mode = if SELF_TEST { TwaiMode::SelfTest } else { TwaiMode::Normal };
    let mut twai_config = twai::TwaiConfiguration::new_no_transceiver(
        peripherals.TWAI0,
        rx_pin,
        tx_pin,
        TWAI_BAUDRATE,
        mode,
    );

    // ---------------- 2. 硬件验收滤波器 ----------------
    // 10 个 'x' 表示这 10 位不关心，最后一位 '1' → 只接收 ID 最低位为 1 的帧。
    // 'x' 是通配符，所以一条规则能覆盖一批 ID，比软件过滤省事。
    twai_config.set_filter(
        const { SingleStandardFilter::new(b"xxxxxxxxxx1", b"x", [b"xxxxxxxx", b"xxxxxxxx"]) },
    );

    // ---------------- 3. 转异步并启动（启动后配置就锁定了） ----------------
    let mut twai = twai_config.into_async().start();
    rprintln!("TWAI 已启动：125 kbps，滤波器只放行 ID 最低位为 1 的帧");
    if SELF_TEST {
        rprintln!("（自检模式：发出去的帧会立刻被自己收到）");
    }

    if IS_FIRST_SENDER {
        send_frame(&mut twai, &[1, 2, 3]).await;
    }

    loop {
        // 等一帧（异步等待期间 CPU 可以去干别的）
        match twai.receive_async().await {
            Ok(frame) => {
                rprintln!("收到一帧：{:?}", frame);
                rprintln!("  数据 = {:?}", frame.data());   // 和发送内容对照
            }
            Err(e) => {
                rprintln!("接收出错：{:?}", e);
                Timer::after(Duration::from_millis(100)).await;
                continue;
            }
        }

        Timer::after(Duration::from_millis(500)).await;
        send_frame(&mut twai, &[1, 2, 3]).await;
    }
}

/// 发一帧标准帧（ID = 0，数据 3 字节）。
///
/// 自检模式必须用 new_self_reception 构造帧，否则硬件会等一个永远不来的 ACK。
async fn send_frame(twai: &mut esp_hal::twai::Twai<'_, esp_hal::Async>, data: &[u8]) {
    let frame = if SELF_TEST {
        EspTwaiFrame::new_self_reception(StandardId::ZERO, data)
    } else {
        EspTwaiFrame::new(StandardId::ZERO, data)
    };

    match frame {
        Some(frame) => match twai.transmit_async(&frame).await {
            Ok(()) => rprintln!("已发送 ID=0 数据={:?}", data),
            Err(e) => rprintln!("发送失败：{:?}", e),
        },
        None => rprintln!("构造帧失败：数据长度超过 8 字节"),
    }
}
