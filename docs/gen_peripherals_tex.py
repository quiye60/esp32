# -*- coding: utf-8 -*-
"""从 esp-metadata-generated 的 _generated_esp32s3.rs 提取 Peripherals 成员，填入 tex 模板。"""
import re, glob, sys

TEX = "/home/qiuye/Desktop/work/Rust/esp32/esp-s3/docs/peripherals_ESP32S3.tex"
META = glob.glob('/home/qiuye/.cargo/registry/src/*/esp-metadata-generated-0.5.3/src/_generated_esp32s3.rs')[0]

raw = open(META).read()
flat = re.sub(r'\s+', ' ', raw)

# ---------- 1. 字段列表（singletons） ----------
seg = flat[flat.find('_for_each_inner_peripheral!((singletons(') + len('_for_each_inner_peripheral!((singletons('):]
seg = seg[:flat.find('_for_each_inner_peripheral!((dma_eligible(', flat.find('_for_each_inner_peripheral!((singletons(')) ]
seg2 = '(' + seg  # 第一个成员没有自己的左括号（它是宏调用自带的那个）
fields = []
# 元数据里真正定义的外设名（用于过滤文档字符串里混进来的假匹配，如 (ADC1_CH0)）
a0 = flat.find('_for_each_inner_peripheral!((all(')
allseg0 = flat[a0:flat.find('(singletons(', a0)]
all_names = set(re.findall(r'\b([A-Z][A-Z0-9_]*)\s*<=\s*(?:virtual|[A-Z][A-Z0-9_]*)\s*\(', allseg0))
for m in re.finditer(r'\((\s*#\[[^\]]*\]\s*)?([A-Z][A-Z0-9_]*)\s*(?:\(\s*([a-z_]+)\s*\))?\)', seg2):
    if m.group(2) not in all_names:
        continue
    cfg = (m.group(1) or '').strip()
    name, tag = m.group(2), m.group(3) or ''
    if name in [f[0] for f in fields]:
        continue
    fields.append((name, tag == 'unstable', cfg))

# ---------- 2. 中断源 / 引脚警告 / DMA 映射 ----------
allseg = flat[flat.find('_for_each_inner_peripheral!((all('):flat.find('(singletons(')]
ints, warns = {}, {}
chunks = re.split(r'\(@ peri_type', allseg)[1:]
for c in chunks:
    m = re.search(r'\b([A-Z][A-Z0-9_]*)\s*<=\s*(\w+)\((.*)', c, re.S)
    if not m:
        continue
    name, rest = m.group(1), m.group(3)
    depth, k = 1, 0
    while k < len(rest) and depth > 0:
        depth += (rest[k] == '(') - (rest[k] == ')')
        k += 1
    inner = rest[:k - 1]
    g = re.findall(r'([A-Z][A-Z0-9_]+) : \{ ([a-z_0-9]+), ([a-z_0-9]+), ([a-z_0-9]+) \}', inner)
    if g:
        ints[name] = g
    li = re.findall(r'<li>([^<]+)</li>', c)
    if li and re.fullmatch(r'GPIO\d+', name):
        warns[name] = li

m = re.search(r'_for_each_inner_peripheral!\(\(dma_eligible\((.*?)\)\)\);', flat)
dma = re.findall(r'\(([A-Z][A-Z0-9_]*), (\w+), (\d+), (\w+)', '(' + m.group(1) + ')') if m else []

