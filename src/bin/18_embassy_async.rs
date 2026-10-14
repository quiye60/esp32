// ============================================================================
// 18 · embassy 任务与同步原语：spawn / Signal / Channel / Mutex
//
// 目标：把「一个 while 循环干所有事」拆成多个任务：
//         button_task —— 等按键（异步边沿）→ 用 Signal 通知
//         led_task    —— 等 Signal → 翻转 LED
//         sensor_task —— 每 500 ms 造一个读数 → 累加进 Mutex + 放进 Channel
//         main        —— 从 Channel 收数据并打印（消费者）
//
// 参考：本项目 teach/04-embassy异步与任务.md、teach/09-实战2
//       官方 examples/async/embassy_multicore（多核版，见讲义附录）
//       结构照 src/bin/tasks.rs
//
// 接线：LED→GPIO2、按键→GPIO9
// 运行：cargo run --release --bin 18_embassy_async
// 预期：每 500 ms 一行「收到读数 N，累计 = 三角数」；按一次键 LED 翻一次
// 坑：  ① 任务函数返回 Result，必须 .unwrap()（embassy-executor 0.10 的写法）
//       ② 任务里别写阻塞代码（delay_millis / while 死等），会把执行器卡死
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::{task, Spawner};         // task：把 async fn 变成可 spawn 的任务
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex; // 锁的实现：关中断级别
use embassy_sync::channel::Channel;            // 队列：任务间传数据流
use embassy_sync::mutex::Mutex;                // 异步锁：保护共享状态
use embassy_sync::signal::Signal;              // 信号：只保留最新一次的通知
use embassy_time::{Duration, Timer};           // 异步定时器
use esp_hal::clock::CpuClock;                  // CPU 频率
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::timer::timg::TimerGroup;          // TIMG：给调度器提供时基
use rtt_target::rprintln;                      // RTT 打印

// Signal：只保存「最新一次」的值，适合「通知一下」的场景（这里传空元组 ()）
static TOGGLE: Signal<CriticalSectionRawMutex, ()> = Signal::new();

// Channel：带队列，容量 8 —— 生产者发太快不会丢，满了会等消费者腾位置
static SENSOR: Channel<CriticalSectionRawMutex, u32, 8> = Channel::new();

// Mutex：异步锁，保护共享状态（这里是累计读数）
static TOTAL: Mutex<CriticalSectionRawMutex, u32> = Mutex::new(0);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);
    loop {}
}

// 按键任务：只关心「按下的瞬间」，然后通知 LED 任务
// #[task] 会给这个函数生成静态任务池；调用 button_task(...) 只是「申请一个任务实例」
#[task]
async fn button_task(mut button: Input<'static>) {
    loop {
        // 挂起直到按下（下降沿）；等待期间不占 CPU
        button.wait_for_falling_edge().await;
        rprintln!("[button_task] 检测到按下 → 通知 led_task");
        TOGGLE.signal(());                     // 通知：不会阻塞，没人等就留着
    }
}

// LED 任务：收通知就翻转。它完全不知道按键的存在（这就是解耦）
#[task]
async fn led_task(mut led: Output<'static>) {
    loop {
        TOGGLE.wait().await;                   // 一直等到有人 signal
        led.toggle();                          // 翻转 LED
        rprintln!("[led_task] LED = {}", if led.is_set_high() { "亮" } else { "灭" });
    }
}

// 采集任务：模拟「每 500 ms 读到一个数据」
#[task]
async fn sensor_task() {
    let mut value: u32 = 0;
    loop {
        Timer::after(Duration::from_millis(500)).await; // 周期由 Timer 控制
        value += 1;                            // 造一个递增的「读数」

        // lock().await 拿到 &mut u32；*guard 才是里面的值，离开表达式自动解锁
        *TOTAL.lock().await += value;

        // 放进队列：队列满时在这里 await，等消费者取走（不丢数据）
        SENSOR.send(value).await;
    }
}

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    rtt_target::rtt_init_print!();             // 打开 RTT

    // 初始化芯片，拿到外设句柄
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // 先装时基：任务里要用 Timer
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // 外设句柄按需交给各个任务（所有权随任务走，main 不再持有）
    let button = Input::new(
        peripherals.GPIO9,
        InputConfig::default().with_pull(Pull::Up),
    );
    let led = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());

    // 0.10 的 spawn 要先「创建任务实例」，这一步返回 Result，所以 unwrap
    spawner.spawn(button_task(button).unwrap());
    spawner.spawn(led_task(led).unwrap());
    spawner.spawn(sensor_task().unwrap());

    // main 自己当消费者：从 Channel 取数据并打印
    let receiver = SENSOR.receiver();
    loop {
        let value = receiver.receive().await;  // 等到一条数据
        // *TOTAL.lock().await 把锁里的 u32 取出来（Copy），锁立刻释放
        rprintln!("[main] 收到读数 {}，累计 = {}", value, *TOTAL.lock().await);
    }
}
