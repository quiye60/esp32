// ============================================================================
// 52 · ESP-NOW：不用路由器，点对点直接发数据
//
// 目标：理解 ESP-NOW 这种「无连接、免配网」的小数据通道，它和 Wi-Fi/TCP 的区别：
//         没有 AP、没有 IP、没有握手 —— 只认 6 字节 MAC 地址，单包 ≤ 250 字节
//       本课同时演示三件事：
//         1) 广播发送（谁都能收）
//         2) 收到别人广播后，自动把对方加进 peer 列表，并一对一回复
//         3) 收发并行（select：谁先来先处理谁）
//
// 参考：官方 examples/esp-now/embassy_esp_now
//       结构照 src/bin/tasks.rs
//
// 接线：不需要任何外部器件
// 运行：cargo run --release --bin 52_esp_now
//       两块板子都烧同一份固件，就能互相看到收发；只有一块也能看到自己的广播包
// 预期：每 5 秒打印一次 "Send broadcast"，收到包时打印 "Received ..."，
//       并且给来源回一句 "Hello Peer"
// 坑：  ① 双方必须在同一信道（这里写死 11）
//       ② 加 peer 是收包后才做的：ESP-NOW 只允许给「已登记的 peer」单播
//       ③ 想加密就填 lmk（16 字节密钥），本课用明文
//       ④ 必须打开 esp-radio 的 esp-now 特性（本项目 Cargo.toml 已开）
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};   // 两个 future 谁先就绪走谁
use embassy_time::{Duration, Ticker};            // Ticker：固定周期的节拍器
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::esp_now::{BROADCAST_ADDRESS, EspNowWifiInterface, PeerInfo};
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

    // ESP-NOW 不需要协议栈，但驱动仍要堆
    esp_alloc::heap_allocator!(size: 72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // 拿 ESP-NOW 句柄：控制器本身用默认配置就行（不连 AP 也不做热点）
    let controller = esp_radio::wifi::WifiController::new(peripherals.WIFI, Default::default()).unwrap();
    let mut esp_now = controller.esp_now();

    // 信道要和对方一致（1~13），改这里两边都得改
    esp_now.set_channel(11).unwrap();
    rprintln!("ESP-NOW 版本 = {}", esp_now.version().unwrap());
    rprintln!("每隔 5 秒广播一次 \"0123456789\"，同时等别人发消息");

    let mut ticker = Ticker::every(Duration::from_secs(5));
    loop {
        // 这里体现「收发并行」：select 让两个 future 竞争，谁先完成用谁的结果
        let res = select(ticker.next(), async {
            let r = esp_now.receive_async().await;
            rprintln!("收到来自 {:02X?} 的 {} 字节：{:?}", r.info.src_address, r.data().len(), r.data());

            // 只有广播包才需要「主动登记对方」：ESP-NOW 只能给已知 peer 单播
            if r.info.dst_address == BROADCAST_ADDRESS {
                if !esp_now.peer_exists(&r.info.src_address) {
                    esp_now
                        .add_peer(PeerInfo {
                            interface: EspNowWifiInterface::Station,
                            peer_address: r.info.src_address,
                            lmk: None,               // 明文；想加密就给 16 字节密钥
                            channel: None,           // None = 用当前信道
                            encrypt: false,
                        })
                        .unwrap();
                    rprintln!("已把 {:02X?} 加入 peer 列表", r.info.src_address);
                }
                // 一对一回复刚才那个来源
                match esp_now.send_async(&r.info.src_address, b"Hello Peer").await {
                    Ok(()) => rprintln!("已回复 Hello Peer"),
                    Err(e) => rprintln!("回复失败：{:?}", e),
                }
            }
        })
        .await;

        match res {
            // Either::First = 节拍到了（该广播了）；Second = 收到了数据（上面已处理）
            Either::First(_) => {
                rprintln!("--- 广播发送 ---");
                match esp_now.send_async(&BROADCAST_ADDRESS, b"0123456789").await {
                    Ok(()) => rprintln!("广播成功（局域网内所有 ESP-NOW 设备都能收到）"),
                    Err(e) => rprintln!("广播失败：{:?}", e),
                }
            }
            Either::Second(_) => {}
        }
    }
}
