# esp-s3 教学笔记（teach/）

这个目录是**用来读的**：里面的 `.rs` 文件不参与编译（cargo 只编译 `src/`），所以你可以随便改、
随便写实验代码，`cargo build` 不会因此报错。想真正跑起来，就把对应文件复制到 `src/bin/<名字>.rs`。

**从第 13 课起的写法**（更贴近真实项目）：

- **讲义**放这里（`teach/NN-主题.md`），**代码**直接放 `src/bin/NN_主题.rs`，改完就能跑，不用再复制。
- 每课都是一个**自包含**文件，结构照 [src/bin/tasks.rs](../src/bin/tasks.rs) 抄：自带 `#[panic_handler]`、
  `esp_app_desc!()`、`rtt_init_print!()`，并在 `esp_hal::init(...)` 之后内联 `esp_rtos::start(...)`。
  这样单看一个文件就能看懂全流程，也避免"忘了启动调度器"这类坑。
- `src/bin/main.rs` 是 esp-generate 的生成物，**不要往里写课程代码**（重跑生成器会覆盖它）。
- `src/lib.rs` / `src/boot.rs` / `src/pins.rs` / `src/prelude.rs` 是第一版设计留下的公共基建，现在课程
  不再依赖它们（文件保留，不影响构建）。

## 前置条件

- 工具链由 `rust-toolchain.toml` 固定成 `channel = "esp"`（espup 装的 Xtensa 工具链）。构建前先 `source ~/export-esp.sh`，
  否则 `build.rs` 会直接 panic 并告诉你去装。
- 本项目用的是 **probe-rs** 烧录 + RTT 打印（见 `.cargo/config.toml` 里的 `runner`），所以必须接调试器
  （板载 USB-JTAG 或 esp-prog）。只插 USB 串口的话能烧录但**看不到任何打印**。

## 常用命令

| 命令 | 作用 |
| --- | --- |
| `cargo build` | 编译（debug，开启了 `opt-level = "s"`） |
| `cargo build --release` | 编译（release，开了 LTO，慢但更小更快） |
| `cargo run` | 编译 → 烧录 → 打开 RTT，看得到 `rprintln!` 输出 |
| `cargo run --bin led` | 跑 `src/bin/led.rs` 这个额外二进制（`src/bin/*.rs` 会被 Cargo 自动发现） |
| `cargo run --release --bin 14_gpio_output` | 跑第 14 课（新课都这么单课跑；`--bins` 会编译全部，很慢） |
| `SSID=xx PASSWORD=yy cargo build --bins` | 编译**所有**二进制时才需要：`src/bin/wifi_dhcp.rs` 在编译期读这两个变量 |
| `cargo clippy` / `cargo fmt` | 老规矩 |
| `cargo clean` | 改了 `.cargo/esp-config.toml` 的 `[env]` 之后**必须**做一次 |

## 项目文件地图

| 文件 | 说明 |
| --- | --- |
| [Cargo.toml](../Cargo.toml) | 依赖、profile、`[[bin]]` |
| [.cargo/config.toml](../.cargo/config.toml) | target、runner（probe-rs）、rustflags；并 `include` 了下一个文件 |
| [.cargo/esp-config.toml](../.cargo/esp-config.toml) | 用户配置区（`[env]`），esp-hal/esp-radio 的编译期选项写这里 |
| [build.rs](../build.rs) | 加链接脚本 `linkall.x`、检查 Xtensa 链接器、给链接错误加友好提示 |
| [rust-toolchain.toml](../rust-toolchain.toml) | 固定 `esp` 工具链 |
| [src/bin/main.rs](../src/bin/main.rs) | 程序入口（第 01 课的主角） |
| [src/bin/tasks.rs](../src/bin/tasks.rs) | **课程代码的模板**：13 课起的每个 bin 都照它的结构写 |
| `target/xtensa-esp32s3-none-elf/debug/build/*/out/*_config_table.md` | 各 crate 的可配置项清单（第 07 课要用） |
| [src/lib.rs](../src/lib.rs)、[src/boot.rs](../src/boot.rs)、[src/pins.rs](../src/pins.rs)、[src/prelude.rs](../src/prelude.rs) | 第一版设计留下的公共基建（panic 处理、初始化、引脚常量、prelude）。课程已改为自包含写法、不再依赖它们；文件保留，不影响构建 |
| [reference/](../reference) | 官方例程快照（esp-hal / esp-radio / trouble-host），只读参考，不参与本工程编译 |

## 阅读顺序

1. **01 逐行看懂 main.rs** —— 先把现有骨架每行都搞明白
2. **02 原理** —— no_std、启动链条、内存与堆（为什么必须这么写）
3. **03 外设所有权与共享** —— 句柄是 move-only 的，怎么给别的任务/中断用
4. **04 embassy 异步与任务** —— `async/.await`、executor、`spawn`
5. **05 时间、延时、超时** —— `Timer` / `Delay` / `with_timeout`
6. **06 日志与调试** —— RTT、probe-rs、panic 怎么查
7. **07 配置（esp-config.toml）** —— 用 `[env]` 调 esp-hal / esp-radio 的编译期开关
8. **08 实战 1：点灯**（可编译的完整 bin，逐行注释）
9. **09 实战 2：多任务 + 共享状态**（可编译的完整 bin，逐行注释）
10. **10 无线：Wi-Fi / BLE 骨架** —— 把 `_wifi_controller`、`_stack` 真正用起来
11. **11 排错清单** —— 常见报错/坑速查

建议节奏：01+02 读懂，然后直接把 08 复制成 `src/bin/led.rs` 跑一次——**先把"能看到打印 + 硬件有反应"这条链路跑通**，
后面所有学习都靠它验证。

## 课程地图（第 13 课起：分 6 批）

