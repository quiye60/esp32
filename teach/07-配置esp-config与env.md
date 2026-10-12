# 07 · 配置：`esp-config.toml` 与 `[env]`

## 7.1 为什么有两个配置文件

```toml
# .cargo/config.toml
# Keep compilation settings in this file and user/runtime settings in `esp-config.toml`.
include = ["esp-config.toml"]          # 把另一个文件也纳入 cargo 配置（同一套层级，等同直接写在这里）

[target.xtensa-esp32s3-none-elf]
runner = "probe-rs run --chip=esp32s3 ..."

[build]
rustflags = ["-C", "link-arg=-nostartfiles"]
target = "xtensa-esp32s3-none-elf"

[unstable]
build-std = ["alloc", "core"]
```

```toml
# .cargo/esp-config.toml
[env]                                  # ← 用户/运行期配置放这里
```

约定：**编译期设置**（target、runner、rustflags）留在 `config.toml`；
**用户配置**（esp-hal/esp-radio/esp-rtos 的编译期选项、日志过滤、SSID 之类）写在 `esp-config.toml`。
esp-config 工具也会提醒你不要往基础 `config.toml` 里塞 `[env]`。

## 7.2 `[env]` 的语法

```toml
[env]
# 简单形式：值必须是字符串（数字/布尔也要加引号）
ESP_HAL_CONFIG_USE_XTAL32K = "false"

# 已存在的环境变量要覆盖时，必须 force
DEFMT_LOG = { value = "debug", force = true }

# 相对路径（值会被当作相对于配置文件所在目录的路径）
MY_DATA_DIR = { value = "../data", relative = true }
```

生效范围：cargo 自己、rustc、build script，以及 `cargo run` 启动的**你的程序**——它们都能读到。
所以 Wi-Fi 的 SSID/密码也常放这里（注意别把密码提交进 git）。

## 7.3 有哪些键可以配？（这是最有用的部分）

`esp-config` 会把每个 crate 的可配置项生成成 markdown 表，放在构建产物的 `out/` 目录里，例如：

```bash
ls target/xtensa-esp32s3-none-elf/debug/build/*/out/*_config_table.md
#  esp_alloc_config_table.md          esp_hal_config_table.md
#  esp_radio_config_table.md          esp_rtos_config_table.md
#  esp_bootloader_esp_idf_config_table.md  esp_phy_config_table.md
```

真实例子（直接从 `esp_hal_config_table.md` / `esp_radio_config_table.md` 里挑的）：

| 键 | 默认 | 什么时候改它 |
| --- | --- | --- |
| `ESP_HAL_CONFIG_USE_XTAL32K` | `false` | 板子上焊了外部 32.768 kHz 晶振；开启后那两个晶振脚不能再当 GPIO |
| `ESP_HAL_CONFIG_PLACE_SPI_MASTER_DRIVER_IN_RAM` | `false` | SPI 时序要求高（例如高速屏），放 RAM 跑得更快、更费 RAM |
| `ESP_HAL_CONFIG_STACK_GUARD_MONITORING` | `true` | 栈溢出排查；也可以关掉省一点开销 |
| `ESP_HAL_CONFIG_ENSURE_MAIN_STACK_MINIMUM` | `8192` | 让链接器在"主栈太小"时直接报错，防止运行时玄学崩溃 |
| `ESP_RADIO_CONFIG_WIFI_MTU` | `1492` | 需要更大/更小的 Wi-Fi 帧 |
| `ESP_RADIO_CONFIG_EVENT_CHANNEL_SUBSCRIBERS` | `2` | Wi-Fi 事件订阅者不够会报 "Unable to subscribe to events"，多任务订阅同一事件时调大 |

每个表里的行都带**说明**和**允许的取值**，遇到"某个行为能不能调"的问题，先来这里翻。

## 7.4 改完之后：clean build + 确认

esp-config 是**编译期**读这些变量的，而且 cargo 有环境变量缓存的老问题：

```bash
# 1) 改 .cargo/esp-config.toml
# 2) 必须清一次，否则可能不生效
cargo clean
# 3) 重新构建
cargo run
```

想知道"最终到底用了什么值"，看同一目录下的 `*_selected_config.md`：

```bash
cat target/xtensa-esp32s3-none-elf/debug/build/esp-hal-*/out/esp_hal_selected_config.md
```

这就是排查"我明明改了配置但没生效"的标准动作。

## 7.5 常见的几个坑

1. **值写成数字/布尔**：`ESP_HAL_CONFIG_USE_XTAL32K = false` 会报错，要写 `"false"`。
2. **忘了 force**：shell 里已经有同名变量（比如你 export 过 `DEFMT_LOG`）时，不加 `force = true` 会报错。
3. **忘了 clean**：改完没 clean，行为还是旧的。
4. **键名拼错**：拼错不会报错，只是静静地不生效——所以要看 `*_selected_config.md` 确认。

→ 下一课开始动手写代码。
