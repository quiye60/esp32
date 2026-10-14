// ============================================================================
// 39 · I2S 接收：数字音频是怎么进来的
//
// 目标：I2S 是「音频专用串行总线」，一根时钟一根帧同步加数据线。本课走接收路径：
//         1) I2s::new(I2S0, DMA通道, TdmConfig) —— 用标准 Philips TDM 格式
//            TdmConfig 里三件事：采样率（44.1 kHz）、数据位宽（16 bit）、声道数（立体声）
//         2) .with_mclk(...) 主时钟，再 .into_async() 走异步
//         3) i2s_rx.with_bclk/.with_ws/.with_din 指定引脚，build() 拿到接收端
//         4) dma_rx_stream_buffer! 建「环形 DMA 缓冲」，用事务(transaction)取数据：
//            wait_for_available_async() 等数据到手，pop() 取走一段
//       这就是「连续不间断采样」的写法：DMA 一直在填缓冲，CPU 只要按时取走。
//
// 参考：官方 qa-test/src/bin/embassy_i2s_read.rs（本项目按它的结构改写）
//       esp-hal/src/i2s/master/mod.rs 的文档
//       结构照 src/bin/tasks.rs
//
// 接线：MCLK=GPIO0、BCLK=GPIO2、WS=GPIO4、DIN=GPIO5
//       没有音频源时：把 DIN 接 3V3 读到的就是 0xFF 全 1，接 GND 就是 0x00；
//       把 DIN 接到 WS 上能看到两种交替的值（最省事的“有数据在动”验证）
// 运行：cargo run --release --bin 39_i2s
// 预期：每秒打印 several 次「available N bytes」和首尾几个字节
// 坑：  ① 采样率/位宽必须和外部器件一致，否则听到的是噪声（或根本收不到）
//       ② DMA 缓冲越大越不容易丢数据，但延迟也越大；音频常用 20~200 ms
//       ③ 本项目用 probe-rs + RTT 打印，数据量大时打印本身会拖慢采集
//       ④ 想发声音（喇叭）就把 i2s_rx 换成 i2s_tx，参考 embassy_i2s_sound
//       ⑤ 注：这是本项目里唯一会触发 Xtensa LLVM 工具链 bug 的课（fat LTO 下
//          报 "Cannot select: PCREL_WRAPPER"），所以 Cargo.toml 的 release 配置
//          用 lto = "thin"，这样 cargo build --release --bins 才能全部编过
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::Timer;
use esp_hal::clock::CpuClock;
use esp_hal::dma_rx_stream_buffer;         // 宏：环形 DMA 接收缓冲
use esp_hal::i2s::master::{Channels, DataFormat, I2s, TdmConfig};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
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

    // ---------------- 1. 环形 DMA 缓冲 ----------------
    // 参数含义：总缓冲 4092 × 8 字节，其中「一次事务」能连续取走 2048 字节
    let buffer = dma_rx_stream_buffer!(4092 * 8, 2048);

    // ---------------- 2. 建 I2S 主机 ----------------
    // TdmConfig::new_tdm_philips() = 最常见的 I2S 时序（左对齐延迟一位）
    let i2s = I2s::new(
        peripherals.I2S0,
        peripherals.DMA_CH0,
        TdmConfig::new_tdm_philips()
            .with_sample_rate(Rate::from_hz(44_100))          // CD 音质
            .with_data_format(DataFormat::Data16Channel16)     // 16 位数据、16 位槽
            .with_channels(Channels::STEREO),
    )
    .unwrap()
    .with_mclk(peripherals.GPIO0)                             // 主时钟：给外部 codec 用
    .into_async();

    // ---------------- 3. 配引脚，拿到接收端 ----------------
    // BCLK = 位时钟，WS = 左右声道选择（帧同步），DIN = 数据输入
    let i2s_rx = i2s
        .i2s_rx
        .with_bclk(peripherals.GPIO2)
        .with_ws(peripherals.GPIO4)
        .with_din(peripherals.GPIO5)
        .build();

    rprintln!("I2S 已启动：44.1 kHz / 16 bit / 立体声，开始接收……");

    // ---------------- 4. 取数据 ----------------
    let mut data = [0u8; 9000];
    let mut transaction = i2s_rx.read(buffer).ok().unwrap();

    loop {
        // 等到 DMA 攒够可读的数据（等待期间 CPU 干别的）
        transaction.wait_for_available_async().await.unwrap();

        let avail = transaction.available_bytes();
        if avail > 0 {
            // 先算出能取多少（避免同时可变/不可变借用 data）
            let want = avail.min(data.len());
            // 把数据搬到本地缓冲（返回值是实际搬走的字节数）
            let count = transaction.pop(&mut data[..want]);
            rprintln!(
                "可读 {} 字节，取走 {} 字节：开头 {:02X?} … 结尾 {:02X?}",
                avail,
                count,
                &data[..count.min(8)],
                &data[count.saturating_sub(8)..count]
            );
        }

        Timer::after(embassy_time::Duration::from_millis(200)).await;
    }
}
