// ============================================================================
// Wi-Fi 连接 + DHCP 例程（官方 examples/wifi/embassy_dhcp 的 rtt/probe-rs 版）
//
// 来源：esp-rs/esp-hal 仓库 tag `esp-hal-v1.2.2`
//       examples/wifi/embassy_dhcp/src/main.rs
// 改动：打印改为 RTT（rtt_target::rprintln），去掉 reqwless 的 HTTP GET 部分，
//       main 宏用 #[esp_rtos::main]（与工程里其它 bin 一致）。
// 用法：复制到 src/bin/wifi_dhcp.rs，然后
//       SSID=你的wifi PASSWORD=你的密码 cargo run --release --bin wifi_dhcp
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::{Spawner, task};
use embassy_net::{Runner, StackResources};
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::ram;
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::wifi::{
    AuthenticationMethodConfig, Config, ControllerConfig, Interface, WifiController,
    scan::ScanConfig, sta::StationConfig,
};
use rtt_target::rprintln;

esp_bootloader_esp_idf::esp_app_desc!();

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);
    loop {}
}

// 没有 make_static! 宏（那需要 nightly 的 static_cell 宏）时用的手写版本
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

// 编译期从环境变量读账号密码，避免把明文写进代码里
const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    rtt_target::rtt_init_print!();

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // Wi-Fi + embassy-net 需要堆：一块回收的 RAM + 一块普通 RAM
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // 目标 AP 和认证方式
    let station_config = Config::Station(
        StationConfig::default()
            .with_ssid(SSID.try_into().unwrap())
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                PASSWORD.try_into().unwrap(),
            )),
    );

    rprintln!("Starting wifi");
    // Interface 是 esp-radio 侧的"网卡"，station() = STA 模式
    let wifi_interface = Interface::station();
    // WifiController 是控制面：扫描、连接、断开都靠它
    let mut controller = WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .unwrap();
    rprintln!("Wifi configured and started!");

    // embassy-net：DHCP 客户端 + TCP/IP 协议栈
    let config = embassy_net::Config::dhcpv4(Default::default());

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    let (stack, runner) = embassy_net::new(
        wifi_interface,
        config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    // 先扫一遍附近的 AP（纯演示，可删）
    rprintln!("Scan");
    let scan_config = ScanConfig::default().with_max(10);
    let result = controller.scan_async(&scan_config).await.unwrap();
    for ap in result {
        rprintln!("{:?}", ap);
    }

    // 连接任务 + 网络轮询任务都必须 spawn 出去，否则永远拿不到 IP
    spawner.spawn(connection(controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());

    stack.wait_config_up().await;

    if let Some(config) = stack.config_v4() {
        rprintln!("Got IP: {}", config.address);
    }

    loop {
        Timer::after(Duration::from_secs(1)).await;
        rprintln!("wifi alive");
    }
}

#[task]
async fn connection(mut controller: WifiController<'static>) {
    rprintln!("start connection task");

    loop {
        rprintln!("About to connect...");

        match controller.connect_async().await {
            Ok(info) => {
                rprintln!("Wifi connected to {:?}", info);

                // 一直等到掉线，然后回到循环开头重连
                let info = controller.wait_for_disconnect_async().await.ok();
                rprintln!("Disconnected: {:?}", info);
            }
            // 注意：rprintln! 不支持 `{e:?}` 这种内联捕获写法，必须显式传参
            Err(e) => rprintln!("Failed to connect to wifi: {:?}", e),
        }

        Timer::after(Duration::from_millis(5000)).await
    }
}

#[task]
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}
