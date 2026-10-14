// ============================================================================
// 16 · GPIO 中断：按键立刻响应，不在主循环里轮询
//
// 目标：掌握 esp-hal 的中断写法：
//         #[handler] 声明中断服务函数（ISR），Io::new + set_interrupt_handler 挂上去
//         critical_section::Mutex<RefCell<Option<..>>> 让 ISR 和主程序安全共享外设
//         AtomicU32 统计次数（ISR 里只改计数，打印留给主循环）
//
// 参考：官方 reference/esp-hal-1.2.2/examples/interrupt/gpio
//       hil-test/src/bin/gpio_custom_handler.rs；本项目 teach/03、09
//       结构照 src/bin/tasks.rs
//
// 接线：LED→GPIO2、按键→GPIO9（同第 15 课）
// 运行：cargo run --release --bin 16_gpio_interrupt
// 预期：LED 每 500 ms 闪一次；每按一次键多一行「累计 N 次」，N 严格 +1
// 坑：  ① ISR 里别做耗时操作（打印、延时）——复杂逻辑交给主循环/任务
//       ② esp-hal 1.2 起不允许在中断处理函数上再加 #[ram]（会直接编译报错）
// ============================================================================

#![no_std]
#![no_main]

use core::cell::RefCell;                       // 中断和主程序共享「可变」数据要靠它
use core::sync::atomic::{AtomicU32, Ordering}; // 原子计数：任何上下文都能安全读写

use critical_section::Mutex;                   // 关中断级别的锁（不是 embassy 的异步锁）
use embassy_executor::Spawner;                 // main 宏要求的形参类型
use embassy_time::{Duration, Timer};           // 异步定时器
use esp_hal::clock::CpuClock;                  // CPU 频率
use esp_hal::gpio::{Event, Input, InputConfig, Io, Level, Output, OutputConfig, Pull};
use esp_hal::handler;                          // 属性宏：把普通函数变成 ISR
use esp_hal::timer::timg::TimerGroup;          // TIMG：给调度器提供时基
use rtt_target::rprintln;                      // RTT 打印

// 按键句柄的「寄存处」：ISR 没有参数、拿不到 main 的局部变量，只能放全局；
// 类型是 Input<'static>，因为句柄本身就活到程序结束
static BUTTON: Mutex<RefCell<Option<Input<'static>>>> = Mutex::new(RefCell::new(None));

// 中断次数：主循环和 ISR 都能安全读写
static PRESS_COUNT: AtomicU32 = AtomicU32::new(0);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    rtt_target::rtt_init_print!();             // 打开 RTT

    // 初始化芯片，拿到外设句柄
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // GPIO 中断的统一入口：必须在初始化时把处理函数挂上，否则中断没人管
    let mut io = Io::new(peripherals.IO_MUX);
    io.set_interrupt_handler(gpio_isr);        // 传函数名（不是调用它）

    // 按键：内部上拉；LED：初始灭
    let mut button = Input::new(
        peripherals.GPIO9,
        InputConfig::default().with_pull(Pull::Up),
    );
    let mut led = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());

    // 把按键交给全局槽，并让它在下降沿（按下）产生中断。
    // critical_section::with 会短暂关中断，保证这段操作不会被 ISR 打断
    critical_section::with(|cs| {
        button.listen(Event::FallingEdge);     // 开启「下降沿触发」
        BUTTON.borrow_ref_mut(cs).replace(button); // 句柄存进全局槽（把 button move 进去）
    });

    // 时基：下面要 await Timer
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    rprintln!("按 GPIO9 上的按键，看看中断计数");

    let mut seen: u32 = 0;                     // 上次看到的中断次数

    loop {
        led.toggle();                          // 主循环专心做自己的事：闪灯

        let now = PRESS_COUNT.load(Ordering::Relaxed); // 原子读，不需要锁

        // 计数变了说明 ISR 刚跑过：打印放在这里，避免在中断里做耗时的事
        if now != seen {
            rprintln!(
                "按键中断累计 {} 次（LED 状态 = {}）",
                now,
                if led.is_set_high() { "亮" } else { "灭" }
            );
            seen = now;                        // 记住，避免重复打印
        }

        Timer::after(Duration::from_millis(500)).await;
    }
}

// 中断服务函数：加 #[handler] 后它就是 GPIO 中断的处理者
#[handler]
fn gpio_isr() {
    // ISR 里只做最轻的事：计数 +1（fetch_add 返回加之前的值）
    PRESS_COUNT.fetch_add(1, Ordering::Relaxed);

    // 进临界区拿按键句柄：这个槽被 main 和 ISR 两边共用，必须加锁
    critical_section::with(|cs| {
        if let Some(button) = BUTTON.borrow_ref_mut(cs).as_mut() {
            // is_interrupt_set() 确认这次确实是我们这个引脚引起的中断
            if button.is_interrupt_set() {
                button.clear_interrupt();      // 清标志；不清会反复触发
            }
        }
    });
}
