# 26 · 系统信息：复位原因 / CPU / 芯片版本 / MAC

> 代码：[src/bin/26_system.rs](../src/bin/26_system.rs)　运行：`cargo run --release --bin 26_system`

> ⚠️ 原计划的「温度传感器（tsens）」课改了：ESP32-S3 在 esp-hal 1.2.2 里没有
> `tsens` 驱动模块（`peripherals.TSENS` 不存在），本课换成「芯片自身信息 + 主动复位」。

## 涉及的东西

| API | 说明 |
| --- | --- |
| `system::reset_reason()` | 这次为什么启动：上电 / 软件复位 / 看门狗 / 深睡眠唤醒…（返回 `Option`） |
| `system::Cpu::current()` | 当前跑在哪个核（0 = PRO_CPU，1 = APP_CPU） |
| `efuse::chip_revision()` | 芯片版本（排查硬件差异时要用） |
| `efuse::interface_mac_address(InterfaceMacAddress::Station)` | 出厂 MAC，全球唯一，可直接当设备 ID |
| `system::software_reset()` | 主动复位（返回类型 `!`，永不返回） |

## 预期

```
复位原因：Some(PowerOn)
当前 CPU 编号：0
芯片版本：ChipRevision { major: 0, minor: 2 }
Wi-Fi STA MAC：xx:xx:xx:xx:xx:xx
MAC 原始字节：[...]
5 秒后主动软件复位（用来验证复位原因会变成 SoftwareReset）
```

复位后 RTT 一般会自动重连，你会看到第二轮打印，这次 `复位原因` 变成 `SoftwareReset` —— 这就是「复位原因」这个 API 的用途。

## 关键点

- `reset_reason()` 返回 `Option`：有些情况读不到就是 `None`，别直接 unwrap。
- **MAC 当设备 ID** 是嵌入式里的常规做法（比随机 ID 稳定、比手写序列号省事）。
- 主动复位在**固件升级、严重错误自恢复**里很常见；配合第 36 课（深睡眠）就是低功耗设备的日常。

## 延伸

- 官方对照：`qa-test/src/bin/software_reset.rs`、`esp-hal/src/system.rs`、`esp-hal/src/efuse/mod.rs`
- 下一批会讲：LEDC/MCPWM/PCNT/RMT 等「波形类」外设
