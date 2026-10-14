// ============================================================================
// 49 · HTTP 客户端（reqwless）：一个 GET 请求要经过哪些环节
//
// 目标：把「联网」升级成「访问网址」，看清一次 HTTP GET 的完整链条：
//         URL 里的域名 → DNS 解析成 IP（embassy-net 的 DnsSocket）
//         → TCP 三次握手连上 80 端口（TcpClient）
//         → 拼请求行/请求头 → 发出去 → 读响应头 → 读响应体
//       reqwless 把后面几步封装好了，但每一层的角色都要能说清。
//
// 参考：官方 examples/wifi/embassy_coex 里的 HTTP 部分、reqwless 0.14 文档
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件；需要能上外网的 2.4 GHz Wi-Fi
// 运行：SSID=你的wifi PASSWORD=你的密码 cargo run --release --bin 49_wifi_http_client
// 预期：打印 "HTTP status: 200 OK" 和一段 JSON（httpbin.org 会把你的请求回显给你）
// 坑：  ① 本课只做 http://，不做 https://（HTTPS 要 TLS 库 + 证书，体积大，见注释）
//       ② 响应缓冲 rx_buf 必须够大，否则会报 buffer too small
//       ③ DNS 失败通常是路由器没给 DNS，或者域名打错
//       ④ 请求头里带上 Connection: close，服务器发完就关，逻辑最简单
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::{Spawner, task};
use embassy_net::{
    Runner, StackResources,
    dns::{DnsSocket, DnsQueryType},
    tcp::client::{TcpClient, TcpClientState},
};
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::ram;
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::wifi::{
    AuthenticationMethodConfig, Config, ControllerConfig, Interface, WifiController,
    sta::StationConfig,
};
use reqwless::client::HttpClient;               // HTTP 客户端
use reqwless::request::{Method, RequestBuilder}; // GET / POST ... 、以及 .headers() 所在的 trait
use rtt_target::rprintln;

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

/// 目标网址：httpbin.org/get 会把收到的请求原样回显，最适合验证链路
const URL: &str = "http://httpbin.org/get?from=esp32s3-rust";
const HOST: &str = "httpbin.org";

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

    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    let station_config = Config::Station(
        StationConfig::default()
            .with_ssid(SSID.try_into().unwrap())
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                PASSWORD.try_into().unwrap(),
            )),
    );
    let wifi_interface = Interface::station();
    let controller = WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .unwrap();

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        embassy_net::Config::dhcpv4(Default::default()),
        mk_static!(StackResources<4>, StackResources::<4>::new()),
        seed,
    );

    spawner.spawn(connection(controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());

    stack.wait_config_up().await;
    if let Some(cfg) = stack.config_v4() {
        rprintln!("拿到 IP：{}，DNS：{:?}", cfg.address, cfg.dns_servers);
    }

    // ---------------- 先单独演示一次 DNS 解析 ----------------
    match stack.dns_query(HOST, DnsQueryType::A).await {
        Ok(addrs) => rprintln!("{} 解析结果：{:?}", HOST, addrs),
        Err(e) => {
            rprintln!("DNS 失败：{:?}（检查路由器是否给了 DNS）", e);
            loop {
                Timer::after(Duration::from_secs(10)).await;
            }
        }
    }

    // ---------------- 建 HTTP 客户端 ----------------
    // TcpClient 内部存的是「连接池」：把 embassy-net 的 socket 借给 HTTP 客户端用。
    // 两个 1500 的数字是每个连接的收发缓冲大小。
    let tcp_client = TcpClient::new(
        stack,
        mk_static!(
            TcpClientState<1, 1500, 1500>,
            TcpClientState::<1, 1500, 1500>::new()
        ),
    );
    let dns_client = DnsSocket::new(stack);

    loop {
        let mut client = HttpClient::new(&tcp_client, &dns_client);

        // rx_buf 必须活到 body 读完，所以每次请求都重新开一块
        let mut rx_buf = [0u8; 4096];

        rprintln!("发起 GET {}", URL);
        // request() 这一步内部就完成了 DNS + TCP 连接
        match client.request(Method::GET, URL).await {
            Ok(builder) => {
                let mut builder = builder.headers(&[
                    ("Host", HOST),
                    ("User-Agent", "esp32s3-rust"),
                    ("Connection", "close"),        // 一次性请求，最省事
                ]);
                match builder.send(&mut rx_buf).await {
                    Ok(response) => {
                        rprintln!("HTTP 状态：{:?}", response.status);
                        // body() 是个异步读流，read_to_end 会把剩余数据都读进缓冲
                        match response.body().read_to_end().await {
                            Ok(data) => {
                                // 板子只有 RTT，用逐字节打印避免一行太长看不清
                                rprintln!("响应体共 {} 字节：", data.len());
                                if let Ok(text) = core::str::from_utf8(data) {
                                    for line in text.lines() {
                                        rprintln!("  {}", line);
                                    }
                                }
                            }
                            Err(e) => rprintln!("读响应体失败：{:?}", e),
                        }
                    }
                    Err(e) => rprintln!("发送请求失败：{:?}", e),
                }
            }
            Err(e) => rprintln!("连接/请求失败：{:?}", e),
        }

        rprintln!("—— 歇 10 秒再来一次 ——");
        Timer::after(Duration::from_secs(10)).await;
    }
}

#[task]
async fn connection(mut controller: WifiController<'static>) {
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
