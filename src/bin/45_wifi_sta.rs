// ============================================================================
// 45 · Wi-Fi STA：连路由器 + DHCP 拿 IP
//
// 目标：把「无线网卡」当成一块普通网卡来用，弄清三件事的角色分工：
//         WifiController —— 控制面：扫描、连接、断线事件
//         Interface      —— 数据面：esp-radio 侧的「网卡」，交给协议栈
//         embassy_net    —— TCP/IP 协议栈 + DHCP 客户端（拿到 IP）
//       三者关系记牢，第 46~56 课都是在这套骨架上加东西。
//
// 参考：官方 examples/wifi/embassy_dhcp（本项目 src/bin/wifi_dhcp.rs 是同源旧版）
//       结构照 src/bin/tasks.rs
//
// 接线：不需要任何外部器件，只要附近有 2.4 GHz 路由器（ESP32-S3 不支持 5 GHz）
// 运行：SSID=你的wifi PASSWORD=你的密码 cargo run --release --bin 45_wifi_sta
//       （账号密码走编译期环境变量，不写进代码；也可写进 .cargo/esp-config.toml 的 [env]）
// 预期：先列出扫描到的 AP，再打印 "Got IP: 192.168.x.x"，之后每秒一句 wifi alive
// 坑：  ① 堆必须够大：Wi-Fi + 协议栈要几十 KB，少了会在初始化时 panic
//       ② 掉线不会自动重连，要自己写重连循环（见 connection 任务）
//       ③ 认证方式写死成 WPA2；公共开放网络要改成 AuthenticationMethodConfig::None
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::{Spawner, task};         // task：把重连/网络轮询拆成独立任务
use embassy_net::{Runner, StackResources};     // Runner：协议栈的“发动机”，必须一直跑
use embassy_time::{Duration, Timer};           // 异步定时器
use esp_hal::clock::CpuClock;                  // CPU 频率
use esp_hal::ram;                              // ram 属性：把堆放到回收的 RAM 里
use esp_hal::rng::Rng;                         // 硬件随机数：协议栈要用它生成初始序列号
use esp_hal::timer::timg::TimerGroup;          // TIMG：给调度器提供时基
use esp_radio::wifi::{
    AuthenticationMethodConfig, Config, ControllerConfig, Interface, WifiController,
    scan::ScanConfig, sta::StationConfig,
};                                             // Wi-Fi 侧的全部类型
use rtt_target::rprintln;                      // RTT 打印

// 没有 nightly 的 static_cell::make_static!，用手写版本：把值放进一个 StaticCell 里
// 取出 'static 引用。协议栈要求这些缓冲区活到程序结束，生命周期写不成局部变量。
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

// 编译期读环境变量：值会直接编进固件，省得在板子上配
const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    rtt_target::rtt_init_print!();             // 打开 RTT

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // 堆：一块「回收的 RAM」（放到原本分给蓝牙/协议栈的那段）+ 一块普通 RAM
    // 这两个数字是官方例程验证过的起点，不够就把第二块调大
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // ---------------- 1. 描述「我要连哪个 AP」 ----------------
    let station_config = Config::Station(
        StationConfig::default()
            .with_ssid(SSID.try_into().unwrap())                 // SSID 最长 32 字节，超了会 panic
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                PASSWORD.try_into().unwrap(),                    // 密码 8~63 字节
            )),
    );

    rprintln!("正在启动 Wi-Fi（STA 模式，目标 SSID：{}）", SSID);
    let wifi_interface = Interface::station();                   // 数据面：网卡
    let mut controller = WifiController::new(                       // 控制面：连接/扫描都靠它
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .unwrap();

    // ---------------- 2. 起协议栈 ----------------
    // DHCP：向路由器要地址；默认配置 = 客户端模式
    let net_config = embassy_net::Config::dhcpv4(Default::default());

    // 随机种子：smoltcp 用它初始化 ISS（初始序列号），没有它 TCP 连接不安全也不好用
    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    let (stack, runner) = embassy_net::new(
        wifi_interface,
        net_config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),  // 3 = 同时最多 3 个 socket
        seed,
    );

    // ---------------- 3. 扫一眼附近有什么 AP（纯演示） ----------------
    rprintln!("扫描附近的 AP……");
    let scan_config = ScanConfig::default().with_max(10);        // 最多返回 10 个
    match controller.scan_async(&scan_config).await {
        Ok(results) => {
            for ap in results {
                // ssid 是定长字节数组，用 as_str() 转成 &str 打印更友好
                rprintln!(
                    "  {}  信道 {}  RSSI {} dBm",
                    ap.ssid.as_str(),
                    ap.channel,
                    ap.signal_strength
                );
            }
        }
        Err(e) => rprintln!("扫描失败：{:?}", e),
    }

    // ---------------- 4. 把两个“长跑任务”交给执行器 ----------------
    // 少了任何一个都拿不到 IP：连接任务负责握手，net_task 负责收发协议栈数据
    spawner.spawn(connection(controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());

    // wait_config_up 会一直等到 DHCP 拿到地址
    stack.wait_config_up().await;
    if let Some(config) = stack.config_v4() {
        rprintln!("拿到 IP：{}", config.address);
        if let Some(gw) = config.gateway {
            rprintln!("网关：{}", gw);
        }
        rprintln!("DNS：{:?}", config.dns_servers);
    }

    let mut alive = 0u32;
    loop {
        Timer::after(Duration::from_secs(5)).await;
        alive += 1;
        // stack.config_v4() 为 Some 说明 DHCP 租约还在，网络可用
        rprintln!(
            "wifi alive {} 秒，网络状态：{}",
            alive * 5,
            if stack.config_v4().is_some() { "已就绪" } else { "等 DHCP" }
        );
    }
}

/// 连接任务：连上 → 等掉线 → 隔 5 秒重连。
/// 把「连接」独立成任务的好处：主逻辑不用管掉线，网络恢复后 stack 自动又能用。
#[task]
async fn connection(mut controller: WifiController<'static>) {
    rprintln!("[connection] 连接任务启动");

    loop {
        rprintln!("[connection] 正在连接……");
        match controller.connect_async().await {
            Ok(info) => {
                rprintln!("[connection] 已连接：{:?}", info);
                // 挂起直到掉线（这里只打印，重连靠外层 loop）
                let info = controller.wait_for_disconnect_async().await.ok();
                rprintln!("[connection] 已断开：{:?}", info);
            }
            Err(e) => rprintln!("[connection] 连接失败：{:?}", e),
        }

        Timer::after(Duration::from_millis(5000)).await;
    }
}

/// 协议栈任务：embassy-net 要求一直跑 runner.run()，它负责所有收发包。
#[task]
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}
