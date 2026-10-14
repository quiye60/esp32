// ============================================================================
// 56 · 802.11 嗅探与注入：直接看无线帧，并自己发一帧出去
//
// 目标：跳过 TCP/IP，直接面对 802.11 帧本身：
//         1) 开混杂模式（promiscuous），让射频把「所有」帧都交给回调
//         2) 手写解析帧头：帧控制字节 → 类型/子类型、三个 MAC 地址
//         3) 从 beacon / probe response 的「元素」里抠出 SSID（TLV 格式）
//         4) 用 send_raw_frame 注入一帧自己拼的 probe request（帧注入的最小例子）
//       为什么不用第三方库：ieee80211 这类库会把帧结构藏起来，本课手撸一遍最直观。
//
// 参考：官方 examples/wifi/sniffer（用 ieee80211 crate 的版本）
//       examples/wifi/80211_tx（注入 beacon 的版本）
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件
// 运行：cargo run --release --bin 56_wifi_sniffer
// 预期：每发现一个新 SSID 就打印一行；每 500 个包打印一次统计
//       10 秒后开始每秒注入一帧 probe request（用手机抓包软件能看到）
// 坑：  ① 回调是在 Wi-Fi 中断上下文里跑的，里面不能 await、不能分配内存
//       ② 打印放在临界区里只是为了让例子简短；真做产品应该「回调里只塞队列，
//          另开任务打印」，否则会拖慢射频甚至丢包
//       ③ 混杂模式会明显增加 CPU 负载和功耗，抓完记得关（set_promiscuous_mode(false)）
//       ④ 注入帧的合法性由你自己负责：MAC 地址别用真机的，否则可能干扰真实网络
// ============================================================================

#![no_std]
#![no_main]

use core::cell::RefCell;

use critical_section::Mutex;                     // 中断上下文里共享数据的标准做法
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::wifi::sniffer::PromiscuousPkt;    // 回调收到的包
use rtt_target::rprintln;

/// 最多记住这么多不同的 SSID
const MAX_SSIDS: usize = 16;

/// 嗅探统计 + 已发现的 SSID 列表（回调在中断里改它，所以要用临界区保护）
struct SnifferState {
    total: u32,
    mgmt: u32,
    data: u32,
    ctrl: u32,
    ssid_count: usize,
    ssids: [[u8; 32]; MAX_SSIDS],
    ssid_lens: [usize; MAX_SSIDS],
}

static STATE: Mutex<RefCell<SnifferState>> = Mutex::new(RefCell::new(SnifferState {
    total: 0,
    mgmt: 0,
    data: 0,
    ctrl: 0,
    ssid_count: 0,
    ssids: [[0; 32]; MAX_SSIDS],
    ssid_lens: [0; MAX_SSIDS],
}));

/// 收到一个包时的回调（**注意**：运行在 Wi-Fi 中断上下文，不能阻塞）
fn sniffer_cb(pkt: PromiscuousPkt<'_>) {
    let data = pkt.data;
    if data.len() < 24 {
        return;                                  // 帧头都不全，直接丢
    }

    // ---- 帧控制字段（前 2 字节）----
    // 第 1 字节：低 2 位 = 协议版本，接着 2 位 = 类型，高 4 位 = 子类型
    let frame_type = (data[0] >> 2) & 0b11;
    let subtype = (data[0] >> 4) & 0b1111;

    // ---- 三个地址（管理帧里 1=目的 2=源 3=BSSID）----
    let addr1 = &data[4..10];
    let addr2 = &data[10..16];

    critical_section::with(|cs| {
        let mut st = STATE.borrow_ref_mut(cs);
        st.total += 1;

        match frame_type {
            0 => st.mgmt += 1,
            1 => st.ctrl += 1,
            2 => st.data += 1,
            _ => {}
        }

        // beacon(8) / probe response(5) 里带 SSID
        if frame_type == 0 && (subtype == 8 || subtype == 5) {
            // 管理帧头 24 字节 + 固定参数 12 字节（时间戳 8 + beacon 间隔 2 + 能力 2）
            let mut pos = 36;
            while pos + 2 <= data.len() {
                let elem_id = data[pos];
                let elem_len = data[pos + 1] as usize;
                if pos + 2 + elem_len > data.len() {
                    break;                       // 元素被截断
                }
                if elem_id == 0 {
                    // SSID 元素：id=0，后面是名字
                    let name = &data[pos + 2..pos + 2 + elem_len];
                    let stored_len = name.len().min(32);

                    let known = st
                        .ssids
                        .iter()
                        .take(st.ssid_count)
                        .any(|s| s[..stored_len] == name[..stored_len]);

                    if !known && st.ssid_count < MAX_SSIDS && stored_len > 0 {
                        // 先取出下标，避免同时可变/不可变借用 st
                        let idx = st.ssid_count;
                        st.ssids[idx][..stored_len].copy_from_slice(name);
                        st.ssid_lens[idx] = stored_len;
                        st.ssid_count += 1;

                        // RTT 打印（临界区里打印只为示例简短，见文件头「坑 ②」）
                        if let Ok(text) = core::str::from_utf8(name) {
                            rprintln!(
                                "发现新 AP：SSID = {:?}  信道 {}  RSSI {} dBm  源 {:02X?}",
                                text, pkt.rx_cntl.channel, pkt.rx_cntl.rssi, addr2
                            );
                        } else {
                            rprintln!("发现新 AP（含非 UTF-8 字节）：{:02X?}", name);
                        }
                    }
                    break;
                }
                pos += 2 + elem_len;             // 跳到下一个元素
            }
        }

        // 每 500 个包报一次统计，避免刷屏
        if st.total % 500 == 0 {
            rprintln!(
                "—— 统计：共 {} 帧（管理 {} / 数据 {} / 控制 {}），已见 {} 个 SSID ——",
                st.total,
                st.mgmt,
                st.data,
                st.ctrl,
                st.ssid_count
            );
        }
    });

    let _ = addr1;                               // 想用得上的话：addr1 是接收方 MAC
}

