// ============================================================================
// 51 · Wi-Fi + BLE 共存（COEX）：一根天线同时干两件事
//
// 目标：ESP32-S3 只有一套 2.4 GHz 射频，Wi-Fi 和 BLE 靠「共存调度器」分时复用。
//       本课同时跑：
//         Wi-Fi STA —— 连路由器拿 IP（数据面流量）
//         BLE 外设 —— 广播 + GATT 电池服务，连上后每 2 秒通知一次
//       观察点：两边都能正常工作，说明 coex 配置生效了。
//
// 参考：官方 examples/wifi/embassy_coex（同一套结构）
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件
// 运行：SSID=你的wifi PASSWORD=你的密码 cargo run --release --bin 51_wifi_ble_coex
//       手机上的 nRF Connect / LightBlue 搜 "TrouBLE" 设备并连接，看 Battery Level 自动变化
// 预期：RTT 里 Wi-Fi 打印 IP、BLE 打印 [gatt] 事件与通知计数，两者互不干扰
// 坑：  ① 共存要更多堆和更长的初始化时间：这里给了 64K + 64K
//       ② 必须打开 esp-radio 的 coex 特性（本项目 Cargo.toml 已开）
//       ③ 两种射频同时收发时，吞吐和延迟都会比单独用时差，这是物理限制
//       ④ 手机连上 BLE 后要订阅 0x2A19 的 notify 才能看到数值跳动
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::{Spawner, task};
use embassy_futures::join::join;                 // 让 runner 和 GATT 逻辑并行跑
use embassy_futures::select::select;             // 两个任务谁先结束就收工
use embassy_net::{Runner, StackResources};       // Wi-Fi 协议栈
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::ram;
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;    // BLE 控制器（HCI 传输层）
use esp_radio::wifi::{
    AuthenticationMethodConfig, Config, ControllerConfig, Interface, WifiController,
    sta::StationConfig,
};
use rtt_target::rprintln;
use trouble_host::prelude::*;                    // troubles 的栈、GATT、宏都在这里

macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

const CONNECTIONS_MAX: usize = 1;                // 只允许一个中心设备连接
const L2CAP_CHANNELS_MAX: usize = 2;             // Signal + ATT

// ---------------- GATT 服务定义（用宏写，编译期生成属性表） ----------------

/// 整个 GATT 数据库：这里只挂一个电池服务
#[gatt_server]
struct Server {
    battery_service: BatteryService,
}

/// 电池服务（UUID 0x180F），里面一个 Battery Level 特征（0x2A19）
#[gatt_service(uuid = service::BATTERY)]
struct BatteryService {
    /// 电量百分比：read（可读）+ notify（可主动推送），初值 10
    /// 两个 descriptor 是标准里要求的「合法范围 0..100」和「测量描述」
    #[descriptor(uuid = descriptors::VALID_RANGE, read, value = [0, 100])]
    #[descriptor(uuid = descriptors::MEASUREMENT_DESCRIPTION, name = "hello", read, value = "Battery Level", type = &'static str)]
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 10)]
    level: u8,
    /// 自定义特征（自己生成的 UUID）：可读可写可通知，用来演示「上位机写数据」
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

    // 共存要的堆明显更多：两块都开大一点
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // ---------------- Wi-Fi 侧 ----------------
    let station_config = Config::Station(
        StationConfig::default()
            .with_ssid(SSID.try_into().unwrap())
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                PASSWORD.try_into().unwrap(),
            )),
    );
    let wifi_interface = Interface::station();
    let wifi_controller = WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .unwrap();

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        embassy_net::Config::dhcpv4(Default::default()),
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    // ---------------- BLE 侧 ----------------
    // BleConnector 是「HCI 传输层」：trouble 这个 host 通过它跟控制器说话
    let connector = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let ble_controller: ExternalController<_, CONNECTIONS_MAX> = ExternalController::new(connector);

    // 三个常驻任务 + Wi-Fi 连接任务，一起跑 = 共存
    spawner.spawn(ble_task(ble_controller).unwrap());
    spawner.spawn(wifi_connection(wifi_controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());

    stack.wait_config_up().await;
    if let Some(cfg) = stack.config_v4() {
        rprintln!("Wi-Fi 就绪，IP = {}", cfg.address);
    }
    rprintln!("BLE 正在广播，设备名 TrouBLE（用手机 App 搜一下）");

    let mut n = 0u32;
    loop {
        Timer::after(Duration::from_secs(5)).await;
        n += 1;
        rprintln!("[main] 共存运行 {} 秒：Wi-Fi {}  BLE 广播中", n * 5, if stack.config_v4().is_some() { "已连接" } else { "断开" });
    }
}

