//! 禁漫加解密：请求 token 与 API 响应体解密。
//!
//! 对应 Python 版 `jm_toolkit.JmCryptoTool`。

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes256;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use md5::{Digest, Md5};

use crate::error::{JmError, Result};

/// 当前 Unix 秒级时间戳。
pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn md5_hex(input: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(32);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// 计算接口 headers 的 `token` 与 `tokenparam`。
pub fn token_and_tokenparam(ts: i64, ver: &str, secret: &str) -> (String, String) {
    let tokenparam = format!("{ts},{ver}");
    let token = md5_hex(&format!("{ts}{secret}"));
    (token, tokenparam)
}

fn aes256_ecb_key(ts: i64, secret: &str) -> [u8; 32] {
    let hex = md5_hex(&format!("{ts}{secret}"));
    let mut key = [0u8; 32];
    key.copy_from_slice(hex.as_bytes());
    key
}

/// 解密接口返回值 `data`（base64 + AES-256-ECB），返回明文 JSON 字符串。
pub fn decode_resp_data(data_b64: &str, ts: i64, secret: &str) -> Result<String> {
    let payload = STANDARD
        .decode(data_b64.trim())
        .map_err(|e| JmError::Msg(format!("响应数据 base64 解码失败: {e}")))?;

    if payload.is_empty() || payload.len() % 16 != 0 {
        return JmError::msg(format!(
            "响应数据密文长度异常: {} 字节（必须是 16 的倍数）",
            payload.len()
        ));
    }

    let key = aes256_ecb_key(ts, secret);
    let cipher =
        Aes256::new_from_slice(&key).map_err(|e| JmError::Msg(format!("AES 初始化失败: {e}")))?;

    let mut buffer = payload.clone();
    for block in buffer.chunks_exact_mut(16) {
        let block: &mut [u8; 16] = block
            .try_into()
            .map_err(|_| JmError::Msg("AES 分块失败".to_string()))?;
        cipher.decrypt_block(block.into());
    }

    // 去掉 PKCS#7 填充
    let pad = *buffer.last().unwrap_or(&0) as usize;
    let keep = if pad > 0 && pad <= 16 && pad <= buffer.len() {
        buffer.len() - pad
    } else {
        buffer.len()
    };
    buffer.truncate(keep);

    String::from_utf8(buffer).map_err(|e| JmError::Msg(format!("响应数据不是合法的 UTF-8: {e}")))
}

/// `decode_resp_data(data, ts='')`：用于域名服务器等无时间戳的响应。
pub fn decode_resp_data_no_ts(data_b64: &str, secret: &str) -> Result<String> {
    decode_resp_data(data_b64, 0, secret)
}

/// 加密接口响应体（`decode_resp_data` 的逆操作）。
///
/// 生产环境不会用到；保留该函数用于本地回环模拟与调试协议。
pub fn encode_resp_data(plain: &str, ts: i64, secret: &str) -> Result<String> {
    let key = aes256_ecb_key(ts, secret);
    let cipher =
        Aes256::new_from_slice(&key).map_err(|e| JmError::Msg(format!("AES 初始化失败: {e}")))?;

    let mut buffer = plain.as_bytes().to_vec();
    let pad = 16 - (buffer.len() % 16);
    buffer.extend(std::iter::repeat(pad as u8).take(pad));

    for block in buffer.chunks_exact_mut(16) {
        let block: &mut [u8; 16] = block
            .try_into()
            .map_err(|_| JmError::Msg("AES 分块失败".to_string()))?;
        cipher.encrypt_block(block.into());
    }

    Ok(STANDARD.encode(&buffer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_reference() {
        assert_eq!(md5_hex(""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex("abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn token_uses_ts_and_secret() {
        let (token, tokenparam) = token_and_tokenparam(1700566805, "2.0.28", "185Hcomic3PAPP7R");
        assert_eq!(tokenparam, "1700566805,2.0.28");
        assert_eq!(token, md5_hex("1700566805185Hcomic3PAPP7R"));
        assert_eq!(token.len(), 32);
    }

    #[test]
    fn decode_roundtrip() {
        let plain = r#"{"code":200,"data":"ok","name":"测试"}"#.as_bytes().to_vec();
        // 手工构造 PKCS#7 填充并加密
        let block_size = 16usize;
        let pad = block_size - (plain.len() % block_size);
        let mut padded = plain.to_vec();
        padded.extend(std::iter::repeat(pad as u8).take(pad));

        let ts = 1700566805;
        let key = aes256_ecb_key(ts, "185Hcomic3PAPP7R");
        let cipher = Aes256::new_from_slice(&key).unwrap();
        let mut encrypted = padded.clone();
        for block in encrypted.chunks_exact_mut(16) {
            let block: &mut [u8; 16] = block.try_into().unwrap();
            cipher.encrypt_block(block.into());
        }

        let b64 = STANDARD.encode(&encrypted);
        let decoded = decode_resp_data(&b64, ts, "185Hcomic3PAPP7R").unwrap();
        assert_eq!(decoded, String::from_utf8_lossy(&plain));
    }

    #[test]
    fn rejects_broken_ciphertext() {
        assert!(decode_resp_data("not-base64!!", 1, "k").is_err());
        assert!(decode_resp_data("c2hvcnQ=", 1, "k").is_err());
    }
}
