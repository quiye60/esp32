// ============================================================================
// 41 · SDMMC：读 SD 卡的第 0 个扇区（MBR）
//
// 目标：走一遍「初始化和读卡」的完整流程，看清 SD 协议分层的价值：
//         1) SdHostController::new(SDHOST, Config) —— 主机控制器（共享引擎）
//         2) controller.slot::<1>(SlotConfig) + with_clk/with_cmd/with_data0
//            —— 一个「槽位」就是一组引脚（1-bit 模式只用 CLK/CMD/DAT0）
//         3) slot.into_async() —— esp-hal 的槽位实现 sdio::MmcBus，可以直接给
//            sdio 这个协议栈用（它负责 CMD0/CMD8/ACMD41/CMD58… 一大堆握手）
//         4) BlockDevice::new_sd_card(slot, 40 MHz, Delay) —— 拿到「块设备」
//         5) read_block(0, &mut buf) —— 读第 0 扇区，最后两字节应该是 0x55 0xAA
//       这就是「不用文件系统，直接读裸扇区」的写法；要文件就得再加 FAT 层。
//
// 参考：官方 qa-test/src/bin/sdmmc_sd_async.rs（它后面还挂了 FAT 文件系统）
//       esp-hal/src/sdmmc/mod.rs 的文档、sdio 0.5 的 BlockDevice
//       结构照 src/bin/tasks.rs
//
// 接线（S3 的 1-bit 模式，引脚和 SD 卡座一一对应）：
//       CLK=GPIO39、CMD=GPIO38、DAT0=GPIO40、3V3、GND（卡座一般自带这些定义）
// 运行：cargo run --release --bin 41_sdmmc
// 预期：打印卡信息 + 第 0 扇区内容，末尾能看到 0x55AA 签名和分区表
// 坑：  ① 40 MHz 初始化失败（超时）时，把 SlotConfig 里的 input_delay_phase 换
//          DelayPhase::_1 / _2 / _3 再试
//       ② 卡座走线差/线太长会读失败，SD 卡对信号质量很敏感
//       ③ 本课**只读不写**，避免破坏卡上的数据；要写请用 write_block 并自己做备份
//       ④ 真正做产品用 FAT（embedded-fatfs 等）或 littlefs，别自己解析文件系统
// ============================================================================

#![no_std]
#![no_main]

use aligned::{A4, Aligned};                // sdio 要求块缓冲 4 字节对齐
use block_device_driver::BlockDevice as _; // 块设备通用 trait：read / write / size
use embassy_executor::Spawner;
use embassy_time::{Delay, Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::sdmmc::{Config, DelayPhase, SdHostController, SlotConfig};
use esp_hal::timer::timg::TimerGroup;
use rtt_target::rprintln;
use sdio::{Addressable, BlockDevice, sd::Card};   // Addressable：卡容量等信息的来源

/// 卡时钟：40 MHz 是 SD 高速模式（SDR25）
const CARD_HZ: u32 = 40_000_000;
/// 单块大小固定 512 字节
const BLOCK_SIZE: usize = 512;

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

    // ---------------- 1. 主机控制器 + 槽位 ----------------
    let controller = SdHostController::new(peripherals.SDHOST, Config::default()).unwrap();

    // 输入采样相位：高频下最容易出问题的地方，读不出来就换 _1/_2/_3
    let slot_config = SlotConfig::default().with_input_delay_phase(DelayPhase::_0);
    let slot = controller
        .slot::<1>(slot_config)
        .unwrap()
        .with_clk(peripherals.GPIO39)
        .with_cmd(peripherals.GPIO38)
        .with_data0(peripherals.GPIO40)
        .into_async();                     // 异步模式才实现 sdio::MmcBus

    // ---------------- 2. 初始化卡 ----------------
    // new_sd_card 内部会做完整握手：上电、识别电压、读 OCR/CSD、设 4 线（这里 1 线）
    let mut card: BlockDevice<Card, _, _, BLOCK_SIZE> =
        match BlockDevice::new_sd_card(slot, CARD_HZ, Delay).await {
            Ok(c) => c,
            Err(e) => {
                rprintln!("SD 卡初始化失败：{:?}", e);
                rprintln!("检查：卡座接线 / 卡是否插好 / 试试 DelayPhase::_1.._3");
                loop {
                    Timer::after(Duration::from_secs(10)).await;
                }
            }
        };

    rprintln!("卡初始化成功：{:?}，当前时钟 {} Hz", card.card(), card.freq());
    rprintln!("容量信息：{:?} 块 × {} 字节", card.card().block_count(), BLOCK_SIZE);

    // ---------------- 3. 读第 0 扇区（MBR） ----------------
    // 注意：底层要求 4 字节对齐的缓冲，所以用 Aligned::<A4, ...> 包一层
    let mut blocks = [Aligned::<A4, [u8; BLOCK_SIZE]>([0u8; BLOCK_SIZE])];
    match card.read(0, &mut blocks).await {
        Ok(()) => {
            let b: &[u8; BLOCK_SIZE] = &blocks[0];
            rprintln!("第 0 扇区前 64 字节：{:02X?}", &b[..64]);
            rprintln!(
                "扇区最后两字节 = {:02X?}（0x55 0xAA 是 MBR 引导签名）",
                &b[510..512]
            );
            // MBR 分区表在 446..510，每项 16 字节
            for i in 0..4 {
                let e = &b[446 + i * 16..446 + (i + 1) * 16];
                if e[4] != 0 {
                    rprintln!("  分区 {}：类型 0x{:02X}，起始 LBA {}", i + 1, e[4], u32::from_le_bytes([e[8], e[9], e[10], e[11]]));
                }
            }
        }
        Err(e) => rprintln!("读第 0 扇区失败：{:?}", e),
    }

    rprintln!("（本课只读不写；要写数据请用 write_block，并先想好坏了怎么恢复）");

    loop {
        Timer::after(Duration::from_secs(30)).await;
    }
}