# ---------- 3. 手工整理的「驱动入口 / 递归下一层」+ 课程号 ----------
E = {
 'DMA_CH0': (r'dma::Channel::new(peripherals.DMA\_CH0) → .rx / .tx（ChannelRx/ChannelTx）→ DmaRxBuf/DmaTxBuf；mem2mem 用 Mem2Mem::new(DMA\_CH0, SPI2)', '38'),
 'DMA_CH1': (r'同 DMA\_CH0（另一个可用通道）', '38'),
 'DMA_CH2': (r'同 DMA\_CH0', ''),
 'DMA_CH3': (r'同 DMA\_CH0', ''),
 'DMA_CH4': (r'同 DMA\_CH0', ''),
 'SDM_CH0': (r'sdm::Channel::new(peripherals.SDM\_CH0, pin, cfg) → set\_duty / set\_pulse\_density', '31'),
 'SDM_CH1': (r'同 SDM\_CH0', ''),
 'SDM_CH2': (r'同 SDM\_CH0', ''),
 'SDM_CH3': (r'同 SDM\_CH0', ''),
 'SDM_CH4': (r'同 SDM\_CH0', ''),
 'SDM_CH5': (r'同 SDM\_CH0', ''),
 'SDM_CH6': (r'同 SDM\_CH0', ''),
 'SDM_CH7': (r'同 SDM\_CH0', ''),
 'AES': (r'Aes::new(peripherals.AES) → encrypt / decrypt（16 字节分组）；aes::dma 下另有 DMA 后端', '43'),
 'APB_CTRL': (r'APB 控制寄存器（TRNG/efuse 时序等内部使用），无用户驱动', ''),
 'APB_SARADC': (r'ADC（SAR）寄存器块，由 Adc 驱动内部使用', '24'),
 'ASSIST_DEBUG': (r'调试辅助（监视点 / 栈监视），esp-hal 内部使用', ''),
 'DMA': (r'DMA 控制器本体（通道池）；用户通过 DMA\_CHx / 外设的 DMA 版构造函数使用', '38'),
 'DS': (r'数字签名寄存器块；esp-hal 1.2.2 没有 ds 模块，无用户驱动', ''),
 'EXTMEM': (r'外部存储（PSRAM）缓存控制，内部使用', '35'),
 'GPIO': (r'GPIO 组（中断/锁定等）；现代写法直接用 peripherals.GPIOn + Input/Output', '16'),
 'GPIO_SD': (r'gpio::etm::Channels::new(peripherals.GPIO\_SD)（ETM 通道；S3 无 ETM 硬件）', ''),
 'HMAC': (r'Hmac::new(peripherals.HMAC) → init / configure(M, key\_id) / update(..) / finalize(out)（nb 风格）', ''),
 'I2C_ANA_MST': (r'模拟域 I2C 主（ADC/SENS 校准用），无用户驱动', ''),
 'I2C0': (r'I2c::new(peripherals.I2C0, cfg) → .with\_sda/.with\_scl → read/write/write\_read', '21'),
 'I2C1': (r'同 I2C0（另一个 I2C 控制器）', ''),
 'I2S0': (r'I2s::new(peripherals.I2S0, DMA\_CHx, TdmConfig) → .i2s\_tx / .i2s\_rx → .with\_bclk/ws/dout/din/mclk', '39'),
 'I2S1': (r'同 I2S0', ''),
 'INTERRUPT_CORE0': (r'核 0 的中断优先级控制器；由 esp\_hal::interrupt 内部使用（用户写 \#[handler]）', '16'),
 'INTERRUPT_CORE1': (r'核 1 的中断优先级控制器（多核时用）', '18'),
 'IO_MUX': (r'gpio::Io::new(peripherals.IO\_MUX)（老式 per-pin 接口；新代码用 peripherals.GPIOn）', '13'),
 'LCD_CAM': (r'LcdCam::new(peripherals.LCD\_CAM) → .lcd（I8080/RGB/DPI）/ .cam（摄像头 DVP）', '40'),
 'LEDC': (r'Ledc::new(peripherals.LEDC) → .timer::<LowSpeed>(Timer0..3) → .channel(Channel0..7, pin) → set\_duty / start\_duty\_fade', '27'),
 'LPWR': (r'LowPower::new(peripherals.LPWR) → set\_wakeup\_deadline + sleep\_light / sleep\_deep', '36,44'),
 'RTC_TIMER': (r'Rtc::new(peripherals.RTC\_TIMER) → .rwdt（看门狗）/ .swd / current\_time\_us / set\_current\_time\_us', '37'),
 'MCPWM0': (r'McPwm::new(peripherals.MCPWM0, clk) → .timer0..2 / .operator0..2 / .fault0..2 / .capture0..2 → with\_linked\_pins', '28'),
 'MCPWM1': (r'同 MCPWM0（第二组 MCPWM，S3 有两组）', ''),
 'PCNT': (r'Pcnt::new(peripherals.PCNT) → .unit0..3 → .channel0/1、.counter、set\_filter、resume/pause/clear', '29'),
 'PERI_BACKUP': (r'外设寄存器备份/保持（retention），内部使用', ''),
 'RMT': (r'Rmt::new(peripherals.RMT, Rate) → .channel0..7 → configure\_tx/configure\_rx → with\_pin → transmit/receive', '30'),
 'RNG': (r'Rng::new()（伪随机，无需参数）；TrngSource::new(peripherals.RNG, peripherals.ADC1) + Trng::try\_new() → read()', '25,43'),
 'RSA': (r'Rsa::new(peripherals.RSA) → start\_modular\_exponentiation / start\_modular\_multiplication（需大缓冲）', ''),
 'RTC_CNTL': (r'RTC 域控制寄存器（Rtc / sleep 的底层），用户通过 RTC\_TIMER 与 LPWR 使用', '37'),
 'LP_I2C0': (r'LP 域 I2C（低功耗域，给 ULP/LP 核使用）', ''),
 'RTC_IO': (r'RTC IO 复用/模拟控制（touch、DAC 等；S3 无 touch/DAC，故仅内部用）', ''),
 'SDHOST': (r'SdHostController::new(peripherals.SDHOST, cfg) → .slot::<0|1>(SlotConfig) → .with\_clk/cmd/data0..3 → into\_async() 实现 sdio::MmcBus', '41'),
 'SENS': (r'模拟前端寄存器块（内部温度/touch 公用）。esp-hal 1.2.2 在 S3 无 tsens 模块（soc\_has\_tsens=false），没有用户驱动', ''),
 'SENSITIVE': (r'RTC 域敏感寄存器，内部使用', ''),
 'SHA': (r'Sha::new(peripherals.SHA) → start::<Sha256>() → update(..) / finish(\&mut out)（nb 风格流式）', '43'),
 'SPI0': (r'内部 flash 专用 SPI（用户不要用）', ''),
 'SPI1': (r'PSRAM 专用 SPI（用户不要用）', ''),
 'SPI2': (r'master::Spi::new(peripherals.SPI2, cfg) → .with\_sck/mosi/miso/cs → transfer/write/read（slave 版：slave::Spi::new）', '22,38'),
 'SPI3': (r'同 SPI2', ''),
 'SYSTEM': (r'系统寄存器（时钟树/复位控制），内部使用', ''),
 'SYSTIMER': (r'SystemTimer::new(peripherals.SYSTIMER) → .alarm0/1/2（配 PeriodicTimer）；SystemTimer::unit\_value(Unit::Unit0) 读原始计数', '37,44'),
 'TIMG0': (r'TimerGroup::new(peripherals.TIMG0) → .timer0 / .timer1 / .wdt（timer0 当调度器时基）', '13,17,37'),
 'TIMG1': (r'同 TIMG0（第二个定时器组）', ''),
 'TWAI0': (r'TwaiConfiguration::new(peripherals.TWAI0, rx, tx, baud, mode).start() → transmit/receive（+ \_async）', '32'),
 'UART0': (r'Uart::new(peripherals.UART0, cfg) → .with\_tx/.with\_rx → read/write；默认被 ROM/bootloader 当日志口', '19'),
 'UART1': (r'同 UART0（本项目练手用这个）', '19,23'),
 'UART2': (r'同 UART0', ''),
 'UHCI0': (r'uart::uhci::Uhci::new(uart, peripherals.UHCI0, DMA\_CHx)（给 UART 挂 DMA 的外设）', ''),
 'USB_FS': (r'Usb::new\_fs(peripherals.USB\_FS, DP=GPIO20, DM=GPIO19) → embassy-usb Driver（CDC-ACM 等）', '42'),
 'USB_DEVICE': (r'UsbSerialJtag::new(peripherals.USB\_DEVICE) → read/write（芯片内置 USB-Serial-JTAG）', '20'),
 'WCL': (r'无线相关内部寄存器块（无用户驱动）', ''),
 'XTS_AES': (r'Flash XTS-AES 加解密（flash 加密功能使用）', ''),
 'ADC1': (r'AdcConfig::new() → enable\_pin(pin, Attenuation) → Adc::new(peripherals.ADC1, cfg) → read\_blocking/read\_oneshot', '24'),
 'ADC2': (r'同 ADC1，但与 Wi-Fi 共用射频，联网项目请用 ADC1', ''),
 'BT': (r'BleConnector::new(peripherals.BT, cfg) → trouble\_host::ExternalController::new(..) → Stack → peripheral()/central()/runner()', '51--58'),
 'CPU_CTRL': (r'system::CpuControl::new(peripherals.CPU\_CTRL) → start\_app\_core(..)（启动第二核）', '18'),
 'FLASH': (r'esp\_storage::FlashStorage::new(peripherals.FLASH) → read/erase/write；OTA 分区表与切槽也走它', '34,55'),
 'GPIO_DEDICATED': (r'gpio::dedicated::DedicatedGpio::new(peripherals.GPIO\_DEDICATED)（CPU 专用快速 GPIO）', ''),
 'PSRAM': (r'esp\_alloc::psram\_allocator!(peripherals.PSRAM, esp\_hal::psram)（把 PSRAM 挂进堆）', '35'),
 'ULP_RISCV_CORE': (r'lp\_core::UlpCore::new(peripherals.ULP\_RISCV\_CORE) → run(wakeup\_src)（需另备 LP 固件）', ''),
 'WIFI': (r'WifiController::new(peripherals.WIFI, cfg) → connect/scan；派生 esp\_now() / sniffer()；数据面 Interface::station() → embassy\_net', '45-52,56'),
 'FROM_CPU_INTR0': (r'核间软件中断；esp\_rtos::start(timg0.timer0, peripherals.FROM\_CPU\_INTR0) 用它当时基中断', '13+'),
 'FROM_CPU_INTR1': (r'同 FROM\_CPU\_INTR0（另一个可用软件中断）', ''),
 'FROM_CPU_INTR2': (r'同前', ''),
 'FROM_CPU_INTR3': (r'同前', ''),
}

