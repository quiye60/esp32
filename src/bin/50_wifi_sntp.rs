// ============================================================================
// 50 · SNTP 对时：手写一个 NTP 客户端，把网络时间算成年月日时分秒
//
// 目标：不引第三方库，亲手实现一次「网络对时」，搞清这几件事：
//         1) NTP 报文只有 48 字节，请求里唯一的有效字段是「模式 = 3（客户端）」
//         2) 服务器回包的第 40..44 字节 = 服务器发报时刻（1970-01-01 起的秒数，
//            准确地说 NTP 纪元是 1900-01-01，所以要先减去 2208988800）
//         3) 有了 Unix 秒，怎么手工算出年月日（闰年规则 + 月份表）
//       顺便体会「UDP 请求-响应」的写法：一个 socket 发出去，再从同一 socket 收。
//
// 参考：官方 examples/wifi/embassy_sntp（它用 sntpc + jiff 两个库，本课改成手写）
//       RFC 4330（SNTP）、RFC 5905（NTP）
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件；需要能上外网的 2.4 GHz Wi-Fi
// 运行：SSID=你的wifi PASSWORD=你的密码 cargo run --release --bin 50_wifi_sntp
// 预期：打印 "UTC 时间：2026-10-14 13:45:07"（比北京时间少 8 小时）
// 坑：  ① UDP 没连上时不会报错，只会一直等；所以用了 embassy_time 的 with_timeout 兜底
//       ② 某些网络封了 123 端口（UDP），换 ntp.aliyun.com 之类的服务器试试
//       ③ 本课只取「秒」，不处理小数部分和时延补偿（真实 NTP 要算往返时延）
//       ④ 想让 RTC 也走起来：拿到 unix 秒后可用 esp-hal 的 rtc_cntl，见文末注释
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::{Spawner, task};
use embassy_net::{
    Runner, StackResources,
    dns::DnsQueryType,
    udp::{PacketMetadata, UdpSocket},
};
use embassy_time::{Duration, Instant, Timer, with_timeout};
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

/// NTP 服务器：国内可换成 ntp.aliyun.com / cn.pool.ntp.org
const NTP_SERVER: &str = "pool.ntp.org";

