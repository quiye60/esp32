// ============================================================================
// 53 · BLE 外设（GATT 服务器）：让手机连上板子读写特征值
//
// 目标：亲手搭一个 BLE 从机（Peripheral），这是做 BLE 项目的第一步：
//         1) 用 esp-radio 的 BleConnector 拿到控制器，交给 trouble 这个 host
//         2) 用 #[gatt_server] / #[gatt_service] 宏声明服务与特征（编译期生成属性表）
//         3) 广播 → 等连接 → 处理读/写事件 → 主动 notify
//       重点理解三个角色：Controller（射频+链路层）、Host（GATT/GAP 逻辑，trouble 干的）、
//       Attribute Server（你的数据表）。
//
// 参考：trouble-host 官方 examples/apps/src/ble_bas_peripheral.rs
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件
// 运行：cargo run --release --bin 53_ble_gatt_peripheral
//       手机装 nRF Connect（Android/iOS 都有）：
//       扫描 → 连接 "TrouBLE" → 展开 Battery Service 0x180F →
//       读 Battery Level（0x2A19）、点三个箭头订阅通知 → 数值每 2 秒变一次
// 预期：RTT 打印连接事件、读写事件、每次推送的电量和 RSSI
// 坑：  ① BLE 需要 72 KB 左右的堆，少了会在建栈时 panic
//       ② 传统广播包只有 31 字节，设备名+标志+UUID 要挤在里面（见 advertise 注释）
//       ③ notify 只有在对方订阅（写 CCCD）后才会真的发出去
//       ④ 想和本项目第 54 课对练：54 课就是按本课的固定地址去连的
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::{Spawner, task};
use embassy_futures::join::join;
use embassy_futures::select::select;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;    // 控制器的 HCI 传输层实现
use rtt_target::rprintln;
use trouble_host::prelude::*;                    // Controller / Stack / GATT / 宏

/// 最多 1 个连接、2 个 L2CAP 通道（信令 + ATT）
const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 2;

// ---------------- GATT 数据库 ----------------

#[gatt_server]
struct Server {
    battery_service: BatteryService,
}

/// 标准电池服务（0x180F）
#[gatt_service(uuid = service::BATTERY)]
struct BatteryService {
    /// 电量：可读 + 可通知，初值 10；属性表里的 descriptor 也是标准要求的
    #[descriptor(uuid = descriptors::VALID_RANGE, read, value = [0, 100])]
    #[descriptor(uuid = descriptors::MEASUREMENT_DESCRIPTION, name = "hello", read, value = "Battery Level", type = &'static str)]
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 10)]
    level: u8,

    /// 自定义特征：可读可写可通知（读写它可以看到「上位机 → 板子」的方向）
    #[characteristic(uuid = "408813df-5dd4-1f87-ec11-cdb001100000", write, read, notify)]
    status: (),
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    rtt_target::rtt_init_print!();

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // BLE host 会动态分配连接/缓冲，72 KB 是官方例程的推荐值
    esp_alloc::heap_allocator!(size: 72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // BleConnector：把 esp-radio 的 BLE 控制器包装成 bt-hci 的「传输层」
    let connector = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    // ExternalController：trouble 眼里的控制器（N = 它能同时处理的命令数）
    let controller: ExternalController<_, CONNECTIONS_MAX> = ExternalController::new(connector);

    // 把整个 BLE 逻辑放进任务里：HostResources 这类大结构体放任务里最省心
    spawner.spawn(ble_task(controller).unwrap());

    loop {
        Timer::after(Duration::from_secs(30)).await;
    }
}

#[task]
async fn ble_task(controller: ExternalController<BleConnector<'static>, CONNECTIONS_MAX>) {
    // 固定随机地址：手机侧每次看到的是同一个设备（生产环境应该用芯片出厂 MAC）
    let address: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    rprintln!("本机 BLE 地址 = {:?}", address);

    // HostResources：连接槽 + 通道槽的存储，容量写进类型里（编译期就能发现不够）
    let mut resources: HostResources<_, DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        .build();

    let mut runner = stack.runner();             // 搬运 controller 事件
    let mut peripheral = stack.peripheral();     // 外设角色（广播/接受连接）

    // GapConfig::Peripheral 里的 name/appearance 决定「手机配对界面」看到的样子
    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "TrouBLE",
        appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
    }))
    .unwrap();

    // join：runner 必须一直跑，广播/服务逻辑同时进行
    let _ = join(runner.run(), async {
        loop {
            match advertise("TrouBLE", &mut peripheral, &server).await {
                Ok(conn) => {
                    // 连上后：事件处理 + 主动通知并行，任一结束（通常是断开）就回去重新广播
                    let events = gatt_events_task(&server, &conn);
                    let notifier = notifier_task(&server, &conn, &stack);
                    select(events, notifier).await;
                    rprintln!("连接已结束，回到广播状态");
                }
                Err(e) => rprintln!("广播出错：{:?}", e),
            }
        }
    })
    .await;
}

