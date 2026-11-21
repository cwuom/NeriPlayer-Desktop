// 网易云音乐 API 加密模块
// 实现 WEAPI / EAPI / LinuxAPI 三种加密模式

use aes::cipher::{
    block_padding::Pkcs7, generic_array::GenericArray, BlockDecrypt, BlockEncrypt, BlockEncryptMut,
    KeyInit, KeyIvInit,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use md5::{Digest, Md5};
use rand::Rng;
use rsa::BigUint;

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

// 网易云固定密钥
const PRESET_KEY: &[u8; 16] = b"0CoJUm6Qyw8W8jud";
const IV: &[u8; 16] = b"0102030405060708";
const EAPI_KEY: &[u8; 16] = b"e82ckenh8dichen8";
const LINUX_KEY: &[u8; 16] = b"rFgB&h#%2?^eDg:Q";

// RSA 公钥
const RSA_PUB_KEY_HEX: &str = "010001";
const RSA_MODULUS_HEX: &str = "\
00e0b509f6259df8642dbc35662901477df22677ec152b5ff68ace615bb7b72\
5152b3ab17a876aea8a5aa76d2e417629ec4ee341f56135fccf695280104e0312\
ecbda92557c93870114af6c9d05c4f7f0c3685b7a46bee255932575cce10b424d\
813cfe4875d3e82047b97ddef52741d546b8e289dc6935b3ece0462db0a22b8e7";

/// AES-128-CBC 加密
fn aes_cbc_encrypt(data: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> Vec<u8> {
    let mut buf = vec![0u8; data.len() + 16];
    buf[..data.len()].copy_from_slice(data);
    let ct = Aes128CbcEnc::new(key.into(), iv.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buf, data.len())
        .expect("CBC encrypt failed");
    ct.to_vec()
}

/// AES-128-ECB 加密（手动 PKCS7 填充 + 逐块加密）
fn aes_ecb_encrypt(data: &[u8], key: &[u8; 16]) -> Vec<u8> {
    let cipher = aes::Aes128::new(GenericArray::from_slice(key));

    // PKCS7 填充
    let pad_len = 16 - (data.len() % 16);
    let mut padded = data.to_vec();
    padded.extend(std::iter::repeat_n(pad_len as u8, pad_len));

    // 逐块加密
    for chunk in padded.chunks_exact_mut(16) {
        let block = GenericArray::from_mut_slice(chunk);
        cipher.encrypt_block(block);
    }
    padded
}

/// AES-128-ECB 解密（手动逐块解密 + PKCS7 去填充）
fn aes_ecb_decrypt(data: &[u8], key: &[u8; 16]) -> Option<Vec<u8>> {
    if data.is_empty() || !data.len().is_multiple_of(16) {
        return None;
    }
    let cipher = aes::Aes128::new(GenericArray::from_slice(key));
    let mut buf = data.to_vec();

    // 逐块解密
    for chunk in buf.chunks_exact_mut(16) {
        let block = GenericArray::from_mut_slice(chunk);
        cipher.decrypt_block(block);
    }

    // 去 PKCS7 填充
    let pad = *buf.last()? as usize;
    if pad == 0 || pad > 16 {
        return None;
    }
    if buf.len() < pad {
        return None;
    }
    // 验证填充
    if buf[buf.len() - pad..].iter().any(|&b| b as usize != pad) {
        return None;
    }
    buf.truncate(buf.len() - pad);
    Some(buf)
}

/// 解密 EAPI 响应：AES-128-ECB 解密 -> 提取 JSON 段
pub fn eapi_decrypt(hex_data: &[u8]) -> Option<String> {
    // 响应可能是 raw bytes（非 hex），直接尝试解密
    let decrypted = aes_ecb_decrypt(hex_data, EAPI_KEY)?;
    let text = String::from_utf8_lossy(&decrypted);

    // EAPI 格式：url-36cd479b6b5-json-36cd479b6b5-md5
    // 提取中间的 JSON 段
    let parts: Vec<&str> = text.split("-36cd479b6b5-").collect();
    if parts.len() >= 2 {
        Some(parts[1].to_string())
    } else {
        // 如果没有分隔符，整体可能就是 JSON
        Some(text.into_owned())
    }
}

/// 生成 16 字节随机密钥
fn random_key() -> [u8; 16] {
    let charset = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    let mut key = [0u8; 16];
    for b in &mut key {
        *b = charset[rng.gen_range(0..charset.len())];
    }
    key
}

/// WEAPI 加密：双重 AES-CBC + RSA
pub fn weapi_encrypt(json: &str) -> (String, String) {
    let sec_key = random_key();

    // 第一层 AES-CBC
    let first = aes_cbc_encrypt(json.as_bytes(), PRESET_KEY, IV);
    let first_b64 = B64.encode(&first);

    // 第二层 AES-CBC
    let second = aes_cbc_encrypt(first_b64.as_bytes(), &sec_key, IV);
    let params = B64.encode(&second);

    // RSA 加密 reversed key（无填充，直接模幂）
    let mut reversed = sec_key;
    reversed.reverse();

    let modulus = BigUint::parse_bytes(RSA_MODULUS_HEX.replace(['\n', ' '], "").as_bytes(), 16)
        .expect("Invalid modulus");
    let exponent = BigUint::parse_bytes(RSA_PUB_KEY_HEX.as_bytes(), 16).expect("Invalid exponent");

    let m = BigUint::from_bytes_be(&reversed);
    let c = m.modpow(&exponent, &modulus);
    let enc_sec_key = format!("{:0>256x}", c);

    (params, enc_sec_key)
}

/// EAPI 加密
pub fn eapi_encrypt(url: &str, json: &str) -> String {
    // 对齐 Android NeteaseCrypto.eApiEncrypt：digest 与载荷都用 /api 路径，
    // 传入 /eapi 路径时先替换（幂等，传 /api 也安全）
    let path = url.replace("/eapi", "/api");
    let message = format!("nobody{}use{}md5forencrypt", path, json);
    let digest = format!("{:x}", Md5::digest(message.as_bytes()));
    let data = format!("{}-36cd479b6b5-{}-36cd479b6b5-{}", path, json, digest);
    let encrypted = aes_ecb_encrypt(data.as_bytes(), EAPI_KEY);
    hex::encode_upper(&encrypted)
}

/// LinuxAPI 加密
pub fn linux_encrypt(json: &str) -> String {
    let encrypted = aes_ecb_encrypt(json.as_bytes(), LINUX_KEY);
    hex::encode(&encrypted)
}