# 本项目各课实际用到的引脚
PIN_USE = {
 'GPIO0': '按 BOOT 键读分区/切槽（55）；LEDC 输出示例（官方版）',
 'GPIO1': 'ADC1 通道 0，接电位器（24）',
 'GPIO2': '板载 LED（14、18、23、27、28 演示灯）；PCNT 计数源示例',
 'GPIO3': '备用（strapping 脚，慎用）',
 'GPIO4': 'RMT 波形输出（30）、TWAI 自检 TX/RX（32）',
 'GPIO5': 'MCPWM PWMA（28）、WS2812 数据线（30）',
 'GPIO6': 'MCPWM PWMB（28，互补输出）',
 'GPIO7': 'SDM 输出，接 RC 看平均电压（31）',
 'GPIO9': '按键（15、16、29，内部上拉）',
 'GPIO17': 'UART1 TX（19、23）；深/浅睡眠唤醒脚（36、44）',
 'GPIO18': 'UART1 RX（19）',
 'GPIO19': 'USB OTG DM（42）',
 'GPIO20': 'USB OTG DP（42）',
 'GPIO38': 'SD 卡 CMD（41）',
 'GPIO39': 'SD 卡 CLK（41）',
 'GPIO40': 'SD 卡 DAT0（41）',
 'GPIO45': 'LCD 背光（40 的 8080 屏示例）',
}

