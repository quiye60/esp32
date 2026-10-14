// ============================================================================
// 35 · PSRAM：把外部「伪静态 RAM」接进堆，一次能开几百 KB 的缓冲
//
// 目标：片内 SRAM 只有 512 KB，做摄像头/音频/大缓冲很快就不够用。带 PSRAM 的模组
//       （例如 ESP32-S3-WROOM-1 N16R8 里的 R8 = 8 MB Octal PSRAM）能补上这块。
//       本课的流程：
//         1) 编译期配置 PSRAM 模式（Octal / Quad，见下面「两个必须的开关」）
//         2) esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram)
//            —— 把 PSRAM 注册成堆的一个区域
//         3) 分配一个几百 KB 的 Vec，打印它的地址（会落在 PSRAM 地址段 0x3C00_0000+）
//         4) 用 HEAP.stats() 看区域划分：内部 RAM 一块、PSRAM 一块
//
// 两个必须的开关（**缺一不可**，不然这段代码要么编译不过，要么运行时报错）：
//   ① .cargo/esp-config.toml 的 [env] 里放开：
//        ESP_HAL_CONFIG_PSRAM_MODE = "octal"     # 八线 PSRAM；四线用 "quad"
//      改完必须 `cargo clean`（这是编译期配置，改配置不重编不会生效）
//   ② 用 --release 编译：debug 模式下 PSRAM 初始化会因为时序过慢而失败
//        cargo run --release --bin 35_psram
//
// 参考：官方 qa-test/src/bin/psram.rs、hil-test/src/bin/alloc_psram.rs
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件（前提：你的模组真的带 PSRAM）
// 预期：打印 "PSRAM 区域 = ... 字节"，分配出的 Vec 地址在 0x3C00_0000 段
// 坑：  ① 没有 PSRAM 的模组（N4、N8 无 R 后缀）跑到这里会初始化失败/地址段为空
//       ② PSRAM 比内部 RAM 慢不少（几十 MHz 的串行接口），别把高频访问的数据放里面
//       ③ DMA 只能搬到 PSRAM 的某些地址范围，做大缓冲时留意手册
//       ④ PSRAM 掉电即失，别拿它当「非易失存储」用
// ============================================================================

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
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

    // ---------------- 1. 内部 RAM 堆（小、快） ----------------
    esp_alloc::heap_allocator!(size: 16 * 1024);

    // ---------------- 2. 把 PSRAM 挂进堆（大、慢） ----------------
    // 这个宏内部会初始化 PSRAM（按上面 ESP_HAL_CONFIG_PSRAM_MODE 选定的模式），
    // 然后把整片 PSRAM 注册成堆的一块区域
    esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // ---------------- 3. 看堆的区域划分 ----------------
    let stats = esp_alloc::HEAP.stats();
    rprintln!("堆总量 {} 字节，已用 {}", stats.size, stats.current_usage);
    for (i, region) in stats.region_stats.iter().enumerate() {
        if let Some(r) = region {
            rprintln!("  区域 {}：总 {} 字节，空闲 {}", i, r.size, r.free);
        }
    }

    // ---------------- 4. 分配一块「内部 RAM 装不下」的内存 ----------------
    // 一次 400 KB：内部 SRAM 总共才 512 KB，这一块只能落在 PSRAM 里
    let mut big: Vec<u32> = Vec::with_capacity(400 * 1024 / 4);
    for i in 0..(400 * 1024 / 4) {
        big.push((i & 0xffff) as u32);
    }
    rprintln!(
        "分配 400 KB Vec 成功：{} 个元素，起始地址 {:p}",
        big.len(),
        big.as_ptr()
    );
    rprintln!("（地址落在 0x3C00_0000 段就是 PSRAM，落在 0x3FC8_0000 附近就是内部 RAM）");
    rprintln!("首尾抽查：big[0] = {}，big[最后] = {}", big[0], big[big.len() - 1]);

    let s = String::from("这行字符串也可能被分配到 PSRAM");
    rprintln!("String = {:?}", s);

    rprintln!("分配后：已用 {} 字节", esp_alloc::HEAP.stats().current_usage);

    loop {
        Timer::after(Duration::from_secs(30)).await;
    }
}
