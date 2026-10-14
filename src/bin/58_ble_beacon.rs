// ============================================================================
// 58 · BLE 信标（Beacon）：只广播、不连接，把数据「喊」出去
//
// 目标：Beacon 是 BLE 里最省的形态 —— 不建立连接、不握手机，只是不停地广播。
//       常见用途：iBeacon 定位、温湿度计、电子价签、防丢器。
//       本课演示三件事：
//         1) Advertisement::NonconnectableNonscannableUndirected
//            —— 不可连接、不可扫描：省电到极致（对方只能看广播包）
//         2) ManufacturerSpecificData —— 厂商自定义数据（要填公司 ID，这里是占位 0xFFFF）
//         3) update_adv_data —— 不改连接、不断广播地「换内容」：
//            本课每 10 ms 把「运行毫秒数」写进广播包，相当于做成了一个计时信标
//
// 参考：trouble-host 官方 examples/apps/src/ble_beacon.rs
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件
// 运行：cargo run --release --bin 58_ble_beacon
// 预期：一直广播 "Trouble Beacon"；用手机 App（nRF Connect 的 Scanner 页）
//       能看到这个设备，并看到 Manufacturer Data 里的数字在飞快变化
// 坑：  ① 传统广播包只有 31 字节，名字 + 标志 + 厂商数据要挤进去
//       ② 广播间隔别设太小（本课 30 ms）：省电与「更新实时性」要折中
//          （BLE 规定最小 20 ms，写成 25 ms 以下有些设备会不适）
//       ③ 厂商 ID 是向 Bluetooth SIG 申请的，自己玩用 0xFFFF 占位
//       ④ 不可连接广播**没有** ATT/GATT 层，对方不能读特征、只能解析广播内容
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use rtt_target::rprintln;
use trouble_host::prelude::*;

/// 自己玩用 0xFFFF 占位；真产品要填自己公司的 ID
const COMPANY_ID: u16 = 0xFFFF;

/// 组厂商数据：8 字节 = 4 字节「广播次数」+ 4 字节「开机以来毫秒数」（都是大端）
fn make_payload(start: Instant, count: u32) -> [u8; 8] {
    let mut data = [0u8; 8];
    let elapsed_ms = Instant::now().duration_since(start).as_millis() as u32;
    data[0..4].copy_from_slice(&count.to_be_bytes());
    data[4..8].copy_from_slice(&elapsed_ms.to_be_bytes());
    data
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

    esp_alloc::heap_allocator!(size: 72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    let connector = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let controller: ExternalController<_, 1> = ExternalController::new(connector);

    // 信标不连接任何设备，所以「连接数」给 0；但需要 1 个广播集（ADV_SETS）
    let mut resources: HostResources<_, DefaultPacketPool, 0, 0, 1> = HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]))
        .build();

    let mut peripheral = stack.peripheral();
    let mut runner = stack.runner();

    // ---------------- 1. 组广播数据（≤31 字节） ----------------
    let mut adv_data = [0; 64];
    let mut update_count = 0u32;
    let start = Instant::now();

    let build = |buf: &mut [u8; 64], count: u32| {
        AdStructure::encode_slice(
            &[
                AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
                AdStructure::CompleteLocalName(b"Trouble Beacon"),
                AdStructure::ManufacturerSpecificData {
                    company_identifier: COMPANY_ID,
                    payload: &make_payload(start, count),
                },
            ],
            buf,
        )
        .unwrap()
    };

    let len = build(&mut adv_data, update_count);

    // ---------------- 2. 开始广播（不可连接、不可扫描） ----------------
    // 广播间隔 20~150 ms：越小更新越快、越费电
    let mut params = AdvertisementParameters::default();
    params.interval_min = Duration::from_millis(25);
    params.interval_max = Duration::from_millis(150);

    rprintln!("开始广播「Trouble Beacon」（不可连接信标）");

    // runner 必须一直跑，广播与刷新数据并行进行
    let _ = join(runner.run(), async {
        let _advertiser = peripheral
            .advertise(
                &params,
                Advertisement::NonconnectableNonscannableUndirected {
                    adv_data: &adv_data[..len],
                },
            )
            .await
            .unwrap();

        loop {
            Timer::after(Duration::from_millis(10)).await;
            update_count = update_count.wrapping_add(1);

            // 重新组包并替换广播内容（不需要断开任何连接，因为本来就没有连接）
            let len = build(&mut adv_data, update_count);
            peripheral
                .update_adv_data(Advertisement::NonconnectableNonscannableUndirected {
                    adv_data: &adv_data[..len],
                })
                .await
                .unwrap();

            // 每 100 次报一声，证明「广播内容在持续刷新」
            if update_count % 100 == 0 {
                rprintln!(
                    "已刷新广播数据 {} 次，运行 {} ms",
                    update_count,
                    Instant::now().duration_since(start).as_millis()
                );
            }
        }
    })
    .await;

    loop {
        Timer::after(Duration::from_secs(10)).await;
    }
}
