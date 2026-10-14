// ============================================================================
// 46 · Wi-Fi 扫描：只扫不连，把附近的 AP 列出来
//
// 目标：单独练 controller.scan_async()，并搞清几个概念：
//         主动扫描 vs 被动扫描（ScanTypeConfig）
//         SSID / BSSID / 信道 / RSSI / 认证方式 各自是什么
//       本课不起协议栈、不连网，是观察无线环境最省事的工具（做 Wi-Fi 项目第一步就该跑它）。
//
// 参考：官方 examples/wifi/sniffer、esp-radio/src/wifi/scan.rs
//       结构照 src/bin/tasks.rs
//
// 接线：不需要任何外部器件
// 运行：cargo run --release --bin 46_wifi_scan
// 预期：每 10 秒扫一遍，按信号强弱列出 AP，并标出哪些是「你自己的路由器」
// 坑：  ① 只能看到 2.4 GHz 的 AP（S3 的射频不支持 5 GHz）
//       ② 扫描期间如果同时连着 AP，会造成短暂断流；所以联网项目里别频繁扫
//       ③ 堆要留够：扫描结果由驱动 malloc 出来，堆太小会失败
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;                        // main 宏要求的形参类型
use embassy_time::{Duration, Timer};                  // 循环节奏
use esp_hal::clock::CpuClock;                         // CPU 频率
use esp_hal::timer::timg::TimerGroup;                 // TIMG：给调度器提供时基
use esp_radio::wifi::{
    Config, ControllerConfig, WifiController,
    scan::{ScanConfig, ScanTypeConfig},
    sta::StationConfig,
};                                                    // 扫描配置相关类型
use rtt_target::rprintln;                             // RTT 打印

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    rtt_target::rtt_init_print!();                    // 打开 RTT

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // 扫描要维护 AP 列表，需要堆（不复用协议栈那套配置，一块就够）
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 48 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // 光扫描不需要指定目标：给一个「空」的 Station 配置即可（随便填个 SSID 占位）
    let station_config = Config::Station(StationConfig::default().with_ssid("scan-only".try_into().unwrap()));
    let mut controller = WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .unwrap();

    // 想扫的数据由三个字段描述：怎么扫 / 扫哪些 / 最多几条
    let scan_config = ScanConfig::default()
        .with_max(20)                                 // 最多返回 20 个结果
        .with_show_hidden(true)                       // 隐藏 SSID 的 AP 也要
        .with_scan_type(ScanTypeConfig::Active {
            min: esp_hal::time::Duration::from_millis(20),   // 每信道至少等 20 ms
            max: esp_hal::time::Duration::from_millis(60),   // 最多等 60 ms
        });
    // 被动扫描的写法（更省电但更慢，适合电池设备）：
    //   .with_scan_type(ScanTypeConfig::Passive(esp_hal::time::Duration::from_millis(300)))

    let mut round = 0u32;
    loop {
        round += 1;
        rprintln!("===== 第 {} 轮扫描 =====", round);

        // scan_async 在网络/射频层是异步的，等待期间 CPU 可以让给别的任务
        match controller.scan_async(&scan_config).await {
            Ok(results) => {
                for ap in results {
                    rprintln!(
                        "  SSID: {:32} 信道 {:>2} RSSI {:>4} dBm BSSID {:02X?}{}",
                        ap.ssid.as_str(),
                        ap.channel,
                        ap.signal_strength,
                        ap.bssid,
                        // 认证方式为 None 表示开放网络（不需要密码）
                        if ap.auth_method.is_none() { "  [开放]" } else { "" }
                    );
                }
            }
            Err(e) => rprintln!("扫描失败：{:?}", e),
        }

        Timer::after(Duration::from_secs(10)).await;  // 别扫太勤，射频和功耗都受不了
    }
}