/// 手工拼一帧 probe request（管理帧 subtype=4），用来演示「帧注入」。
///
/// 帧结构：帧控制(2) + 时长(2) + addr1(6) + addr2(6) + addr3(6) + 序号(2) + SSID 元素
fn build_probe_request(buf: &mut [u8; 64], ssid: &[u8], src_mac: [u8; 6]) -> usize {
    let ssid_len = ssid.len().min(32);
    let total = 24 + 2 + ssid_len;               // 头 24 + SSID 元素头 2 + 名字

    buf[0] = 0b0100_0000;                        // 类型=0(管理)、子类型=4(probe request)
    buf[1] = 0x00;
    buf[2..4].copy_from_slice(&0u16.to_le_bytes());            // 时长
    buf[4..10].copy_from_slice(&[0xFF; 6]);                    // addr1 = 广播（谁都能收）
    buf[10..16].copy_from_slice(&src_mac);                     // addr2 = 自己
    buf[16..22].copy_from_slice(&[0xFF; 6]);                   // addr3 = 广播 BSSID
    buf[22..24].copy_from_slice(&0u16.to_le_bytes());          // 序号，0 即可
    buf[24] = 0;                                               // 元素 ID 0 = SSID
    buf[25] = ssid_len as u8;
    buf[26..26 + ssid_len].copy_from_slice(&ssid[..ssid_len]);

    total
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    rtt_target::rtt_init_print!();

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // 混杂模式会把数据包复制出来，堆要大一点
    esp_alloc::heap_allocator!(size: 72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // 嗅探不需要连 AP，但控制器要先初始化起来
    let controller = esp_radio::wifi::WifiController::new(peripherals.WIFI, Default::default()).unwrap();
    let mut sniffer = controller.sniffer();

    // 打开混杂模式并把回调装上（回调必须是普通函数指针，不能带捕获）
    sniffer.set_promiscuous_mode(true).unwrap();
    sniffer.set_receive_cb(sniffer_cb);
    rprintln!("混杂模式已开启，正在嗅探……（新 SSID 会即时打印）");

    // ---------------- 10 秒后演示帧注入 ----------------
    Timer::after(Duration::from_secs(10)).await;
    rprintln!("开始每秒注入一帧 probe request（SSID 写的是 esp-s3-probe）");

    let mut frame = [0u8; 64];
    // 用一个「本地试验用」的 MAC 前缀（第一个字节 bit1=1 表示本地管理地址），别冒用真机地址
    let src_mac = [0x02, 0x00, 0x00, 0x11, 0x22, 0x33];

    loop {
        let len = build_probe_request(&mut frame, b"esp-s3-probe", src_mac);
        // 参数：用 STA 接口发 / 帧内容 / 不用硬件自动序号
        match sniffer.send_raw_frame(true, &frame[..len], false) {
            Ok(()) => rprintln!("已注入 {} 字节的 probe request", len),
            Err(e) => rprintln!("注入失败：{:?}", e),
        }
        Timer::after(Duration::from_secs(1)).await;
    }
}
