# 16 · GPIO 中断：按键立刻响应

> 代码：[src/bin/16_gpio_interrupt.rs](../src/bin/16_gpio_interrupt.rs)　运行：`cargo run --release --bin 16_gpio_interrupt`

## 涉及的东西

| API | 说明 |
| --- | --- |
| `#[handler] fn gpio_isr()` | 把普通函数变成**中断服务函数**（ISR）；esp-hal 会把它挂到对应的中断向量上 |
| `Io::new(peripherals.IO_MUX)` + `io.set_interrupt_handler(gpio_isr)` | GPIO 中断的统一入口，必须在初始化时挂上 |
| `button.listen(Event::FallingEdge)` | 让这个引脚在下降沿（按下）产生中断 |
| `static BUTTON: Mutex<RefCell<Option<Input>>>` | 让 ISR 也能拿到按键句柄（ISR 没有参数、拿不到局部变量） |
| `critical_section::with(...)` | 短暂关中断的保护，保证 share 的数据不会被 ISR 打断 |
| `AtomicU32` | ISR 和主循环之间传递计数，不需要锁 |
| `button.is_interrupt_set()` / `clear_interrupt()` | 判断中断来源、清标志（**不清会反复触发**） |

## 预期

```
按 GPIO9 上的按键，看看中断计数
按键中断累计 1 次（LED 状态 = 亮）
按键中断累计 2 次（LED 状态 = 灭）
...
```

LED 每 500 ms 闪一次，按键随时按随时计数——两者互不干扰。

## 关键点：中断里只做最轻的事

本课的 ISR **只做了两件事**：计数 +1、清中断标志。打印留给了主循环（它比对计数有没有变化）。

这是刻意的示范：

- ISR 会打断任何正在跑的代码，在里面打印/延时/等锁，会让系统时序变坏，甚至卡死。
- 正确模式：**ISR 只改标志/计数/入队，复杂逻辑交给线程或任务处理**。

（官方的 `examples/interrupt/gpio` 直接在 ISR 里打印，那是为了示例短；本项目选择更接近实战的写法。）

## 坑

- esp-hal 1.2 起，**中断处理函数上不允许再加 `#[ram]`**（会报 `this attribute is not allowed on an interrupt handler controlled by esp-hal`）。需要"代码放 RAM"的场景由 esp-hal 内部处理。
- ISR 里用 `critical_section::with` 是必须的：`RefCell` 不是线程安全的，而 ISR 相当于"另一个线程"。
- 中断标志不清 → 中断反复触发，看起来像"按键按一次打印几十次"。

## 延伸

- 官方对照：`reference/esp-hal-1.2.2/examples/interrupt/gpio`、`hil-test/src/bin/gpio_custom_handler.rs`
- 概念铺垫：[03-外设所有权与共享](03-外设所有权与共享.md)（`Mutex<RefCell<Option<T>>>` 的来历）、[09-实战2-多任务与共享状态](09-实战2-多任务与共享状态.rs)
