# esp-s3 教学笔记（teach/）

这个目录是**用来读的**：里面的 `.rs` 文件不参与编译（cargo 只编译 `src/`），所以你可以随便改、
随便写实验代码，`cargo build` 不会因此报错。想真正跑起来，就把对应文件复制到 `src/bin/<名字>.rs`。

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
| [src/lib.rs](../src/lib.rs) | 目前只有 `#![no_std]`；以后可以把它当"库"，方便写单元测试 |
| `target/xtensa-esp32s3-none-elf/debug/build/*/out/*_config_table.md` | 各 crate 的可配置项清单（第 07 课要用） |

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
