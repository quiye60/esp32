// ============================================================================
// 48 · TCP / UDP 收发：两个最小回声服务器
//
// 目标：协议栈起来之后，用 socket 收发数据。本课同时跑：
//         TCP 回声服务器（1234 端口）—— 面向连接、可靠、有流量控制
//         UDP 回声服务器（1235 端口）—— 无连接、不保证送达、开销小
//       两者代码几乎一样，差别全在「要不要先 accept / 要不要记对端地址」。
//
// 参考：官方 examples/async/embassy_hello_world（TCP echo 的最简版）
//       esp-hal-radio examples/wifi/embassy_dhcp（联网骨架）
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件；需要一台和板子同网段的电脑做客户端
// 运行：SSID=你的wifi PASSWORD=你的密码 cargo run --release --bin 48_wifi_tcp_udp
// 预期：RTT 打印本机 IP；然后在这台电脑上：
//         TCP：nc <板子IP> 1234      → 输入什么就回什么
//         UDP：nc -u <板子IP> 1235   → 同样回什么（注意 nc 的 UDP 模式要按回车看回包）
// 坑：  ① 先看 RTT 里打印的 IP，别用错网段
//       ② TCP 的读返回值是 0 表示对端关了连接（不是「没数据」，那是 await 挂起）
//       ③ UDP 的缓冲区大小要 >= 单个数据报；超了会被截断（Truncated）
//       ④ 电脑防火墙可能挡住入站，测试时留意
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::{Spawner, task};
use embassy_net::{
    IpListenEndpoint, Runner, Stack, StackResources,
    tcp::TcpSocket,
    udp::{PacketMetadata, UdpSocket},
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

/// 两个服务器的端口
const TCP_PORT: u16 = 1234;
const UDP_PORT: u16 = 1235;

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

    // 连接参数：和第 45 课一样
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
        // 这里改成 5：TCP 服务器 + UDP 服务器 + 连接任务里可能用到的临时 socket
        mk_static!(StackResources<5>, StackResources::<5>::new()),
        seed,
    );

    spawner.spawn(connection(controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());

    stack.wait_config_up().await;
    let ip = stack.config_v4().map(|c| c.address).unwrap();
    rprintln!("板子 IP：{}", ip);
    rprintln!("测试命令：nc {} {}  /  nc -u {} {}", ip, TCP_PORT, ip, UDP_PORT);

    spawner.spawn(tcp_echo(stack).unwrap());
    spawner.spawn(udp_echo(stack).unwrap());

    loop {
        Timer::after(Duration::from_secs(10)).await;
    }
}

/// TCP 回声服务器：accept → 循环读 → 原样写回。
///
/// 对比 UDP：TCP 必须先 accept 建立连接，之后收发都在这个 socket 上，
/// 不需要每次记对端地址（协议栈帮你记着）。
#[task]
async fn tcp_echo(stack: Stack<'static>) {
    // socket 的收发缓冲：这里放在任务体里，生命周期跟着任务走
    let mut rx_buffer = [0u8; 1024];
    let mut tx_buffer = [0u8; 1024];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(Duration::from_secs(30)));      // 空闲 30 秒自动断开

    loop {
        rprintln!("[tcp] 等待连接……");
        // addr: None = 监听本机所有地址
        if let Err(e) = socket.accept(IpListenEndpoint { addr: None, port: TCP_PORT }).await {
            rprintln!("[tcp] accept 失败：{:?}", e);
            continue;
        }
        rprintln!("[tcp] 客户端已连接");

        let mut buf = [0u8; 512];
        loop {
            match socket.read(&mut buf).await {
                Ok(0) => {
                    rprintln!("[tcp] 对端关闭连接");
                    break;
                }
                Ok(n) => {
                    rprintln!("[tcp] 收到 {} 字节，原样回送", n);
                    use embedded_io_async::Write;
                    if socket.write_all(&buf[..n]).await.is_err() {
                        rprintln!("[tcp] 写失败，断开");
                        break;
                    }
                    let _ = socket.flush().await;
                }
                Err(e) => {
                    rprintln!("[tcp] 读失败：{:?}", e);
                    break;
                }
            }
        }
        socket.abort();                                     // 强制回到初始状态，准备 accept 下一个
    }
}

/// UDP 回声服务器：收一个数据报 → 原样发回来源地址。
///
/// UDP 没有连接概念：每次 recv_from 都会告诉你「这个包是谁发的」，
/// 你要自己把地址连同数据一起记住，回包时再交回去。
#[task]
async fn udp_echo(stack: Stack<'static>) {
    // UDP 的元数据缓冲（记录每个数据报的源地址/长度），8 个够用
    let mut rx_meta = [PacketMetadata::EMPTY; 8];
    let mut rx_buffer = [0u8; 1024];
    let mut tx_meta = [PacketMetadata::EMPTY; 8];
    let mut tx_buffer = [0u8; 1024];

    let mut socket = UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut rx_buffer,
        &mut tx_meta,
        &mut tx_buffer,
    );
    if let Err(e) = socket.bind(UDP_PORT) {
        rprintln!("[udp] bind 失败：{:?}", e);
        return;
    }
    rprintln!("[udp] 已在 {} 端口监听", UDP_PORT);

    let mut buf = [0u8; 512];
    loop {
        // 返回 (收到的字节数, 对端地址)
        match socket.recv_from(&mut buf).await {
            Ok((n, remote)) => {
                rprintln!("[udp] 来自 {} 的 {} 字节，原样回送", remote.endpoint, n);
                if let Err(e) = socket.send_to(&buf[..n], remote).await {
                    rprintln!("[udp] 发送失败：{:?}", e);
                }
            }
            Err(e) => {
                rprintln!("[udp] 接收失败：{:?}", e);
                Timer::after(Duration::from_millis(100)).await;
            }
        }
    }
}

#[task]
async fn connection(mut controller: WifiController<'static>) {
    loop {
        rprintln!("[wifi] 正在连接 {}……", SSID);
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
