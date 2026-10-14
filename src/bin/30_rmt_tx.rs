// ============================================================================
// 30 · RMT 发送：自己拼任意波形（WS2812 / 红外遥控 / DHT 的思路）
//
// 目标：RMT 是「可编程脉冲发生器」，能把一串「高电平几拍、低电平几拍」精确打出去，
//       时序完全由硬件保证，CPU 只管准备好数据。本课走四步：
//         1) Rmt::new(peripherals.RMT, 时钟频率) —— 先定好 1 拍 = 多少 ns
//         2) PulseCode::new(电平1, 拍数1, 电平2, 拍数2) —— 一个 code 描述两个沿
//         3) channel.transmit(&codes)            —— 把整段波形一次性发出去
//         4) 结尾必须有 PulseCode::end_marker()，否则硬件不知道什么时候停
//       最后给出 WS2812（RGB 灯带）的时序换算，接上灯带就能点亮。
//
// 参考：官方 examples/async/embassy_rmt_tx、examples/async/embassy_rmt_rx
//       结构照 src/bin/tasks.rs
//
// 接线：GPIO4 接逻辑分析仪/示波器看波形；接 WS2812 灯带的数据线也能看到颜色循环
// 运行：cargo run --release --bin 30_rmt_tx
// 预期：每秒打印一次「发了多少 code」，并切一次 WS2812 的颜色
// 坑：  ① 时间精度由「主时钟 ÷ 分频」决定：80 MHz 分频 255 ≈ 3.2 µs/拍（粗），
//          分频 8 = 0.1 µs/拍（WS2812 正好用它），两个通道各配各的
//       ② PulseCode 的拍数是 u16，单个 code 最长 65535 拍，超长要拆开
//       ③ 一次 transmit 的 code 数量受 RMT RAM 限制（CHANNEL_RAM_SIZE）
//       ④ end_marker 之后别再跟有效数据，硬件看到它就开始收尾
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::Level;
use esp_hal::rmt::{PulseCode, Rmt, TxChannelConfig, TxChannelCreator};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use rtt_target::rprintln;

/// WS2812 用的灯珠数量（接上灯带才看得到效果）
const WS2812_LEDS: usize = 8;

/// 把「一个字节的 8 个 bit」按 WS2812 时序展开成 RMT code。
///
/// WS2812 用高电平的持续时间区分 0 和 1（1 拍 = 0.1 µs 时）：
///   bit 0：高 4 拍 + 低 8 拍（0.4 µs / 0.8 µs）
///   bit 1：高 8 拍 + 低 4 拍（0.8 µs / 0.4 µs）
/// 每个 code 描述「高一段 + 低一段」，所以正好一个 bit 一个 code。
fn encode_ws2812_byte(byte: u8, out: &mut [PulseCode], base: usize) {
    for bit in 0..8 {
        // 高位先发（WS2812 是 MSB first）
        let is_one = (byte & (0x80 >> bit)) != 0;
        out[base + bit] = if is_one {
            PulseCode::new(Level::High, 8, Level::Low, 4)
        } else {
            PulseCode::new(Level::High, 4, Level::Low, 8)
        };
    }
}

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

    // RMT 的基准时钟：S3 上是 80 MHz（= 12.5 ns/拍）
    let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(80))
        .unwrap()
        .into_async();                     // 转异步：transmit 可以 await，不阻塞其他任务

    // ---------------- 1. 普通脉冲序列（给示波器看） ----------------
    // 分频 255 → 1 拍 ≈ 3.19 µs；20 个「高 200 拍 + 低 50 拍」
    let mut channel = rmt
        .channel0
        .configure_tx(&TxChannelConfig::default().with_clk_divider(255))
        .unwrap()
        .with_pin(peripherals.GPIO4);

    let mut pulses = [PulseCode::new(Level::High, 200, Level::Low, 50); 20];
    // 收尾用「长高 + 长低」，方便在示波器上分辨帧边界
    pulses[pulses.len() - 2] = PulseCode::new(Level::High, 3000, Level::Low, 500);
    pulses[pulses.len() - 1] = PulseCode::end_marker();  // 必须有：告诉硬件「到此结束」

    // ---------------- 2. WS2812 用另一个通道（时间精度不同） ----------------
    // 分频 8 → 1 拍 = 0.1 µs，正好匹配 WS2812 的时序要求
    let mut ws_channel = rmt
        .channel1
        .configure_tx(&TxChannelConfig::default().with_clk_divider(8))
        .unwrap()
        .with_pin(peripherals.GPIO5);      // 灯带的数据线接这里（GPIO4 被上面占用了）

    // 8 颗灯 × 3 字节(GRB) × 8 bit + 1 个结束标记
    let mut ws_codes = [PulseCode::end_marker(); WS2812_LEDS * 3 * 8 + 1];

    let mut step = 0u8;
    loop {
        // ---- 发普通脉冲序列 ----
        rprintln!("transmit 普通脉冲序列（{} 个 code）", pulses.len());
        if let Err(e) = channel.transmit(&pulses).await {
            rprintln!("发送失败：{:?}", e);
        }

        // ---- 发一帧 WS2812：颜色循环 红 → 绿 → 蓝 → 白 ----
        let (r, g, b) = match step % 4 {
            0 => (32u8, 0, 0),
            1 => (0, 32, 0),
            2 => (0, 0, 32),
            _ => (16, 16, 16),
        };
        // WS2812 的字节顺序是 GRB，不是 RGB
        let frame_bytes = [g, r, b];
        for led in 0..WS2812_LEDS {
            for (i, byte) in frame_bytes.iter().enumerate() {
                encode_ws2812_byte(*byte, &mut ws_codes, (led * 3 + i) * 8);
            }
        }
        rprintln!(
            "transmit WS2812 一帧（{} 颗灯，颜色 step={}，共 {} 个 code）",
            WS2812_LEDS,
            step % 4,
            ws_codes.len()
        );
        if let Err(e) = ws_channel.transmit(&ws_codes).await {
            rprintln!("WS2812 发送失败：{:?}", e);
        }

        step = step.wrapping_add(1);
        Timer::after(Duration::from_secs(1)).await;
    }
}
