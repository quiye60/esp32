// ============================================================================
// 43 · 硬件加密加速：AES / SHA / TRNG
//
// 目标：ESP32-S3 里有专门的加密外设，跑同样的算法比软件快很多，而且不占 CPU：
//         1) AES   —— 对称加密（一把钥匙加密和解密）。本课用 AES-128 加/解密一块数据
//         2) SHA   —— 单向哈希（摘要），常用于校验和、签名。本课算 SHA-256
//         3) TRNG  —— 真随机数（靠物理噪声），比伪随机 Rng 更安全，
//                     适合生成密钥/证书；代价是必须先提供「随机源」
//       三个都用「块设备式」的接口：给数据、拿结果，硬件在背后干活。
//
// 参考：官方 hil-test/src/bin/crypto.rs（及 crypto/aes.rs、crypto/sha.rs、crypto/rng.rs）
//       esp-hal/src/aes/mod.rs、esp-hal/src/sha.rs、esp-hal/src/rng/trng.rs 的文档
//       结构照 src/bin/tasks.rs
//
// 接线：不需要外部器件
// 运行：cargo run --release --bin 43_crypto
// 预期：打印出 AES 解密后与原文一致；SHA-256("abc") 应该等于
//       BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD
//       （这是 RFC 标准测试向量，能对上就说明硬件算得没错）
// 坑：  ① AES 是「分组加密」：一次 16 字节，长度不够要补位（padding）
//       ② 硬件加速只加速算法本身，密钥管理/协议设计还是得自己保证安全
//       ③ TRNG 需要先建 TrngSource（它会占着 RNG 和 ADC1 两个外设），
//          想随时关掉的话要先把 Trng 放掉，否则 try_disable 会报错
//       ④ 想算 RSA/ECC（非对称）也有硬件加速，但缓冲区大得多，本课不展开
// ============================================================================

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::aes::Aes;
use esp_hal::clock::CpuClock;
use esp_hal::rng::{Rng, Trng, TrngSource};
use esp_hal::sha::{Sha, Sha256};
use esp_hal::timer::timg::TimerGroup;
use nb::block;                              // block! = 一直重试 nb::Result 直到完成
use rtt_target::rprintln;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("panic: {}", info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    rtt_target::rtt_init_print!();

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // ================= 1. AES-128：加解密 16 字节 =================
    // 密钥必须是 16 / 24 / 32 字节（对应 AES-128/192/256）
    let key: [u8; 16] = *b"SUp4SeCp@sSw0rd!";
    let mut block_data = [0u8; 16];
    block_data.copy_from_slice(b"hello esp32s3!!!");

    let mut aes = Aes::new(peripherals.AES);

    // encrypt/decrypt 都是「原地」操作：密文覆盖明文，注意先备份
    let plain_backup = block_data;
    aes.encrypt(&mut block_data, key);
    rprintln!("AES-128 密文：{:02X?}", block_data);
    rprintln!("明文和密文不同 = {}（当然）", block_data != plain_backup);

    aes.decrypt(&mut block_data, key);
    rprintln!("AES-128 解密：{:02X?}", block_data);
    rprintln!(
        "解密后和原文一致 = {}",
        block_data == plain_backup
    );
    // 换一把错误的钥匙会得到「看起来随机」的垃圾 —— 对称加密的完整性要靠 HMAC 之类保证
    let wrong_key: [u8; 16] = *b"WrongKey12345678";
    let mut wrong = plain_backup;
    aes.encrypt(&mut wrong, wrong_key);
    aes.decrypt(&mut wrong, key);
    rprintln!("用错钥匙解密 = {:02X?}（垃圾数据，协议里要靠校验发现）", wrong);

    // ================= 2. SHA-256：算摘要 =================
    // 用「标准测试向量」验证硬件：SHA-256("abc") 是公开写死的值
    let mut sha = Sha::new(peripherals.SHA);
    let mut digest = [0u8; 32];
    {
        let mut hasher = sha.start::<Sha256>();
        let mut input: &[u8] = b"abc";
        // update 是「流式」的：返回 nb::Result，用 block! 一直喂到吃完
        while !input.is_empty() {
            input = block!(hasher.update(input)).unwrap();
        }
        block!(hasher.finish(&mut digest)).unwrap();
    }
    rprintln!("SHA-256(\"abc\") = {:02X?}", digest);
    rprintln!(
        "与标准向量一致 = {}",
        digest
            == [
                0xBA, 0x78, 0x16, 0xBF, 0x8F, 0x01, 0xCF, 0xEA, 0x41, 0x41, 0x40, 0xDE,
                0x5D, 0xAE, 0x22, 0x23, 0xB0, 0x03, 0x61, 0xA3, 0x96, 0x17, 0x7A, 0x9C,
                0xB4, 0x10, 0xFF, 0x61, 0xF2, 0x00, 0x15, 0xAD,
            ]
    );

    // 换个长一点的内容，只要改动一个字节，摘要就应该完全不同（雪崩效应）
    let mut digest2 = [0u8; 32];
    {
        let mut hasher = sha.start::<Sha256>();
        let mut input: &[u8] = b"abc!";
        while !input.is_empty() {
            input = block!(hasher.update(input)).unwrap();
        }
        block!(hasher.finish(&mut digest2)).unwrap();
    }
    rprintln!("SHA-256(\"abc!\") 与上面不同的字节数 = {}（应该接近 16 个 = 一半）",
        digest.iter().zip(digest2.iter()).filter(|(a, b)| a != b).count());

    // ================= 3. TRNG：真随机数 =================
    // TrngSource 表示「打开了随机噪声源」，它占着 RNG 与 ADC1 两个外设
    let _source = TrngSource::new(peripherals.RNG, peripherals.ADC1);
    match Trng::try_new() {
        Ok(trng) => {
            let mut bytes = [0u8; 16];
            trng.read(&mut bytes);
            rprintln!("TRNG 16 字节真随机：{:02X?}", bytes);

            // 对比一下伪随机（Rng 是基于硬件的确定性源，速度快但可预测）
            let rng = Rng::new();
            rprintln!("Rng（伪随机）两个 u32：{:08X} {:08X}", rng.random(), rng.random());
        }
        Err(e) => rprintln!("TRNG 不可用：{:?}", e),
    }

    rprintln!("小结：密钥用 TRNG 生成、完整性用 SHA/HMAC 校验、数据用 AES 加密，是嵌入式安全的最小组合");

    loop {
        Timer::after(Duration::from_secs(30)).await;
    }
}
