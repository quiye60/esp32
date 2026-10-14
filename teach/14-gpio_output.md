# 14 · GPIO 输出：点亮 LED

> 代码：[src/bin/14_gpio_output.rs](../src/bin/14_gpio_output.rs)　运行：`cargo run --release --bin 14_gpio_output`

## 涉及的东西

| API | 说明 |
| --- | --- |
| `Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default())` | 初始电平低（灭）、推挽输出、20 mA 驱动能力 |
| `led.toggle()` / `set_high()` / `set_low()` | 翻转 / 拉高 / 拉低 |
| `led.is_set_high()` | 查询当前输出电平（只是"我们设的是什么"，不是读回引脚电压） |
| `Delay::new()` + `delay_millis/micros/nanos` | 阻塞忙等：不依赖调度器、精度稳，但纯占 CPU |
| `Timer::after(d).await` | 异步等待：期间让出 CPU，是 embassy 程序的常态 |

## 接线

```
GPIO2 ──[220Ω~1kΩ]── LED(长脚/正极) ── LED(短脚/负极) ── GND
```

不接 LED 也能跑，只是看不到灯亮，RTT 打印照样有。

## 预期

```
[阻塞] 第 1 次翻转，LED = 亮
[阻塞] 第 2 次翻转，LED = 灭
[阻塞] 第 3 次翻转，LED = 亮
[异步] 第 1 次翻转，LED = 灭
[异步] 第 2 次翻转，LED = 亮
...每秒一行
```

## 关键点

- `Output::new` 会把 `peripherals.GPIO2` **move** 走，之后就不能再取 GPIO2（见 [03-外设所有权与共享](03-外设所有权与共享.md)）。
- 想换引脚：改 `Output::new(peripherals.GPIOx, ...)` 里的 `GPIOx`；本项目的约定是 LED=GPIO2、按键=GPIO9、电位器=GPIO1。
- 从"阻塞闪 3 次"切到"异步闪"时，中间必须 `esp_rtos::start(...)`：**没有它，embassy 的 `Timer` 永远等不到时间**（会卡住不动）。
- 用 `LED = 亮/灭` 这种输出代替"看灯"：手里没接 LED、或者灯接线接反时，靠打印就能判断程序逻辑对不对。

## 延伸

- 官方对照：`reference/esp-hal-1.2.2/examples/hello_world`、`esp-hal/src/gpio/mod.rs` 的文档示例
- 下一课用输入设备做交互：GPIO 输入（按键）
