# 25 · 随机数：RNG

> 代码：[src/bin/25_rng.rs](../src/bin/25_rng.rs)　运行：`cargo run --release --bin 25_rng`

> ⚠️ 原计划的「触摸传感器」课改了：ESP32-S3 在 esp-hal 1.2.2 里**没有**
> `touch` 驱动模块（`peripherals.TOUCH` 都不存在），本课换成同样实用的随机数。

## 涉及的东西

| API | 说明 |
| --- | --- |
| `Rng::new()` | 建随机数驱动（不需要传外设） |
| `rng.random()` | 返回一个 `u32` |
| `(rng.random() as u64) << 32 \| rng.random() as u64` | 拼出 64 位种子 —— 网络协议栈初始化就需要它 |

## 两个典型用途

1. **业务随机**：随机延时、随机闪烁、随机选任务（本课演示 `200 + value % 1000` 毫秒）。
2. **协议栈种子**：`esp-radio` / `embassy-net` 初始化时都要一个 seed，官方 Wi-Fi 例程就是这么写的。

## 预期

```
给网络栈用的 64 位随机种子：0x...
第 1 个随机数：...（0x...）
   这次等 734 ms
第 2 个随机数：...（0x...）
...
```

## 关键点

- `Rng` 是**伪随机**：种子来自硬件噪声，序列本身可复现，普通业务足够。
- 要**真随机**（密码学用途）用 `esp_hal::rng::Trng`：熵源是 RNG 外设 + ADC1 的噪声，代价是初始化更慢、更耗电。
- 想给每台设备一个唯一 ID：用第 26 课的 eFuse MAC 地址，别用随机数。

## 延伸

- 官方对照：`examples/wifi/embassy_dhcp` 里的 seed 生成、`esp-hal/src/rng/trng.rs`
