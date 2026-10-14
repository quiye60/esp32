// ============================================================================
// 33 · 堆内存：Vec / String 从哪来，以及 reclaimed RAM 是怎么回事
//
// 目标：嵌入式 Rust 一样能用 Vec / String / Box，只要给分配器一块内存。本课讲清：
//         1) heap_allocator! 把一段 RAM 注册成堆（可以有多个区域，形成「区域列表」）
//         2) #[ram(reclaimed)] 的意思：这段 RAM 平时被 ROM/蓝牙/协议栈占着，
//            我们「抢回来」当堆用（不能和 Wi-Fi/蓝牙同时用！）
//         3) 怎么看堆用得好不好：HEAP.stats() 给出总大小/当前用量
//         4) 内存的三个大坑：碎片、栈溢出、堆太小
//
// 参考：官方 hil-test/src/bin/alloc_dcache_reclaimed.rs、alloc_psram.rs
//       esp-alloc 的 README、本项目 teach/02-原理-no_std与启动与内存.md
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件
// 运行：cargo run --release --bin 33_alloc
// 预期：打印堆总量、每次分配后的用量、以及一个「不断分配直到失败」的演示
// 坑：  ① 堆用完不会 panic，而是分配失败（Vec 扩容失败会 abort，所以别把堆当无限用）
//       ② #[ram(reclaimed)] 的堆和 Wi-Fi/BLE 冲突：无线课程里要把这行去掉
//       ③ 堆的碎片化：频繁分配/释放不同大小，会让你「总量够但还是分配失败」
//       ④ 任务栈也有大小限制：局部大数组既占栈也可能溢出（用堆更安全）
// ============================================================================

#![no_std]
#![no_main]

extern crate alloc;                      // 打开 alloc crate（no_std 下默认不带）

use alloc::string::String;
use alloc::vec::Vec;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::ram;                        // #[ram(reclaimed)] 属性
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

    // ---------------- 1. 注册两块堆 ----------------
    // 第一块：从「被 ROM 回收的 RAM」里划 32 KB。
    //   reclaimed 的代价：和 Wi-Fi / BLE 抢内存，所以无线项目里不要开这块。
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 32 * 1024);
    // 第二块：普通内部 RAM 里划 16 KB（这块随时都能用）
    esp_alloc::heap_allocator!(size: 16 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // ---------------- 2. 看一眼堆的初始状态 ----------------
    let stats = esp_alloc::HEAP.stats();
    rprintln!("堆总量 {} 字节，当前已用 {} 字节", stats.size, stats.current_usage);
    // 区域列表：region_stats 里每个 Some 就是一块注册过的堆
    for (i, region) in stats.region_stats.iter().enumerate() {
        if let Some(r) = region {
            rprintln!(
                "  区域 {}：分配器总 {} 字节，已用 {}，空闲 {}",
                i, r.size, r.used, r.free
            );
        }
    }

    // ---------------- 3. 用 Vec：动态数组 ----------------
    // 在 no_std 里 Vec 需要 alloc crate + 全局分配器，两者都已就绪
    let mut v: Vec<u32> = Vec::new();
    for i in 0..1000 {
        v.push(i);
    }
    rprintln!(
        "Vec 里放了 {} 个 u32（容量 {}），元素在堆上的地址 {:p}",
        v.len(),
        v.capacity(),
        v.as_ptr()
    );
    rprintln!("第 500 个元素 = {}", v[500]);
    rprintln!("分配 Vec 后：已用 {} 字节", esp_alloc::HEAP.stats().current_usage);

    // ---------------- 4. 用 String：可变字符串 ----------------
    let mut s = String::from("hello");
    s.push_str(", esp32s3");
    rprintln!("String = {:?}，长度 {}", s, s.len());

    // ---------------- 5. Box：把大对象放堆上，避免撑爆任务栈 ----------------
    // 局部变量在「栈」上，任务栈有限；用 Box 换成堆分配
    let big = alloc::boxed::Box::new([0u8; 4096]);
    rprintln!("Box<[u8; 4096]> 放在 {:p}（堆里，而不是栈上）", &big[0] as *const u8);

    // ---------------- 6. 显式释放看一眼用量变化 ----------------
    drop(v);
    drop(s);
    drop(big);
    rprintln!("释放后：已用 {} 字节", esp_alloc::HEAP.stats().current_usage);

    // ---------------- 7. 一直分配会怎样？ ----------------
    // 「堆不够」时分配返回 None 而不是 panic：用 try_reserve 能优雅处理
    let mut probe: Vec<u8> = Vec::new();
    let mut chunks = 0u32;
    loop {
        if probe.try_reserve(1024).is_err() {
            rprintln!("堆已满：成功分配 {} 次 × 1 KB，第 {} 次失败", chunks, chunks + 1);
            break;
        }
        probe.resize(probe.len() + 1024, 0);   // 真正占住这块空间
        chunks += 1;
    }
    drop(probe);
    rprintln!("全部释放，现在已用 {} 字节", esp_alloc::HEAP.stats().current_usage);

    loop {
        Timer::after(Duration::from_secs(30)).await;
    }
}
