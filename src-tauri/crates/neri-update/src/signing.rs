//! Ed25519 清单签名。私钥只存在于发布流水线的 Secret 里，应用内只嵌公钥。
//!
//! 签名覆盖整份清单字节，清单里又有每个文件的 SHA-256，因此镜像、代理或被劫持的下载
//! 都无法替换任何文件；签名前加域前缀，避免同一把钥匙签出的其它数据被拿来冒充清单

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};

use crate::error::{UpdateError, UpdateResult};

const DOMAIN: &[u8] = b"neri-update-manifest-v1\n";

fn signed_message(manifest: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(DOMAIN.len() + manifest.len());
    message.extend_from_slice(DOMAIN);
    message.extend_from_slice(manifest);
    message
}

/// 校验清单签名。`public_key` 与 `signature` 均为 Base64（标准字母表，允许首尾空白）
pub fn verify(manifest: &[u8], signature: &str, public_key: &str) -> UpdateResult<()> {
    let key = STANDARD
        .decode(public_key.trim())
        .map_err(|error| UpdateError::BadKey(error.to_string()))?;
    if key.len() != 32 {
        return Err(UpdateError::BadKey("public key must be 32 bytes".into()));
    }
    let signature = STANDARD
        .decode(signature.trim())
        .map_err(|_| UpdateError::BadSignature)?;
    UnparsedPublicKey::new(&ED25519, key)
        .verify(&signed_message(manifest), &signature)
        .map_err(|_| UpdateError::BadSignature)
}

/// 生成新密钥对，返回 (PKCS#8 私钥 Base64, 公钥 Base64)
pub fn generate_keypair() -> UpdateResult<(String, String)> {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
        .map_err(|_| UpdateError::BadKey("key generation failed".into()))?;
    let pair = load_keypair_bytes(pkcs8.as_ref())?;
    Ok((
        STANDARD.encode(pkcs8.as_ref()),
        STANDARD.encode(pair.public_key().as_ref()),
    ))
}

/// 用 PKCS#8 私钥（Base64）签名，返回签名 Base64
pub fn sign(manifest: &[u8], private_key: &str) -> UpdateResult<String> {
    let pkcs8 = STANDARD
        .decode(private_key.trim())
        .map_err(|error| UpdateError::BadKey(error.to_string()))?;
    let pair = load_keypair_bytes(&pkcs8)?;
    Ok(STANDARD.encode(pair.sign(&signed_message(manifest)).as_ref()))
}

/// 由私钥推出公钥，发布前用来核对 Secret 与应用内嵌的公钥是同一对
pub fn public_key_of(private_key: &str) -> UpdateResult<String> {
    let pkcs8 = STANDARD
        .decode(private_key.trim())
        .map_err(|error| UpdateError::BadKey(error.to_string()))?;
    Ok(STANDARD.encode(load_keypair_bytes(&pkcs8)?.public_key().as_ref()))
}

fn load_keypair_bytes(pkcs8: &[u8]) -> UpdateResult<Ed25519KeyPair> {
    Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|error| UpdateError::BadKey(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_round_trips_and_detects_tampering() {
        let (private_key, public_key) = generate_keypair().unwrap();
        assert_eq!(public_key_of(&private_key).unwrap(), public_key);

        let manifest = br#"{"version":"1.1.0"}"#;
        let signature = sign(manifest, &private_key).unwrap();
        verify(manifest, &format!("{signature}\n"), &public_key).unwrap();

        let tampered = br#"{"version":"9.9.9"}"#;
        assert!(matches!(
            verify(tampered, &signature, &public_key),
            Err(UpdateError::BadSignature)
        ));

        let (_, other_key) = generate_keypair().unwrap();
        assert!(matches!(
            verify(manifest, &signature, &other_key),
            Err(UpdateError::BadSignature)
        ));
    }

    #[test]
    fn rejects_malformed_keys_and_signatures() {
        let (_, public_key) = generate_keypair().unwrap();
        assert!(matches!(
            verify(b"x", "not base64!", &public_key),
            Err(UpdateError::BadSignature)
        ));
        assert!(matches!(
            verify(b"x", "AAAA", "AAAA"),
            Err(UpdateError::BadKey(_))
        ));
        assert!(matches!(sign(b"x", "AAAA"), Err(UpdateError::BadKey(_))));
    }
}
