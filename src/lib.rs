// ============================================================================
// esp-s3 共享基建（用户代码区，不是 esp-generate 生成的内容）
//
// 这里放所有课程都要用、且只有一份的东西：
//   - panic 处理器：RTT 打印后原地停住，方便接调试器看现场
//   - esp_app_desc!()：esp-idf bootloader 需要的固件头（少一行芯片就不启动）
//   - boot   ：芯片初始化 / 时基与调度器启动
//   - pins   ：板级引脚约定（LED / 按键 / 电位器）——要改接线只改这里
//   - prelude：每课都 import 的常用类型，一次 `use esp_s3::prelude::*;`
//
// 为什么放在 lib 里：每个 `src/bin/*.rs` 都是独立程序，各自写一份 panic
// 处理器和 esp_app_desc 太啰嗦；放进 lib 后只要课程调用了
// `esp_s3::boot::init()`，这些内容就会被链接进对应的固件。
//
// 注意：`src/bin/main.rs`（esp-generate 的生成物）不引用本 crate，
// 它自带一份 panic 处理器和 app_desc，两边互不冲突。
//
// 重跑 esp-generate 会把这个文件覆盖回只有 `#![no_std]` 的内容，
// 届时从这里把代码贴回来即可（本文件内容基本不用改）。
// ============================================================================
#![no_std]

// 固件头：bootloader 读它判断这是不是合法的 app 镜像
esp_bootloader_esp_idf::esp_app_desc!();

// panic 处理器：no_std 程序必须自己提供，全局只能有一份
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // 把 panic 信息（消息 + 位置）打到 RTT，probe-rs 那边能看到
    rtt_target::rprintln!("panic: {}", info);
    // 原地死循环：不要复位，这样调试器还能连着看寄存器/调用栈
    loop {}
}

pub mod boot;
pub mod pins;
pub mod prelude;
