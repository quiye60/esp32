# 10 · 无线：把 Wi-Fi / BLE 真正用起来

> 说明：这一课的 Wi-Fi 段和 BLE 段我都在本机用 `cargo build`（xtensa-esp32s3-none-elf）
> **实际编译链接验证过**，API 是照 `esp-radio 1.0.0-beta.1`、`embassy-net 0.9.1`、
> `trouble-host 0.7.0` 的真实签名写的，不是凭印象写的。
> 唯一没验证的是**真机行为**（有没有网、手机能不能连上），那要你上板子试。
> 另外要 100% 可跑的完整例子，看文末的上游示例链接。

## 10.1 现在 main.rs 里的无线部分是什么状态

```rust
let _wifi_controller = ...;                     // Wi-Fi 驱动已初始化，但没配 SSID，也没连接
let _wifi_interface  = Interface::station();    // station 模式的网络"网卡"，还没接进协议栈
let _stack = trouble_host::new(...).build();    // BLE 协议栈建好了，但没在广播，也没注册 GATT 服务
```

也就是说：**射频都开着，但什么业务都没做**。下面是把它接起来的两条路。

## 10.2 Wi-Fi：连上热点 + 跑起网络栈

```rust
use embassy_net::StackResources;                     // 网络栈资源（socket 数量等）
use esp_radio::wifi::{
    AuthenticationMethodConfig, Config as WifiConfig, ControllerConfig,
    Interface, WifiController, sta::StationConfig,
};
use static_cell::StaticCell;

// ---- 1) 建控制器（注意：drop 掉它就是关 Wi-Fi，所以必须一直持有） ----
let mut wifi = WifiController::new(peripherals.WIFI, ControllerConfig::default())
    .expect("wifi init failed");

// ---- 2) 配 SSID / 密码；再 set_config 让配置生效（这一步会启动 station） ----
//     .try_into() 把 &str 转成 Ssid / Password，超长等错误会在这里暴露
let sta_config = WifiConfig::Station(
    StationConfig::default()
        .with_ssid("你的SSID".try_into().unwrap())
        .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
            "你的密码".try_into().unwrap(),
        )),
);
wifi.set_config(&sta_config).expect("set_config failed");

// ---- 3) 连接（异步）。包一层 timeout，避免热点不在时永远卡住 ----
match embassy_time::with_timeout(embassy_time::Duration::from_secs(15), wifi.connect_async()).await {
    Ok(Ok(info)) => rprintln!("connected: {:?}", info),
    Ok(Err(e))   => rprintln!("connect failed: {:?}", e),
    Err(_)       => rprintln!("connect timeout"),
}

// ---- 4) 建网络接口 + 网络栈（embassy-net） ----
let sta = Interface::station();                      // station 接口是单例，同时只能有一个
static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();   // 3 = 最多 3 个并发 socket
let resources = RESOURCES.init(StackResources::new());

let (stack, runner) = embassy_net::new(
    sta,                                             // 网卡：Interface 实现了 embassy-net-driver 的 Driver
    embassy_net::Config::dhcpv4(Default::default()), // 用 DHCP 自动获取 IP
    resources,                                       // 协议栈资源（'static）
    0x1234_5678_9abc_def0,                           // 随机种子（协议栈/DHCP 用）
);

spawner.spawn(net_task(runner).unwrap());            // Runner 必须在后台一直跑，否则协议栈不工作
stack.wait_config_up().await;                        // 等 DHCP 拿到地址
rprintln!("ip = {:?}", stack.config_v4());           // 打印拿到的 IPv4 配置（None = 还没拿到）

// ---- 5) 网络栈的后台任务（固定写法） ----
#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, Interface>) -> ! {
    runner.run().await                               // 内部是无限循环
}

// ---- 6) 用网络：发一个 TCP 连接（举例） ----
use embassy_net::{IpAddress, IpEndpoint, tcp::TcpSocket};

let mut rx_buffer = [0u8; 1024];                     // socket 的接收缓冲（放在函数里也行，注意别太大）
let mut tx_buffer = [0u8; 1024];
let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

let remote = IpEndpoint::new(IpAddress::v4(192, 168, 1, 100), 8080);
match socket.connect(remote).await {
    Ok(()) => {
        use embedded_io_async::Write;                // 项目已依赖 embedded-io-async
        socket.write_all(b"hello from esp32s3\n").await.ok();
        socket.flush().await.ok();
    }
    Err(e) => rprintln!("tcp connect error: {:?}", e),
}
```

