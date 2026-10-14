# 24 · ADC：把模拟电压读成数字

> 代码：[src/bin/24_adc.rs](../src/bin/24_adc.rs)　运行：`cargo run --release --bin 24_adc`

## 三步走

| 步骤 | 代码 | 说明 |
| --- | --- | --- |
| 建配置 | `AdcConfig::new()` | |
| 交引脚 | `config.enable_pin(pin, Attenuation::_11dB)` | 返回 `AdcPin`，读的时候要传它 |
| 建驱动 + 读 | `Adc::new(ADC1, config)` → `read_blocking(&mut pin)` | 12 位：0~4095 |

## 接线

```
电位器：两端接 3V3 / GND，中间抽头 → GPIO1（ADC1 通道 0）
没电位器：把 GPIO1 用杜邦线碰 3V3 和 GND，看读数是否接近 4095 / 0
```

## 预期

```
原始值 2048 / 4095  ≈ 1550 mV
原始值 4095 / 4095  ≈ 3100 mV   ← 碰到 3V3
原始值 3 / 4095  ≈ 2 mV         ← 碰到 GND
```

## 关键点

- **衰减决定量程**：11 dB ≈ 0~3.1 V；接 3.3 V 满量程附近读满值属正常。
- **ADC2 和 Wi-Fi 抢资源**，能用 ADC1 就用 ADC1。
- **悬空引脚读数乱跳**是正常的（高阻态拾取环境噪声）。
- 想要「直接读毫伏」：把 `enable_pin` 换成 `enable_pin_with_cal::<_, AdcCalLine<ADC1>>(...)`（走出厂校准），代码里已写好注释。
- `read_oneshot` 是**非阻塞**版（返回 `nb::Result`），本课用阻塞的 `read_blocking`。

## 延伸

- 官方对照：`qa-test/src/bin/embassy_adc.rs`（异步 + 校准版）、`qa-test/src/bin/embassy_adc_async_abstraction.rs`
- 想连续采样（DMA 搬运）：第 38 课 DMA 会展开
