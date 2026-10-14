// ============================================================================
// 34 · Flash 读写：NOR Flash 的「只能写 0、必须先擦」规则
//
// 目标：搞清片内 Flash 与 RAM 的三个本质区别：
//         1) 擦除单位是「扇区」（4 KB），擦完一个扇区所有位都是 1（0xFF）
//         2) 写入只能把 1 变成 0，所以「改写」= 先擦扇区再写
//         3) 写入要 4 字节对齐；掉电会丢半页数据，重要数据要留备份/校验
//       本课用 esp-storage 的 FlashStorage 做：读 → 擦 → 写 → 读回校验。
//
// 参考：官方 hil-test/src/bin/storage.rs、esp-storage 的 storage.rs / nor_flash.rs
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件
// 运行：cargo run --release --bin 34_flash
// 预期：打印 Flash 总容量、写入一串数据、读回来对比一致，
//       然后再写一次，看「0 变不回 1」；最后用「擦 → 写」的正确顺序修正
// 坑：  ① **别写 0x9000**：那是 NVS 分区（存 WiFi 校准和配网信息）
//          本课写「最后一个扇区」，正常布局下固件不会用到那里
//       ② 擦写次数有限（约 10 万次/扇区），别在循环里疯狂写
//       ③ 想正经存数据用 NVS / littlefs / 自己划的分区，别硬编码偏移
//       ④ 写完要读回来校验：Flash 写入并不总是 100% 可靠
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_storage::FlashStorage;
use rtt_target::rprintln;

/// 擦除/写入的单位：一个扇区 4 KB
const SECTOR_SIZE: u32 = 4096;

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

    let mut flash = FlashStorage::new(peripherals.FLASH);

    // ---------------- 1. 基本信息和一次「无害的读」 ----------------
    let capacity = flash.capacity() as u32;
    rprintln!("Flash 总容量 = {} 字节（{} MB）", capacity, capacity / 1024 / 1024);

    // 0x10000 起是 app 镜像，前 32 字节是镜像头（magic 0xE9 ...），读它最安全
    let mut header = [0u8; 32];
    flash.read(0x10000, &mut header).unwrap();
    rprintln!("app 镜像头（偏移 0x10000）= {:02X?}", header);

    // ---------------- 2. 选一个「安全地址」：最后一个扇区 ----------------
    let base = capacity - SECTOR_SIZE;
    rprintln!("本课使用地址 0x{:06X}（最后一个 4 KB 扇区）", base);

    // ---------------- 3. 擦除：整扇区变成 0xFF ----------------
    flash.erase(base, base + SECTOR_SIZE).unwrap();
    let mut buf = [0u8; 16];
    flash.read(base, &mut buf).unwrap();
    rprintln!("擦除后读回：{:02X?}（应该全是 FF）", buf);

    // ---------------- 4. 写入：长度要是 4 的倍数，且只能 1→0 ----------------
    let payload: [u8; 16] = [
        0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04, 0xAA, 0xBB, 0xCC, 0xDD, 0x55, 0x66,
        0x77, 0x88,
    ];
    flash.write(base, &payload).unwrap();

    let mut readback = [0u8; 16];
    flash.read(base, &mut readback).unwrap();
    rprintln!("写入：{:02X?}", payload);
    rprintln!("读回：{:02X?}", readback);
    if readback == payload {
        rprintln!(">> 读回内容与写入一致");
    } else {
        rprintln!(">> 不一致（Flash 写入出错了？）");
    }

    // ---------------- 5. 再写一次：体验「不能把 0 变回 1」 ----------------
    // 想写 0xFF，但原来的位已经被写成别的值了 —— 结果只会是「按位与」
    let second: [u8; 16] = [0xFF; 16];
    flash.write(base, &second).unwrap();
    flash.read(base, &mut readback).unwrap();
    rprintln!("第二次写 0xFF 后读回：{:02X?}", readback);
    rprintln!("（原值没变回 1 —— 这就是「必须先擦除」的原因）");

    // ---------------- 6. 正确的「改写」流程：擦 → 写 ----------------
    flash.erase(base, base + SECTOR_SIZE).unwrap();
    flash.write(base, &second).unwrap();
    flash.read(base, &mut readback).unwrap();
    rprintln!("擦除后写 0xFF：{:02X?}（这次对了）", readback);

    rprintln!("演示结束。想真存数据请用 NVS / littlefs，并给数据加校验和");

    loop {
        Timer::after(Duration::from_secs(30)).await;
    }
}
