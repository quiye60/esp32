# 15 · GPIO 输入：按键

> 代码：[src/bin/15_gpio_input.rs](../src/bin/15_gpio_input.rs)　运行：`cargo run --release --bin 15_gpio_input`

## 涉及的东西

| API | 说明 |
| --- | --- |
| `Input::new(peripherals.GPIO9, InputConfig::default().with_pull(Pull::Up))` | 打开**内部上拉**（省掉外部电阻），按下=低电平 |
| `button.is_low()` | 轮询读电平 |
| `button.wait_for_falling_edge().await` | 异步等"按下"（下降沿），期间不占 CPU |
| `button.wait_for_rising_edge().await` | 异步等"松开"（上升沿） |

## 接线

```
GPIO9 ──── 按键一端
           按键另一端 ──── GND
```

按键不接时引脚被内部上拉成高电平，程序会一直打印"松开"——这是正常现象。

## 预期

先 10 次轮询打印，然后进入异步等待：

```
[轮询] 按键 = 松开
...
[异步] 检测到按下
[异步] 确认按下（已去抖）
[异步] 松开
```

## 关键点：为什么"轮询"会漏事件

轮询是"每 200 ms 看一眼"。如果按键只按下 50 ms，这一眼正好没看，事件就丢了。
异步等待则是由中断驱动的：**按下瞬间**任务被唤醒，想漏都难——这就是要讲中断（第 16 课）的原因。

## 坑

- 机械按键有抖动（几毫秒内电平乱跳）。本课的去抖写法很简单：`等待边沿 → 等 50 ms → 再确认一次电平`。
- `wait_for_falling_edge()` 会覆盖这个引脚之前设置的 `listen()` 事件；同一个引脚不要同时用"手动 listen + 异步等待"。
- 引脚被 `Input::new` move 走后，主循环里不能再用 `peripherals.GPIO9`。

## 延伸

- 官方对照：`esp-hal/src/gpio/mod.rs`、`esp-hal/src/gpio/asynch.rs` 里的文档示例
- 想让按键**立刻**触发（不占主循环）：下一课用真正的中断