/// NTP 纪元（1900-01-01）与 Unix 纪元（1970-01-01）之间差 2208988800 秒
const NTP_UNIX_OFFSET: u64 = 2_208_988_800;

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
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    spawner.spawn(connection(controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());

    stack.wait_config_up().await;
    if let Some(cfg) = stack.config_v4() {
        rprintln!("拿到 IP：{}", cfg.address);
    }

    // ---------------- 1. 解析域名 ----------------
    let addr = match stack.dns_query(NTP_SERVER, DnsQueryType::A).await {
        Ok(addrs) => {
            let ip = addrs.iter().next().copied().unwrap();
            rprintln!("{} → {}", NTP_SERVER, ip);
            ip
        }
        Err(e) => {
            rprintln!("DNS 失败：{:?}", e);
            loop {
                Timer::after(Duration::from_secs(10)).await;
            }
        }
    };

    // ---------------- 2. 建 UDP socket（本地端口随便给，123 是服务器端的） ----------
    let mut rx_meta = [PacketMetadata::EMPTY; 4];
    let mut rx_buffer = [0u8; 512];
    let mut tx_meta = [PacketMetadata::EMPTY; 4];
    let mut tx_buffer = [0u8; 512];
    let mut socket = UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut rx_buffer,
        &mut tx_meta,
        &mut tx_buffer,
    );
    socket.bind(12345u16).unwrap();                 // 本地端口：1024~65535 随便挑

    let remote = (addr, 123u16);                    // NTP 服务器固定用 UDP 123

    let mut round = 0u32;
    loop {
        round += 1;

        // ---------------- 3. 构造 48 字节请求 ----------------
        // 只需要第一个字节：LI=0（无警告）、VN=4（NTPv4）、Mode=3（客户端）→ 0b00_100_011 = 0x23
        let mut request = [0u8; 48];
        request[0] = 0x23;
        // 其余字段（stratum、精度、时间戳…）在请求里都可以是 0

        let sent_at = Instant::now();
        if let Err(e) = socket.send_to(&request, remote).await {
            rprintln!("发送失败：{:?}", e);
            Timer::after(Duration::from_secs(5)).await;
            continue;
        }

        // ---------------- 4. 等响应（UDP 不会自己报错，必须加超时） ----------------
        let mut response = [0u8; 48];
        let received = with_timeout(Duration::from_secs(3), socket.recv_from(&mut response)).await;

        match received {
            // Ok(Ok(...)) = 3 秒内收到了；Ok(Err) = socket 层错误；Err = 超时
            Ok(Ok((n, from))) if n >= 48 => {
                let rtt_ms = sent_at.elapsed().as_millis();

                // 第 40..44 字节：服务器发报时刻（大端 u32，NTP 纪元秒）
                let ntp_secs = u32::from_be_bytes([response[40], response[41], response[42], response[43]]);
                let unix_secs = ntp_secs as u64 - NTP_UNIX_OFFSET;

                // 模式字段（第 1 字节的低 3 位）应该是 4 = 服务器
                rprintln!(
                    "第 {} 次对时：来自 {} 共 {} 字节，模式 {}，往返 {} ms",
                    round,
                    from.endpoint,
                    n,
                    response[0] & 0x07,
                    rtt_ms
                );
                let (y, mo, d, h, mi, s) = unix_to_utc(unix_secs);
                rprintln!("UTC 时间：{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, mo, d, h, mi, s);
                rprintln!("北京时间（UTC+8）：{:02}:{:02}:{:02}", (h + 8) % 24, mi, s);
                rprintln!("开机已运行 {} ms", Instant::now().as_millis());

                // 想把这台机器的 RTC 也校准的话，可以接着用 esp-hal 的 rtc_cntl：
                //   let rtc = esp_hal::rtc_cntl::Rtc::new(peripherals.LPWR);
                //   rtc.set_current_time(esp_hal::time::Instant::from_secs(unix_secs));
                // 本项目没有引入 RTC 时钟源配置，这里只演示「算时间」这一步。
            }
            Ok(Ok((n, _))) => rprintln!("响应太短（{} 字节），忽略", n),
            Ok(Err(e)) => rprintln!("接收错误：{:?}", e),
            Err(_) => rprintln!("第 {} 次对时超时（3 秒）", round),
        }

        Timer::after(Duration::from_secs(30)).await;    // 每 30 秒对一次，够看清过程
    }
}

/// 把 Unix 秒换算成 (年, 月, 日, 时, 分, 秒)，全用 UTC。
///
/// 纯手工算法，不依赖任何库：先按 86400 秒切成天数，然后一年一年减、一月一月减。
fn unix_to_utc(unix_secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = unix_secs / 86_400;              // 从 1970-01-01 起的整天数
    let secs_of_day = unix_secs % 86_400;

    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    // 1968 是最近的、能被 4 整除的闰年，方便写通用的闰年判断：
    // 能被 4 整除但不能被 100 整除，或者能被 400 整除 → 闰年
    const DAYS_IN_MONTH: [u64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    let mut year = 1970u64;
    let mut remaining = days;
    loop {
        let year_days = if is_leap(year) { 366 } else { 365 };
        if remaining < year_days {
            break;
        }
        remaining -= year_days;
        year += 1;
    }

    let mut month = 0usize;
    loop {
        let mut month_days = DAYS_IN_MONTH[month];
        if month == 1 && is_leap(year) {
            month_days = 29;                     // 二月闰年多一天
        }
        if remaining < month_days {
            break;
        }
        remaining -= month_days;
        month += 1;
    }

    (year, month as u64 + 1, remaining + 1, hour, minute, second)
}

/// 闰年判断（公历规则）。
fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
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
