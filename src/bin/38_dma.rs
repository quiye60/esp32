// ============================================================================
// 38 · DMA：让数据搬运不占 CPU（内存到内存的搬运）
//
// 目标：DMA 的价值是「CPU 去干别的，搬运交给外设」。本课用 mem2mem DMA 做一次
//       4 KB 的拷贝，并把 DMA 的四个要点讲清：
//         1) dma_buffers! 宏生成「对齐 + 描述符齐全」的缓冲区（对齐很关键）
//         2) Mem2Mem::new(DMA_CH0, SPI2)：S3 上 mem2mem 借用 SPI2 的 DMA 通道
//         3) with_descriptors(...) 装填描述符链（DMA 靠描述符知道搬哪里、搬多少）
//         4) start_transfer(...) → wait()：发起后可以 await，等完成再回来
//       顺便对比：同样的拷贝用 CPU 循环要花多久。
//
// 参考：官方 examples/peripheral/dma/mem2mem、examples/peripheral/dma/extmem2mem
//       esp-hal/src/dma/ 下的 m2m.rs、dma_buffers 宏
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件
// 运行：cargo run --release --bin 38_dma
// 预期：每 2 秒做一次拷贝，打印 CPU 版和 DMA 版的耗时，并确认数据一致
// 坑：  ① dma_buffers! 的缓冲区是静态的，占的是 RAM 而不是栈，别开太大
//       ② 缓冲必须按 DMA 要求对齐（宏已处理好；自己拿 Vec 很容易踩坑）
//       ③ 传完了所有权才还给你：start_transfer 会吃掉两个 buffer，
//          wait 之后用 into_inner() 拿回来，这是 esp-hal 的防呆设计
//       ④ 内部 RAM 与 PSRAM 之间也能搬，但 PSRAM 带宽低，速度会差一截
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::dma::{BurstConfig, Mem2Mem};
use esp_hal::dma_buffers;                 // 宏：生成对齐的收发缓冲 + 描述符
use esp_hal::time::Instant;
use esp_hal::timer::timg::TimerGroup;
use rtt_target::rprintln;

/// 一次搬运的数据量
const DATA_SIZE: usize = 1024 * 4;

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

    // ---------------- 1. 准备缓冲区 ----------------
    // 宏返回 (rx_buffer, rx_descriptors, tx_buffer, tx_descriptors)：
    //   rx_buffer = 目标（DMA 往里写），tx_buffer = 源（DMA 从这读）
    let (mut rx_buffer, rx_descriptors, tx_buffer, tx_descriptors) = dma_buffers!(DATA_SIZE);

    // ---------------- 2. 建 mem2mem 通道 ----------------
    // S3 上 mem2mem 走 SPI2 的 DMA（借用），所以第二个参数传 SPI2
    let mem2mem = Mem2Mem::new(peripherals.DMA_CH0, peripherals.SPI2);
    let mut mem2mem = mem2mem
        .with_descriptors(rx_descriptors, tx_descriptors, BurstConfig::default())
        .unwrap();

    // ---------------- 3. 准备源数据 ----------------
    for (i, byte) in tx_buffer.iter_mut().enumerate() {
        *byte = (i % 251) as u8;          // 用不规则图案，避免「刚好看着一样」的错判
    }
    rprintln!("每次搬运 {} 字节，源数据已准备好", DATA_SIZE);

    loop {
        // ---------------- 4. CPU 版拷贝（对照组） ----------------
        let t0 = Instant::now();
        rx_buffer.copy_from_slice(&tx_buffer);
        let cpu_us = t0.elapsed().as_micros();

        // ---------------- 5. DMA 版拷贝 ----------------
        // start_transfer 吃掉 buffer（防止搬运途中被改），返回一个等待句柄
        let t1 = Instant::now();
        // start_transfer 只是「借用」缓冲区（&mut / &），所以 wait 之后还能继续用它们
        match mem2mem.start_transfer(&mut rx_buffer, &tx_buffer) {
            Ok(dma_wait) => {
                dma_wait.wait().unwrap();          // 等 DMA 搬完
                let dma_us = t1.elapsed().as_micros();

                let same = rx_buffer == tx_buffer;
                rprintln!(
                    "CPU 拷贝 {} µs，DMA 拷贝 {} µs",
                    cpu_us,
                    dma_us
                );
                rprintln!(
                    "数据一致性：{}（DMA 真正省下的不只是时间，而是 await 期间 CPU 能去跑别的任务）",
                    if same { "一致" } else { "不一致" }
                );
            }
            Err(e) => rprintln!("发起 DMA 失败：{:?}", e),
        }

        Timer::after(Duration::from_secs(2)).await;
    }
}