# ---------- 4. 生成表格 ----------
def esc(s):
    return s.replace('_', r'\_')

n_total = len(fields)
n_unstable = sum(1 for f in fields if f[1])
gpios = [f for f in fields if re.fullmatch(r'GPIO\d+', f[0])]
nongpio = [f for f in fields if not re.fullmatch(r'GPIO\d+', f[0])]

# 统计表
n_stable = n_total - n_unstable
stat_lines = [
 r'\begin{center}', r'\small',
 r'\begin{tabular}{@{}lr@{}}', r'\toprule',
 r'\tblhead{统计项} & \tblhead{数量} \\', r'\midrule',
 f'\\fld{{Peripherals}} 字段总数 & {n_total} \\\\',
 f'其中 GPIO 引脚（GPIO0--21、GPIO26--48） & {len(gpios)} \\\\',
 f'其它外设 & {len(nongpio)} \\\\',
 f'稳定字段（无需 \\texttt{{unstable}} 特性） & {n_stable} \\\\',
 f'标记 \\texttt{{unstable}} 的字段（本项目已开启该特性） & {n_unstable} \\\\',
 f'带中断源子成员的外设 & {len(ints)} \\\\',
 f'可挂 DMA 的外设 & {len(dma)} \\\\',
 r'\bottomrule', r'\end{tabular}', r'\end{center}',
]
FIG_STATS = "\n".join(stat_lines)