/// 组广播数据（≤31 字节）并等待连接。
async fn advertise<'values, 'server, C: Controller>(
    name: &'values str,
    peripheral: &mut Peripheral<'values, C, DefaultPacketPool>,
    server: &'server Server<'values>,
) -> Result<GattConnection<'values, 'server, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut adv_data = [0; 31];
    // AdStructure::encode_slice 把「TLV 结构」拼成字节流，返回实际长度
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),  // 可被发现、不支持经典蓝牙
            AdStructure::IncompleteServiceUuids16(&[[0x0f, 0x18]]),               // 0x180F 电池服务
            AdStructure::CompleteLocalName(name.as_bytes()),                      // 设备名
        ],
        &mut adv_data[..],
    )?;

    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {   // 可连接 + 可扫描
                adv_data: &adv_data[..len],
                scan_data: &[],
            },
        )
        .await?;
    rprintln!("开始广播（手机 App 里找 {}）……", name);

    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    rprintln!("有中心设备连上了");
    Ok(conn)
}

/// 处理 GATT 读写事件，直到断开。
async fn gatt_events_task<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
) -> Result<(), Error> {
    let level = server.battery_service.level;
    let status_handle = server.battery_service.status.handle;
    let mut status = false;

    let reason = loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => break reason,
            GattConnectionEvent::Gatt { event } => {
                let reply = match event {
                    GattEvent::Read(event) => {
                        if event.handle() == level.handle {
                            rprintln!("[gatt] 读电量 → {:?}", conn.get(&level));
                            event.accept()
                        } else if event.handle() == status_handle {
                            event.accept_unprocessed(&status)
                        } else {
                            event.accept()
                        }
                    }
                    GattEvent::Write(event) => {
                        if event.handle() == level.handle {
                            event.with_data(|offset, data| {
                                rprintln!("[gatt] 写电量（偏移 {}）：{:?}", offset, data)
                            });
                            event.accept()
                        } else if event.handle() == status_handle {
                            match event.validate(1, 1) {
                                Ok(()) => {
                                    event.with_data(|offset, data| {
                                        if data.len() == 1 {
                                            assert!(offset == 0);
                                            status = data[0] != 0;
                                            rprintln!("[gatt] status 更新为 {}", status);
                                        }
                                    });
                                    event.accept_unprocessed()
                                }
                                Err(err) => event.reject(err),
                            }
                        } else {
                            event.accept()
                        }
                    }
                    _ => event.accept(),
                };
                match reply {
                    Ok(reply) => reply.send().await,
                    Err(e) => rprintln!("[gatt] 应答失败：{:?}", e),
                };
            }
            _ => {}
        }
    };

    rprintln!("[gatt] 连接断开：{:?}", reason);
    Ok(())
}

/// 每 2 秒 notify 一次电量，并读一次 RSSI。
async fn notifier_task<C: Controller, P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    stack: &Stack<'_, C, P>,
) {
    let mut tick: u8 = 0;
    let level = server.battery_service.level;

    loop {
        tick = tick.wrapping_add(1);
        // 第 3 个参数 store=true：顺便把新值写进属性表（否则只有通知，读还是旧值）
        if level.notify(conn, &tick, true).await.is_err() {
            break;
        }
        rprintln!("[notify] 已推送电量 {}（对方要订阅了才收得到）", tick);

        if let Ok(rssi) = conn.raw().rssi(stack).await {
            rprintln!("[notify] RSSI = {} dBm", rssi);
        } else {
            break;
        }
        Timer::after(Duration::from_secs(2)).await;
    }
}