/// BLE 外设任务：建栈 → 广播 → 连上后处理 GATT 事件 + 定期 notify。
///
/// 注意 HostResources 是「所有连接/通道的存储」，容量在类型参数里定死，
/// 这样放不下时在编译期就能发现，而不是运行期 OOM。
#[task]
async fn ble_task(controller: ExternalController<BleConnector<'static>, CONNECTIONS_MAX>) {
    // 固定一个随机地址，方便手机反复认同一台设备（真机产品应该用芯片 MAC）
    let address: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    rprintln!("[ble] 本机地址 = {:?}", address);

    let mut resources: HostResources<_, DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        .build();
    let mut runner = stack.runner();             // runner 负责把 controller 的事件送进 host
    let mut peripheral = stack.peripheral();

    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "TrouBLE",
        appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
    }))
    .unwrap();

    let _ = join(runner.run(), async {
        loop {
            match advertise("TrouBLE", &mut peripheral, &server).await {
                Ok(conn) => {
                    // 连上之后：GATT 事件 + 主动通知 并行跑，任意一个结束就回到广播
                    let a = gatt_events_task(&server, &conn);
                    let b = notifier_task(&server, &conn, &stack);
                    select(a, b).await;
                }
                Err(e) => rprintln!("[ble] 广播出错：{:?}", e),
            }
        }
    })
    .await;
}

/// 造广播数据并等中心设备连接。
async fn advertise<'values, 'server, C: Controller>(
    name: &'values str,
    peripheral: &mut Peripheral<'values, C, DefaultPacketPool>,
    server: &'server Server<'values>,
) -> Result<GattConnection<'values, 'server, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut adv_data = [0; 31];                  // 传统广播包最多 31 字节
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::IncompleteServiceUuids16(&[[0x0f, 0x18]]),   // 0x180F 电池服务（小端）
            AdStructure::CompleteLocalName(name.as_bytes()),
        ],
        &mut adv_data[..],
    )?;

    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &adv_data[..len],
                scan_data: &[],
            },
        )
        .await?;
    rprintln!("[ble] 开始广播，等中心设备连接……");
    // accept：挂起直到有人连上；with_attribute_server 把 GATT 表挂到这个连接上
    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    rprintln!("[ble] 已连接！");
    Ok(conn)
}

/// 处理读/写请求：每次上位机读写某个特征，这里都会收到事件。
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
                            rprintln!("[gatt] 上位机读取电量 = {:?}", conn.get(&level));
                            event.accept()
                        } else if event.handle() == status_handle {
                            event.accept_unprocessed(&status)   // 原样返回我们维护的布尔值
                        } else {
                            event.accept()                      // 其余属性按表里的值回
                        }
                    }
                    GattEvent::Write(event) => {
                        if event.handle() == level.handle {
                            event.with_data(|offset, data| {
                                rprintln!("[gatt] 上位机写入电量（偏移 {}）={:?}", offset, data)
                            });
                            event.accept()
                        } else if event.handle() == status_handle {
                            // validate(最小长度, 最大长度)：长度不对就用标准错误码回绝
                            match event.validate(1, 1) {
                                Ok(()) => {
                                    event.with_data(|offset, data| {
                                        if data.len() == 1 {
                                            assert!(offset == 0);
                                            status = data[0] != 0;
                                            rprintln!("[gatt] status 被写成 {}", status);
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
                // 必须显式 send()，否则上位机会一直等应答（drop 时也会发，但显式更清楚）
                match reply {
                    Ok(reply) => reply.send().await,
                    Err(e) => rprintln!("[gatt] 发应答失败：{:?}", e),
                };
            }
            _ => {}                              // 其它事件（加密协商等）本课不处理
        }
    };

    rprintln!("[gatt] 连接结束：{:?}", reason);
    Ok(())
}

/// 主动通知任务：每 2 秒把电量 +1 推给上位机，顺便读一次 RSSI。
async fn notifier_task<C: Controller, P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    stack: &Stack<'_, C, P>,
) {
    let mut tick: u8 = 0;
    let level = server.battery_service.level;
    loop {
        tick = tick.wrapping_add(1);
        // notify 会检查上位机是否订阅了 CCCD，没订阅就直接返回（不报错）
        if level.notify(conn, &tick, true).await.is_err() {
            rprintln!("[ble] 通知失败，连接可能已断开");
            break;
        }
        rprintln!("[ble] 已推送电量 {}", tick);
        // RSSI = 接收信号强度，排查「信号差导致丢包」时很有用
        match conn.raw().rssi(stack).await {
            Ok(rssi) => rprintln!("[ble] 当前 RSSI = {} dBm", rssi),
            Err(_) => break,
        }
        Timer::after(Duration::from_secs(2)).await;
    }
}

#[task]
async fn wifi_connection(mut controller: WifiController<'static>) {
    loop {
        match controller.connect_async().await {
            Ok(info) => {
                rprintln!("[wifi] 已连接：{:?}", info);
                let info = controller.wait_for_disconnect_async().await.ok();
                rprintln!("[wifi] 已断开：{:?}", info);
            }
            Err(e) => rprintln!("[wifi] 连接失败：{:?}", e),
        }
        Timer::after(Duration::from_millis(5000)).await;
    }
}

#[task]
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}