# 主表
rows = [r'\begin{center}\footnotesize',
        r'\begin{longtable}{@{}p{2.15cm}p{1.0cm}>{\raggedright\arraybackslash}p{9.4cm}p{1.65cm}@{}}',
        r'\caption{非 GPIO 字段（按元数据顺序）}\label{tab:main}\\',
        r'\toprule',
        r'\tblhead{字段} & \tblhead{门控} & \tblhead{驱动入口 / 递归下一层} & \tblhead{课} \\',
        r'\midrule', r'\endfirsthead',
        r'\toprule',
        r'\tblhead{字段} & \tblhead{门控} & \tblhead{驱动入口 / 递归下一层} & \tblhead{课} \\',
        r'\midrule', r'\endhead',
        r'\bottomrule', r'\endlastfoot']
def brk(s):
    # 让长条目可以在 “/”、“::”、“→” 之后断行，避免长表格溢出页面
    s = s.replace('/', '/\\allowbreak ')
    s = s.replace('::', '::\\allowbreak ')
    s = s.replace('→', '→\\allowbreak ')
    return s

for name, uns, cfg in nongpio:
    entry, lesson = E.get(name, (r'（内部寄存器块，无用户驱动）', ''))
    entry = brk(entry)
    rows.append(f'\\fld{{{esc(name)}}} & {"unstable" if uns else "稳定"} & {entry} & {lesson} \\\\')
rows += [r'\end{longtable}', r'\end{center}']
TBL_MAIN = "\n".join(rows)

# GPIO 表
rows = [r'\begin{center}\footnotesize',
        r'\begin{longtable}{@{}p{1.4cm}>{\raggedright\arraybackslash}p{5.2cm}>{\raggedright\arraybackslash}p{5.2cm}p{1.4cm}@{}}',
        r'\caption{GPIO 引脚字段与特殊说明}\label{tab:gpio}\\',
        r'\toprule',
        r'\tblhead{引脚} & \tblhead{限制 / 特殊说明} & \tblhead{本项目用途} & \tblhead{课} \\',
        r'\midrule', r'\endfirsthead',
        r'\toprule',
        r'\tblhead{引脚} & \tblhead{限制 / 特殊说明} & \tblhead{本项目用途} & \tblhead{课} \\',
        r'\midrule', r'\endhead', r'\bottomrule', r'\endlastfoot']
def pin_warn_text(name):
    w = warns.get(name, [])
    txt = '；'.join(x.rstrip('.').replace('GPIO', 'GPIO') for x in w)
    txt = txt.replace('This pin is a strapping pin, it determines how the chip boots',
                      'strapping 脚：决定芯片启动方式')
    txt = txt.replace('These pins may be used to debug the chip using USB',
                      '可被 USB 调试占用')
    txt = txt.replace('These pins may be used to debug the chip using an external JTAG debugger',
                      '可被外部 JTAG 调试器占用')
    txt = txt.replace('By default, this pin is used by the UART programming interface',
                      '默认被 UART 下载/日志接口使用')
    txt = txt.replace('This pin may be reserved for interfacing with SPI flash',
                      '可能被 SPI flash 占用')
    txt = txt.replace('This pin may be reserved for interfacing with Octal SPI flash',
                      '可能被 Octal SPI flash 占用')
    txt = txt.replace('This pin may be reserved for interfacing with SPI PSRAM',
                      '可能被 SPI PSRAM 占用')
    txt = txt.replace('This pin may be reserved for interfacing with Octal SPI PSRAM',
                      '可能被 Octal SPI PSRAM 占用')
    txt = txt.replace('This pin may be available with certain limitations. Check your hardware to make sure whether you can use it',
                      '有使用限制')
    return txt if txt else '通用 IO（无特殊限制）'
for name, uns, cfg in gpios:
    use = PIN_USE.get(name, '')
    lesson = ''
    m = re.search(r'（([0-9、,\-]+)）', use)
    if m:
        lesson = m.group(1).replace('、', ',')
    extra = '（需 \fld{use\\_xtal32k} 关闭才存在）' if cfg and 'xtal32k' in cfg else ''
    warn = pin_warn_text(name) + extra
    rows.append(f'\\fld{{{name}}} & {warn} & {use or "--"} & {lesson} \\\\')
rows += [r'\end{longtable}', r'\end{center}']
TBL_GPIO = "\n".join(rows)

