// ============================================================================
// 57 · BLE 扫描器：把附近的蓝牙设备列出来（附「去重」和「回调」的写法）
//
// 目标：中心角色的第一课不是连接，而是「先看见」。本课：
//         1) 建栈时把角色取成 central()，再包一层 Scanner::new(central)
//         2) ScanConfig 控制扫描节奏：active（主动扫描，会发 probe request）、
//            phys（1M / coded）、interval/window（占空比）
//         3) **回调式收数据**：EventHandler::on_adv_reports 在广播报文到达时被调用，
//            这比「循环拉取」更贴近 BLE 的实际工作方式
//         4) 去重：同一个地址会反复出现，用 heapless::Deque 记下已见过的地址
//         （heapless = 不用堆的固定容量容器，嵌入式里很常用）
//
// 参考：trouble-host 官方 examples/apps/src/ble_scanner.rs
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件（把第 53 课的板子开着，就能在结果里看到 TrouBLE）
// 运行：cargo run --release --bin 57_ble_scanner
// 预期：不断打印 "discovered: ..." 的新设备地址；手机/手环/耳机都能被扫到
// 坑：  ① 扫描很费电：占空比越高越费电，电池设备要把 interval 调大、window 调小
//       ② 想扫到「不可连接广播」（beacon、广播式传感器）必须用主动扫描
//       ③ 回调里做的事要短：它跑在 host 的处理路径上，别在这里打印一大堆东西
//       ④ 想看设备名要解析广播数据（AdStructure），那属于第 53 课的 GATT 范畴
// ============================================================================

#![no_std]
#![no_main]

use core::cell::RefCell;

use embassy_executor::Spawner;
use embassy_futures::join::join;            // runner 和扫描逻辑并行
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use heapless::Deque;                        // 固定容量双端队列（不用堆）
use rtt_target::rprintln;
use trouble_host::prelude::*;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

/// 扫描结果接收器：每收到一份广播报文，on_adv_reports 就会被调用一次。
///
/// 用 RefCell 是因为 EventHandler 的接收方法是 &self（trouble 允许同一份
/// 回调被多处共享），而我们要在里面改「已见过的地址」这个列表。
struct Printer {
    seen: RefCell<Deque<BdAddr, 128>>,      // 最多记 128 个地址，满了就挤掉最早的
}

impl EventHandler for Printer {
    fn on_adv_reports(&self, mut it: LeAdvReportsIter<'_>) {
        let mut seen = self.seen.borrow_mut();

        // it 里装着这一批的所有广播报告，逐条处理
        while let Some(Ok(report)) = it.next() {
            // 同一个设备会反复广播，只打印第一次见到的
            let already = seen.iter().any(|addr| addr.raw() == report.addr.raw());
            if !already {
                rprintln!("discovered: {:?}", report.addr);

                if seen.is_full() {
                    seen.pop_front();           // 挤掉最旧的，腾出位置
                }
                let _ = seen.push_back(report.addr);
            }
        }
    }
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    rtt_target::rtt_init_print!();

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    esp_alloc::heap_allocator!(size: 72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    let connector = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let controller: ExternalController<_, 1> = ExternalController::new(connector);

    // ---------------- 1. 建栈，取中心角色 ----------------
    let mut resources: HostResources<_, DefaultPacketPool, 1, 1> = HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(Address::random([0xff, 0x8f, 0x1b, 0x05, 0xe4, 0xff]))
        .build();

    let central = stack.central();
    let mut runner = stack.runner();         // 搬运控制器事件
    let mut scanner = Scanner::new(central);

    let printer = Printer {
        seen: RefCell::new(Deque::new()),
    };

    rprintln!("开始扫描（主动扫描，1M PHY，每秒扫 1 秒即满占空比）");

    // runner 必须一直跑；扫描会话的返回值 _session 要保持存活（drop 掉就停止扫描）
    let _ = join(runner.run_with_handler(&printer), async {
        let mut config = ScanConfig::default();
        config.active = true;                 // 主动扫描：会发 probe request 问设备名
        config.phys = PhySet::M1;             // 只用 1M PHY（兼容性最好）
        config.interval = Duration::from_secs(1);
        config.window = Duration::from_secs(1);  // 和 interval 一样 = 不停扫描（费电）

        let _session = scanner.scan(&config).await.unwrap();
        rprintln!("扫描会话已建立，等回调送来结果……");

        // 一直挂着：扫描本身在后台进行，结果通过回调送进来
        loop {
            Timer::after(Duration::from_secs(10)).await;
        }
    })
    .await;

    loop {
        Timer::after(Duration::from_secs(10)).await;
    }
}
