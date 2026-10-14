// ============================================================================
// 55 · OTA 升级：分区表、双槽位、以及「怎么安全地切过去」
//
// 目标：OTA 的本质不是「网络下载」，而是「分区表 + 引导选择」。本课把这块讲透：
//         1) flash 里的分区表长什么样（每个分区从哪开始、多大、什么类型）
//         2) 当前是从哪个槽启动的（factory / ota_0 / ota_1）
//         3) 怎么把「下一个槽」选成启动目标（ota.activate_next_partition）
//         4) 回滚保护：新固件启动后要自己标记 VALID，否则 bootloader 会回退
//       真正的升级流程 = 从网络/串口拿到镜像 → 写进下一个槽 → 切槽 → 重启。
//
// 参考：官方 examples/ota/update（本课删掉了内嵌镜像那部分，改成更安全的演示）
//       esp-bootloader-esp-idf 的 partitions / ota_updater 两个模块
//       结构照 src/bin/tasks.rs
//
// 接线：按键接 GPIO0（大多数 S3 开发板上的 BOOT 键就是它）
// 运行：cargo run --release --bin 55_ota
// 预期：打印分区表 + 当前槽位 + OTA 状态；按 BOOT 键打印「下一个槽」的信息
// 坑：  ① **默认的 espflash 分区表没有 OTA 槽**，OtaUpdater::new 会返回 Invalid。
//          想真正跑 OTA：写一份带 factory + ota_0 + ota_1 + otadata 的 partitions.csv，
//          用 `espflash flash --partition-table partitions.csv ...` 烧进去。
//       ② 切到空槽再重启会启动失败（bootloader 会尝试回滚，但仍属危险操作），
//          所以下面用 REALLY_SWITCH_PARTITION 常量把「真的切槽」关掉了。
//       ③ 自动回滚只在 bootloader 编译时开启才有；espflash 自带的 bootloader 没有。
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_bootloader_esp_idf::ota_updater::OtaUpdater;
use esp_bootloader_esp_idf::partitions::{PARTITION_TABLE_MAX_LEN, read_partition_table};
use esp_bootloader_esp_idf::ota::OtaImageState;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Pull};
use esp_hal::timer::timg::TimerGroup;
use esp_storage::FlashStorage;                 // 片内 flash 的读写入口
use rtt_target::rprintln;

/// 安全开关：false = 只打印信息，不动启动槽；确认分区表没问题后可以改成 true 试切槽
const REALLY_SWITCH_PARTITION: bool = false;

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

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // FlashStorage 直接操作片内 flash。注意：读分区表只是读，不会破坏任何东西
    let mut flash = FlashStorage::new(peripherals.FLASH);

    // 分区表就放在 flash 的固定偏移 0x8000 处，最多 0xC00 字节
    let mut buffer = [0u8; PARTITION_TABLE_MAX_LEN];
    let partition_table = match read_partition_table(&mut flash, &mut buffer) {
        Ok(pt) => pt,
        Err(e) => {
            rprintln!("读分区表失败：{:?}", e);
            loop {
                Timer::after(Duration::from_secs(60)).await;
            }
        }
    };

    rprintln!("=========== 分区表 ===========");
    for part in partition_table.iter() {
        rprintln!(
            "  {:<12} 偏移 0x{:06X} 大小 {:>7} 字节 类型 {:?}",
            part.label_as_str(),
            part.offset(),
            part.len(),
            part.partition_type()
        );
    }

    // 这次是从哪个槽启动的？bootloader 会把结果写进 otadata
    match partition_table.booted_partition() {
        Ok(Some(p)) => rprintln!("当前从分区 {} 启动（偏移 0x{:06X}）", p.label_as_str(), p.offset()),
        Ok(None) => rprintln!("当前从分区表里没有记录的槽启动（可能是 factory）"),
        Err(e) => rprintln!("读取启动分区失败：{:?}", e),
    }

    // ---------------- OTA 更新器 ----------------
    // new() 会检查「有没有 otadata 分区 + 至少两个 app 槽」，没有就直接报 Invalid
    match OtaUpdater::new(&mut flash, &mut buffer) {
        Ok(mut ota) => {
            // 当前被选中的槽
            match ota.selected_partition() {
                Ok(p) => rprintln!("otadata 里选中的槽：{:?}", p),
                Err(_) => {
                    rprintln!("otadata 还是空的，复位一次（重置为 factory）");
                    let _ = ota.reset_data();
                }
            }

            // 镜像状态：New/PendingVerify 说明 bootloader 开了自动回滚，
            // 新固件必须在启动后自己把它改成 Valid，否则下一次重启会回滚
            if let Ok(state) = ota.current_ota_state() {
                rprintln!("当前镜像状态：{:?}", state);
                if state == OtaImageState::New || state == OtaImageState::PendingVerify {
                    rprintln!("把当前镜像标记为 VALID（防止被回滚）");
                    let _ = ota.set_current_ota_state(OtaImageState::Valid);
                }
            }

            // ---------------- 按键触发「切到下一个槽」 ----------------
            let button = Input::new(peripherals.GPIO0, InputConfig::default().with_pull(Pull::Up));
            rprintln!("按 BOOT(GPIO0) 键 → 查看/切换下一个 OTA 槽");

            loop {
                if button.is_low() {
                    // 真正的升级会先把镜像写进这个区域，再切换
                    match ota.next_partition() {
                        Ok((region, subtype)) => {
                            rprintln!("下一个槽：{:?}，大小 {} 字节（写入时从该分区起点算偏移）",
                                subtype, region.partition_size());

                            if REALLY_SWITCH_PARTITION {
                                // activate_next_partition 只是改 otadata 里的「启动选择」，
                                // 重启后 bootloader 就会去读那个槽
                                match ota.activate_next_partition() {
                                    Ok(()) => rprintln!("已切换到 {:?}，重启后生效（注意：槽里要有固件！）", subtype),
                                    Err(e) => rprintln!("切换失败：{:?}", e),
                                }
                            } else {
                                rprintln!("（演示模式：没有真的切槽。把 REALLY_SWITCH_PARTITION 改成 true 才会切）");
                            }
                        }
                        Err(e) => rprintln!("取下一个槽失败：{:?}", e),
                    }

                    // 等按键松开，避免一直重复打印
                    while button.is_low() {
                        Timer::after(Duration::from_millis(20)).await;
                    }
                }
                Timer::after(Duration::from_millis(50)).await;
            }
        }
        Err(e) => {
            rprintln!("OTA 不可用：{:?}", e);
            rprintln!();
            rprintln!("原因通常是分区表里没有双 OTA 槽。这里给一份可用的 partitions.csv：");
            rprintln!("  # name,   type, subtype, offset, size");
            rprintln!("  nvs,      data, nvs,     0x9000,  0x6000");
            rprintln!("  phy_init, data, phy,     0xf000,  0x1000");
            rprintln!("  factory,  app,  factory, 0x10000, 1M");
            rprintln!("  ota_0,    app,  ota_0,   0x110000,1M");
            rprintln!("  ota_1,    app,  ota_1,   0x210000,1M");
            rprintln!("  otadata,  data, ota,     0x310000,0x2000");
            rprintln!("然后用：espflash flash --partition-table partitions.csv ...");
            loop {
                Timer::after(Duration::from_secs(60)).await;
            }
        }
    }
}