# 中断源表
rows = [r'\begin{center}\footnotesize',
        r'\begin{longtable}{@{}p{2.2cm}p{2.85cm}>{\raggedright\arraybackslash}p{7.55cm}@{}}',
        r'\caption{带中断源子成员的外设}\label{tab:ints}\\',
        r'\toprule',
        r'\tblhead{字段} & \tblhead{中断源} & \tblhead{由元数据生成的方法} \\',
        r'\midrule', r'\endfirsthead',
        r'\toprule',
        r'\tblhead{字段} & \tblhead{中断源} & \tblhead{由元数据生成的方法} \\',
        r'\midrule', r'\endhead', r'\bottomrule', r'\endlastfoot']
for name in sorted(ints, key=lambda n: fields.index([f for f in fields if f[0] == n][0])):
    for intname, b, e, d in ints[name]:
        rows.append(f'\\fld{{{esc(name)}}} & \\fld{{{esc(intname)}}} & \\fld{{{esc(b)}}} / \\fld{{{esc(e)}}} / \\fld{{{esc(d)}}} \\\\')
rows += [r'\end{longtable}', r'\end{center}']
TBL_INTS = "\n".join(rows)

# DMA 表
rows = [r'\begin{center}\small',
        r'\begin{tabular}{@{}llll@{}}', r'\toprule',
        r'\tblhead{外设} & \tblhead{类型名} & \tblhead{通道号} & \tblhead{通道类型} \\',
        r'\midrule']
for name, ty, num, ch in dma:
    rows.append(f'\\fld{{{esc(name)}}} & \\fld{{{esc(ty)}}} & {num} & \\fld{{{esc(ch)}}} \\\\')
rows += [r'\bottomrule', r'\end{tabular}', r'\end{center}']
TBL_DMA = "\n".join(rows)



# ---------- 5. 把生成区写回 tex（幂等：可反复运行） ----------
import re as _re

def _wrap_region(anchor, body):
    """anchor 是生成区里唯一的一小段（表格列定义）。向前找 \\begin{center}、
    向后找 \\end{center}，把整块换成新的 body。"""
    global tex
    a = tex.find(anchor)
    if a < 0:
        raise SystemExit('没找到生成区锚点: ' + anchor)
    b = tex.rfind('\\begin{center}', 0, a)
    if b < 0:
        raise SystemExit('锚点前面没有 begin{center}: ' + anchor)
    e = tex.find('\\end{center}', a)
    if e < 0:
        raise SystemExit('生成区没有收尾的 end{center}: ' + anchor)
    e += len('\\end{center}')
    tex = tex[:b] + body + tex[e:]

tex = open(TEX).read()
_wrap_region('\\begin{tabular}{@{}lr@{}}', FIG_STATS)              # §1.4 统计表
_wrap_region('\\begin{longtable}{@{}p{2.15cm}', TBL_MAIN)          # §2.1 非 GPIO
_wrap_region('\\begin{longtable}{@{}p{1.4cm}', TBL_GPIO)           # §2.2 GPIO
_wrap_region('\\begin{longtable}{@{}p{2.2cm}', TBL_INTS)           # §2.3 中断源
_wrap_region('\\begin{tabular}{@{}llll@{}}', TBL_DMA)              # §2.4 DMA

# 正文里的数字同步刷新（表内数字由表格自身保证）
def _sub(pat, repl):
    global tex
    tex, n = _re.subn(pat, lambda _m: repl, tex)  # 用函数替换：避免把 repl 里的反斜杠当转义
    if n == 0:
        print('  提示：数字已经是最新，无需替换：' + pat[:40])

_sub(r'共 \d+ 个字段', '共 ' + str(n_total) + ' 个字段')
_sub(r'所以下面 \\fld\{Peripherals\} \d+ 个字段全部可用',
     '所以下面 \\fld{Peripherals} ' + str(n_total) + ' 个字段全部可用')
_sub(r'\\section\{全部成员一览（\d+ 个字段）\}',
     '\\section{全部成员一览（' + str(n_total) + ' 个字段）}')
_sub(r'\\subsection\{非 GPIO 字段（\d+ 个）\}',
     '\\subsection{非 GPIO 字段（' + str(len(nongpio)) + ' 个）}')
_sub(r'\\subsection\{带「中断源子成员」的字段（\d+ 个）\}',
     '\\subsection{带「中断源子成员」的字段（' + str(len(ints)) + ' 个）}')

open(TEX, 'w').write(tex)
print('已刷新 ' + TEX)
print(f'  字段总数={n_total}  非GPIO={len(nongpio)}  带中断源={len(ints)}  可挂DMA={len(dma)}')