要点：

- **顺序很重要**：控制器 → `set_config` → `connect_async` → 建 `Interface` + `embassy_net::new` → spawn `runner` → 等 DHCP。
- `WifiController`、`Interface`、`Stack` 都是有意义的句柄，**不能随便 drop**。要么留在 main 里，
  要么 move 进一个长命任务。
- 想断线重连：用一个任务持有一份 `SendSpawner`（`spawner.make_send()`）或 `Signal`，
  在 `wait_for_event` 收到断线事件时重新 `connect_async`。上游示例（见文末）里的 `wifi_task` 就是干这个的。
- DHCP + TCP/UDP/DNS 都是 `embassy-net` 的功能，`Cargo.toml` 里对应 feature 已经开好了
  （`dhcpv4`、`medium-ethernet`、`tcp`、`udp`）；要 DNS 查询再加 `dns` feature。

## 10.3 BLE：广播 + 一个 GATT 服务

trouble-host 的 API 结构是：`Stack`（协议栈）→ `runner()`（后台跑协议栈）+ `peripheral()`（外设角色）
→ `advertise()`（广播）→ `accept()`（接受连接）→ `AttributeServer`（GATT 服务表）。

```rust
use bt_hci::controller::ExternalController;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;   // 需要把 embassy-sync = "0.8" 加进 Cargo.toml
use esp_radio::ble::controller::BleConnector;
use static_cell::StaticCell;
use trouble_host::prelude::*;

const CONNECTIONS_MAX: usize = 1;                       // 最多 1 个连接
const L2CAP_CHANNELS_MAX: usize = 3;                    // 每个连接的 L2CAP 通道数

// 控制器类型起个别名，下面 StaticCell 需要写出完整类型
type BleCtrl = ExternalController<BleConnector<'static>, 4>;   // 4 = HCI 命令缓冲个数

// 注意：这些必须是 'static（要 spawn 的任务要求 'static），所以用 StaticCell 放到静态存储里。
// 另外 HostResources 的第一个泛型参数是【控制器类型】，不是 mutex 类型。
static BLE_RESOURCES: StaticCell<
    HostResources<BleCtrl, DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX>,
> = StaticCell::new();
static BLE_STACK: StaticCell<Stack<'static, BleCtrl, DefaultPacketPool>> = StaticCell::new();
static BLE_STORAGE: StaticCell<[u8; 1]> = StaticCell::new();   // 可写特征值的存放处

// ---- 1) 控制器 + 协议栈 ----
let transport = BleConnector::new(peripherals.BT, Default::default()).unwrap();
let ble_controller: BleCtrl = ExternalController::<_, 4>::new(transport);
let ble_resources = BLE_RESOURCES.init(HostResources::new());       // &'static mut
let ble_stack = BLE_STACK.init(trouble_host::new(ble_controller, ble_resources).build());

// ---- 2) runner 必须一直跑（它是协议栈的"发动机"）；peripheral 用来发起广播 ----
let ble_runner_handle = ble_stack.runner();             // 注意：runner() 每个 Stack 只能取一次
let mut peripheral = ble_stack.peripheral();
spawner.spawn(ble_runner(ble_runner_handle).unwrap());

#[embassy_executor::task]
async fn ble_runner(mut runner: Runner<'static, BleCtrl, DefaultPacketPool>) {
    // Runner<'d, C, P>：三个泛型参数（生命周期、控制器、包池），少写一个就编译不过
    if let Err(e) = runner.run().await {
        rprintln!("ble host error: {:?}", e);          // run() 会返回错误，处理掉
    }
}

// ---- 3) 建一张 GATT 表（属性表）：这里放"电池服务"之类的自定义服务 ----
const SERVICE_UUID: Uuid = Uuid::new_long([0x00,0x00,0x10,0x00, 0xb0,0xcd,0x11,0xec, 0x87,0x1f,0xd4,0x5d, 0xdf,0x13,0x88,0x40]);
const VALUE_UUID:   Uuid = Uuid::new_long([0x00,0x00,0x10,0x01, 0xb0,0xcd,0x11,0xec, 0x87,0x1f,0xd4,0x5d, 0xdf,0x13,0x88,0x40]);

let mut table: AttributeTable<'_, NoopRawMutex, 10> = AttributeTable::new();   // 10 = 属性数量上限（编译期）
let mut svc = table.add_service(Service::new(0x1800u16));                     // 0x1800 = Generic Access
svc.add_characteristic_ro(0x2a00u16, b"esp-s3");                              // 设备名
svc.build();
table.add_service(Service::new(0x1801u16));                                   // 0x1801 = Generic Attribute（必选）

let storage = BLE_STORAGE.init([0u8; 1]);                                     // 借用必须活得比 table 长
let handle: Characteristic<u8> = table
    .add_service(Service::new(SERVICE_UUID))
    .add_characteristic(
        VALUE_UUID,
        &[CharacteristicProp::Read, CharacteristicProp::Write, CharacteristicProp::Notify],
        0,
        &mut storage[..],
    )
    .build();

let server = AttributeServer::<NoopRawMutex, DefaultPacketPool, 10, CONNECTIONS_MAX>::new(table);

// ---- 4) 组装广播数据 → 广播 → 接受连接 → 处理 GATT 事件 ----
let mut adv_data = [0u8; 31];
let adv_len = AdStructure::encode_slice(
    &[AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED)],
    &mut adv_data[..],
).unwrap();

let mut scan_data = [0u8; 31];
let scan_len = AdStructure::encode_slice(
    &[AdStructure::CompleteLocalName(b"esp-s3")],                                // 手机蓝牙列表里显示的名字
    &mut scan_data[..],
).unwrap();

loop {
    let acceptor = peripheral
        .advertise(&Default::default(), Advertisement::ConnectableScannableUndirected {
            adv_data: &adv_data[..adv_len],
            scan_data: &scan_data[..scan_len],
        })
        .await
        .unwrap();                                                                // 开始广播，等连接

    let conn = acceptor.accept().await.unwrap().with_attribute_server(&server).unwrap();
    rprintln!("BLE connected");

    loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => {
                rprintln!("BLE disconnected: {:?}", reason);
                break;                                                           // 回到广播状态
            }
            GattConnectionEvent::Gatt { event: GattEvent::Write(event) } => {
                event.accept().unwrap().send().await;                             // 确认写操作
                let value: u8 = conn.get(&handle).unwrap();                       // 读回写入的值
                rprintln!("written: {}", value);
            }
            _ => {}
        }
    }
}
```

