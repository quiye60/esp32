// ============================================================================
// 47 · Wi-Fi AP：自己做热点 + 跑一个 HTTP 服务器
//
// 目标：把角色反转过来——板子当 AP，手机/电脑当 STA：
//         1) 用 AccessPointConfig 描述热点（SSID / 密码 / 信道）
//         2) 网卡换成 Interface::access_point()
//         3) 地址不再靠 DHCP 要，而是自己指定静态 IP（网关就是自己）
//         4) 在上面跑一个 TCP 服务器，浏览器一访问就回一段 HTML
//       顺带理解「为什么 AP 还需要一个 DHCP 服务器」（见下面「坑」）。
//
// 参考：官方 examples/wifi/embassy_access_point（它用 edge-dhcp 起 DHCP 服务，
//       本项目没有引入 edge-dhcp，所以改成让客户端手配静态 IP，其余逻辑一致）
//       结构照 src/bin/tasks.rs
//
// 接线：不需要任何外部器件
// 运行：cargo run --release --bin 47_wifi_ap
// 预期：RTT 打印 AP 已启动；手机连上热点后浏览器打开 http://192.168.2.1:8080/
//       应看到 "Hello Rust! Hello esp-radio!" 的页面
// 坑：  ① 没有 DHCP 服务器：手机连上后要手动把 IPv4 设成静态，例如
//          地址 192.168.2.100 / 掩码 255.255.255.0 / 网关 192.168.2.1
//          （Android 选「高级选项 → IP 设置 → 静态」；iOS 用「配置 IP → 手动」）
//       ② 热点默认密码 8 位以上，写错长度在第 3 行就会 panic
//       ③ 手机若提示「此 Wi-Fi 无法访问互联网」，选择「仍然连接/保持连接」
// ============================================================================

#![no_std]
#![no_main]

use core::net::Ipv4Addr;
use core::str::FromStr;

use embassy_executor::{Spawner, task};                 // 任务宏
use embassy_net::{
    IpListenEndpoint, Ipv4Cidr, Runner, StackResources, StaticConfigV4, tcp::TcpSocket,
};                                                     // 静态 IP 配置 + TCP socket
use embassy_time::{Duration, Timer};                   // 异步定时器
use esp_hal::clock::CpuClock;                          // CPU 频率
use esp_hal::rng::Rng;                                 // 随机种子
use esp_hal::timer::timg::TimerGroup;                  // TIMG：时基
use esp_radio::wifi::{
    AuthenticationMethodConfig, Config, ControllerConfig, Interface, WifiController,
    ap::{AccessPointConfig, EventInfo},
};                                                     // AP 侧配置
use rtt_target::rprintln;                              // RTT 打印

macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

/// 热点名字与密码（WPA2；想开开放热点就改成 AuthenticationMethodConfig::None）
const AP_SSID: &str = "esp-s3-rust";
const AP_PASSWORD: &str = "12345678";

/// 板子自己的地址，同时充当网关。改这里要记得改上面注释里的客户端配置说明
const GW_IP: &str = "192.168.2.1";
const HTTP_PORT: u16 = 8080;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    rtt_target::rtt_init_print!();                     // 打开 RTT

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // AP 模式同样要堆：驱动 + 协议栈
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // ---------------- 1. 描述热点 ----------------
    let ap_config = Config::AccessPoint(
        AccessPointConfig::default()
            .with_ssid(AP_SSID.try_into().unwrap())
            .with_channel(1)                                    // 1/6/11 是 2.4G 互不干扰的三个信道
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                AP_PASSWORD.try_into().unwrap(),
            )),
    );

    rprintln!("正在启动热点 {}（密码 {}）", AP_SSID, AP_PASSWORD);
    let device = Interface::access_point();                     // 数据面：AP 网卡
    let controller = WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(ap_config),
    )
    .unwrap();
    rprintln!("热点已启动");

    // ---------------- 2. 静态 IP：自己就是网关 ----------------
    let gw_addr = Ipv4Addr::from_str(GW_IP).expect("网关地址写错了");
    let net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(gw_addr, 24),                    // /24 → 网段 192.168.2.0/24
        gateway: Some(gw_addr),
        dns_servers: Default::default(),                        // 没有上游 DNS，留空
    });

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    let (stack, runner) = embassy_net::new(
        device,
        net_config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    // 两个常驻任务：AP 事件（谁连上/断开）+ 协议栈轮询
    spawner.spawn(ap_events(controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());

    rprintln!("浏览器访问 http://{}:{}/", GW_IP, HTTP_PORT);

    // ---------------- 3. HTTP 服务器（足够手写的最小实现） ----------------
    let mut rx_buffer = [0; 1536];                              // 收请求
    let mut tx_buffer = [0; 1536];                              // 发响应
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(Duration::from_secs(10)));           // 半开连接别一直占着

    loop {
        rprintln!("等待客户端连接……");
        if let Err(e) = socket
            .accept(IpListenEndpoint {
                addr: None,                                     // None = 监听所有本机地址
                port: HTTP_PORT,
            })
            .await
        {
            rprintln!("accept 失败：{:?}", e);
            continue;
        }
        rprintln!("客户端已连接");

        // 读请求头，直到遇到空行（\r\n\r\n 表示头部结束）
        let mut buffer = [0u8; 1024];
        let mut pos = 0;
        loop {
            match socket.read(&mut buffer).await {
                Ok(0) => break,                                  // 对端关闭
                Ok(len) => {
                    pos += len;
                    // 用 from_utf8 校验，避免 unsafe
                    if let Ok(head) = core::str::from_utf8(&buffer[..pos]) {
                        if head.contains("\r\n\r\n") {
                            // 只打印请求行（GET / HTTP/1.1），免得刷屏
                            if let Some(line) = head.lines().next() {
                                rprintln!("收到请求：{}", line);
                            }
                            break;
                        }
                    }
                }
                Err(e) => {
                    rprintln!("读失败：{:?}", e);
                    break;
                }
            }
        }

        use embedded_io_async::Write;                            // write_all / flush 来自这个 trait
        // 注意：字节串里只能写 ASCII，中文说明放注释里
        let body = b"HTTP/1.0 200 OK\r\n\r\n\
            <html><body><h1>Hello Rust! Hello esp-radio!</h1>\
            <p>Served by ESP32-S3 (Rust + embassy) running as its own AP.</p>\
            </body></html>\r\n";
        if let Err(e) = socket.write_all(body).await {
            rprintln!("写失败：{:?}", e);
        }
        let _ = socket.flush().await;

        Timer::after(Duration::from_millis(500)).await;
        socket.close();                                          // 优雅关闭（发 FIN）
        Timer::after(Duration::from_millis(500)).await;
        socket.abort();                                          // 兜底：强制断开，回到 accept 状态
    }
}

/// 监听「有 STA 连上 / 断开」的事件。生产项目里可以在这里做白名单、亮灯提示等。
#[task]
async fn ap_events(controller: WifiController<'static>) {
    rprintln!("[ap_events] 事件任务启动");
    loop {
        match controller.wait_for_access_point_connected_event_async().await {
            Ok(EventInfo::Connected(info)) => rprintln!("[ap_events] 有设备连上：{:02X?}", info.mac),
            Ok(EventInfo::Disconnected(info)) => {
                rprintln!("[ap_events] 设备断开：{:02X?}（原因 {:?}）", info.mac, info.reason)
            }
            _ => {}
        }
        Timer::after(Duration::from_millis(100)).await;
    }
}

/// 协议栈轮询任务，必须常驻。
#[task]
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}
