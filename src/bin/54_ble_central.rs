// ============================================================================
// 54 · BLE 中心（GATT 客户端）：主动去连别人、读特征、订阅通知
//
// 目标：把第 53 课的板子当从机，本课这块当主机，完整走一遍中心侧的流程：
//         扫描（可选）→ 连接 → 发现服务 → 发现特征
//         → 读特征值 → 订阅通知 → 收通知
//       和 53 课对比着看，就能理解 GATT 是「客户端-服务器」模型：
//       从机维护数据表，主机逐条去查/读/订阅。
//
// 参考：trouble-host 官方 examples/apps/src/ble_bas_central.rs
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件；最好有两块板（一块烧 53 课，一块烧本课）
// 运行：cargo run --release --bin 54_ble_central
// 预期：先打印连接过程，然后每 10 秒读一次 Battery Level，
//       同时收到对方推送的通知（"Got notification"）
// 坑：  ① 脚本里的目标地址写死成 53 课的固定地址；如果只烧了本课，会一直连不上
//       ② 只连不扫的写法最快，但真实产品应该先扫、按名字或 UUID 过滤
//       ③ 订阅（subscribe）之后要有一个任务一直 next()，否则通知会堆积
//       ④ 发现服务/特征也要发 ATT 请求，所以 GattClient 也得跑自己的 task()
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::{Spawner, task};
use embassy_futures::join::join;                 // 组合 runner / client.task / 读循环
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use rtt_target::rprintln;
use trouble_host::prelude::*;

const CONNECTIONS_MAX: usize = 1;
/// 信令 + ATT + 可能的 CoC（面向连接通道）
const L2CAP_CHANNELS_MAX: usize = 3;

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

    esp_alloc::heap_allocator!(size: 72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    let connector = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let controller: ExternalController<_, CONNECTIONS_MAX> = ExternalController::new(connector);

    spawner.spawn(central_task(controller).unwrap());

    loop {
        Timer::after(Duration::from_secs(30)).await;
    }
}

#[task]
async fn central_task(controller: ExternalController<BleConnector<'static>, CONNECTIONS_MAX>) {
    // 本机（中心）也用一个随机地址
    let address: Address = Address::random([0xff, 0x8f, 0x1b, 0x05, 0xe4, 0xff]);
    rprintln!("本机（中心）地址 = {:?}", address);

    let mut resources: HostResources<_, DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        .build();
    let mut runner = stack.runner();
    let mut central = stack.central();           // 中心角色

    // 目标 = 第 53 课那块板子用的固定随机地址（改这里就能连别的设备）
    let target: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);

    // 连接参数：filter_accept_list 让扫描只回目标设备，省电也省时间
    let config = ConnectConfig {
        connect_params: Default::default(),
        scan_config: ScanConfig {
            filter_accept_list: &[target],
            ..Default::default()
        },
    };

    rprintln!("开始扫描并连接目标 {:?}……", target);

    // runner 必须一直跑，所以和「连接逻辑」join 在一起
    let _ = join(runner.run(), async {
        let conn = central.connect(&config).await.unwrap();
        rprintln!("已连接，开始发现服务");

        // GattClient<控制器, 包池, 最多缓存几个服务>
        let client = GattClient::<_, DefaultPacketPool, 10>::new(&stack, &conn)
            .await
            .unwrap();

        // client.task() 负责收发 ATT 请求/响应，必须一直跑
        let _ = join(client.task(), async {
            // ---- 1. 按 UUID 找服务：0x180F 是标准电池服务 ----
            let services = client.services_by_uuid(&Uuid::new_short(0x180f)).await.unwrap();
            let service = services.first().unwrap().clone();
            rprintln!("找到电池服务 {:?}（句柄范围 {:?}）", service.uuid(), service.handle_range());

            // ---- 2. 在服务里找特征：0x2A19 是电池电量 ----
            let characteristic: Characteristic<u8> = client
                .characteristic_by_uuid(&service, &Uuid::new_short(0x2a19))
                .await
                .unwrap();
            rprintln!("找到电量特征");

            // ---- 3. 订阅通知：对方 notify 时我们就能收到 ----
            let mut listener = client.subscribe(&characteristic, false).await.unwrap();
            rprintln!("已订阅通知");

            let _ = join(
                // 每 10 秒主动读一次（read：请求-响应模式）
                async {
                    loop {
                        let mut data = [0u8; 1];
                        client.read_characteristic(&characteristic, &mut data[..]).await.unwrap();
                        rprintln!("主动读取电量 = {}", data[0]);
                        Timer::after(Duration::from_secs(10)).await;
                    }
                },
                // 同时等着对方推送的通知（notify：不用问，对方自己送）
                async {
                    loop {
                        let data = listener.next().await;
                        rprintln!("收到通知：{:?}（值 {}）", data.as_ref(), data.as_ref()[0]);
                    }
                },
            )
            .await;
        })
        .await;
    })
    .await;
}