要点：

- **`embassy-sync` 要自己加**（trouble-host 0.7 依赖 0.8 版本），因为 GATT 属性表需要一个 `RawMutex` 类型。
  这不算额外负担：Wi-Fi/BLE 应用几乎必然会用到它（`Signal`、`Channel` 都在里面）。
- **`'static` 是这段代码最容易踩的坑**：任务存储和 `spawn` 都要求 `'static`，
  所以 `HostResources` / `Stack` / 特征值缓冲都要放 `StaticCell`。模板里之所以能用普通局部变量
  `let mut resources = HostResources::new();`，是因为它只是"建好放着"，**没有把 runner 交给别的任务**。
- 别把 `HostResources` 的第一个泛型参数写成 mutex 类型——那是**控制器类型**
  （写成 `_` 的时候由编译器推断，写 static 时必须显式）。
- `CONNECTIONS_MAX` / `L2CAP_CHANNELS_MAX` / 属性表大小 N 都是**编译期常量**，决定静态内存占用；
  手机连不上或内存不够时先看这几个。
- Wi-Fi 和 BLE 一起用时很吃 RAM：模板里第二个 64 KiB 堆就是为 COEX 加的；不够就调大它。

## 10.4 上游示例（强烈建议直接看这些）

- esp-hal 官方示例（Wi-Fi/网络/BLE/外设全能查）：
  <https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.2.2/examples>
  —— 在仓库里找 `examples/src/bin/wifi_*.rs`、`embassy_*.rs`。
- trouble（BLE 协议栈）官方示例，包含 ESP32 板：
  <https://github.com/embassy-rs/trouble/tree/main/examples/esp32>

看示例时的注意点：这些仓库可能用 embassy 的旧版本，`spawner.spawn(...)` 的写法会不一样（见第 04 课 4.3）。

→ 下一课：出问题时的排查手册。