状态：✅ = 已可跑（讲义 + 代码都在）　⬜ = 待写（等上一批上板确认后再写）

| 批次 | 课号 | 主题 | 状态 |
| --- | --- | --- | --- |
| 1 · 地基 | 13 | RTT 打印、最小自包含骨架（照 tasks.rs 抄） | ✅ |
| | 14 | GPIO 输出：LED、阻塞 vs 异步等待 | ✅ |
| | 15 | GPIO 输入：按键、轮询 vs 异步边沿、去抖 | ✅ |
| | 16 | GPIO 中断：`#[handler]`、临界区共享、原子计数 | ✅ |
| | 17 | 时间 API：Instant / Delay / Rate / Timer / Ticker / with_timeout | ✅ |
| | 18 | embassy 任务：spawn、Signal、Channel、Mutex（附多核附录） | ✅ |
| 2 · 串行与模拟 | 19 | UART：阻塞收发 + 转异步 | ✅ |
| | 20 | USB-Serial-JTAG：内置 USB 当串口 | ✅ |
| | 21 | I2C：总线扫描 + 寄存器读写 | ✅ |
| | 22 | SPI：主机回环自测 | ✅ |
| | 23 | 抽象层：embedded-hal / embedded-io trait | ✅ |
| | 24 | ADC：读模拟电压（电位器） | ✅ |
| | 25 | 随机数：RNG（含网络栈 seed 用法） | ✅ |
| | 26 | 系统信息：复位原因 / CPU / 芯片版本 / MAC | ✅ |
| 3 · 定时/计数/PWM | 27 | LEDC：PWM 呼吸灯 | ✅ |
| | 28 | MCPWM：互补 PWM + 死区 | ✅ |
| | 29 | PCNT：脉冲计数 / 编码器 | ✅ |
| | 30 | RMT：发送任意波形 / WS2812 思路 | ✅ |
| | 31 | SDM：Sigma-Delta 调制 | ✅ |
| | 32 | TWAI：CAN 收发 | ✅ |
| 4 · 内存/存储/系统 | 33 | esp-alloc：堆、reclaimed RAM、碎片 | ✅ |
| | 34 | Flash 读写（esp-storage） | ✅ |
| | 35 | PSRAM 初始化（需带 PSRAM 的模组 + 改配置） | ✅ |
| | 36 | 深睡眠 / 定时唤醒 / GPIO 唤醒 | ✅ |
| | 37 | 系统（续）：看门狗、Systimer、临界区 | ✅ |
| | 38 | DMA：mem2mem 搬运 | ✅ |
| 5 · 高级与冷门（了解型） | 39 | I2S（接收 / 环形 DMA） | ✅ |
| | 40 | LCD_CAM：8080 并口屏 | ✅ |
| | 41 | SDMMC：读 SD 卡裸扇区 | ✅ |
| | 42 | USB OTG：CDC-ACM 虚拟串口 | ✅ |
| | 43 | 加密加速器：AES / SHA / TRNG | ✅ |
| | 44 | 浅睡眠（LP 核 / ETM 见下方说明） | ✅ |
| 6 · 无线 | 45 | Wi-Fi STA：连网 + DHCP + 打印 IP | ✅ |
| | 46 | Wi-Fi 扫描 | ✅ |
| | 47 | Wi-Fi AP：自己做热点 + HTTP 服务器 | ✅ |
| | 48 | TCP / UDP 收发（echo） | ✅ |
| | 49 | HTTP 客户端（reqwless） | ✅ |
| | 50 | SNTP 对时（手写实现） | ✅ |
| | 51 | Wi-Fi + BLE 共存 | ✅ |
| | 52 | ESP-NOW | ✅ |
| | 53 | BLE 外设：GATT 服务（trouble） | ✅ |
| | 54 | BLE 中心：连接 / 读特征 / 订阅通知 | ✅ |
| | 55 | OTA 升级（分区表与双槽位） | ✅ |
| | 56 | 802.11 嗅探 + 帧注入 | ✅ |
| 6+ · 追加 | 57 | BLE 扫描器（回调 + 去重） | ✅ |
| | 58 | BLE 信标（不可连接广播） | ✅ |

> `src/bin/wifi_dhcp.rs`（你从 `reference/adapted/` 复制过去的那份）不在课程编号里，属于第 45 课的前身，可以保留。

### 第 3~6 批的几处「和原课程表不一样」，都是有原因的

| 课号 | 原计划 | 实际做法 | 原因 |
| --- | --- | --- | --- |
| 39 | I2S | 保持异步 DMA 版不变 | 这课的固件在 fat LTO 下会让 Xtensa 后端崩掉（`Cannot select: PCREL_WRAPPER`），所以 `Cargo.toml` 的 release `lto` 由 `fat` 改成 `thin`（更稳，代价很小） |
| 40 | LCD_CAM + PARL_IO | 只讲 `40_lcd_i8080.rs`（LCD_CAM 的 8080 模式） | **ESP32-S3 没有 PARL_IO 外设**（C6/P4 才有），写不出来 |
| 44 | LP 核 + ETM | `44_light_sleep.rs`（浅睡眠）+ 讲义补充 LP 核/ETM | LP 核要额外的 LP 程序 crate + 改 build.rs；ETM 在 esp-hal 里 `etm_driver_supported = false`（S3 无此硬件） |
| 47 | 含 DHCP server | AP + 静态 IP（客户端手工配 IP） | DHCP 服务器需要 `edge-dhcp`/`edge-nal` 两个额外依赖，本工程没引 |

新增课都做了「编译验证」：`SSID=xx PASSWORD=yy cargo build --release --bins` 全部通过的。
各课的接线/运行/预期/坑写在每个 `src/bin/<课号>_*.rs` 的文件头注释里，照着看就行。
