// Release 构建使用应用侧加密文件存储；Debug 构建完全使用随机路径明文文件
// 各类凭据使用独立目录，删除 Cookie 时不会影响同步凭据
//
// 为什么不用系统钥匙串：未经正式签名的应用每次更新后二进制指纹变化，
// macOS Keychain 会把它当成新应用反复弹授权框（甚至要求输入登录密码），
// 用户会以为软件在偷凭据。对标 Android 端的 EncryptedSharedPreferences：
// 应用侧加密、零系统交互。威胁模型一致——防的是「文件被拷走后可读」，
// 不承诺抵御同机同用户的恶意进程（Keychain 对未签名应用同样给不出这个承诺）。
// 旧 Keychain 凭据在首次读取时自动迁入并删除。

const SERVICE_NAME: &str = "moe.ouom.neriplayer.desktop";

pub const AUTH_STATE_KEY: &str = "auth-state-v1";
pub const GITHUB_TOKEN_KEY: &str = "github-token-v1";
pub const WEBDAV_PASSWORD_KEY: &str = "webdav-password-v1";

/// 从当前构建配置的凭据存储读取秘密值
///
/// Release 只读加密文件。旧 Keychain 凭据在整个安装生命周期里只迁移一次
/// （见 [`migrate_legacy_keychain_once`]）：以前每次读不到文件都回落 Keychain，
/// 没配置过的 WebDAV/GitHub 凭据永远读不到，ad-hoc 签名的新版本每次启动都会弹授权框。
pub fn get_secret(key: &str) -> Option<String> {
    #[cfg(debug_assertions)]
    {
        debug_secret_storage::get(key)
    }

    #[cfg(not(debug_assertions))]
    {
        if let Some(value) = encrypted_secret_storage::get(key) {
            return Some(value);
        }
        if migrate_legacy_keychain_once() {
            return encrypted_secret_storage::get(key);
        }
        None
    }
}

/// 把旧 Keychain 里的全部已知凭据一次性迁入加密文件，并写下标记永不再碰 Keychain。
/// 返回本次是否真的执行了迁移（已迁移过则为 false）
#[cfg(not(debug_assertions))]
fn migrate_legacy_keychain_once() -> bool {
    static MIGRATION: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = MIGRATION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if encrypted_secret_storage::legacy_keychain_migrated() {
        return false;
    }
    for key in [AUTH_STATE_KEY, GITHUB_TOKEN_KEY, WEBDAV_PASSWORD_KEY] {
        if encrypted_secret_storage::get(key).is_some() {
            continue;
        }
        let Some(legacy) = get_keyring_secret(key).filter(|value| !value.is_empty()) else {
            continue;
        };
        if encrypted_secret_storage::set(key, &legacy) {
            let _ = delete_keyring_secret(key);
            log::info!(target: "security", "credential migrated off the keychain: {key}");
        }
    }
    encrypted_secret_storage::mark_legacy_keychain_migrated();
    true
}

/// 写入当前构建配置的凭据存储，失败时返回 false
pub fn set_secret(key: &str, value: &str) -> bool {
    #[cfg(debug_assertions)]
    {
        debug_secret_storage::set(key, value)
    }

    #[cfg(not(debug_assertions))]
    {
        encrypted_secret_storage::set(key, value)
    }
}

/// 删除当前构建配置的凭据存储中的秘密值
pub fn delete_secret(key: &str) -> bool {
    #[cfg(debug_assertions)]
    {
        debug_secret_storage::delete(key)
    }

    #[cfg(not(debug_assertions))]
    {
        // 先完成一次性迁移，确保 Keychain 里不会残留一份删不掉、下次又被迁回来的旧副本
        migrate_legacy_keychain_once();
        encrypted_secret_storage::delete(key)
    }
}

/// Debug 构建中检查指定凭据是否存在，不会访问系统钥匙串
pub fn debug_secret_exists(key: &str) -> bool {
    #[cfg(debug_assertions)]
    {
        debug_secret_storage::exists(key)
    }

    #[cfg(not(debug_assertions))]
    {
        let _ = key;
        false
    }
}

/// 应用侧加密凭据存储（Release 默认）
///
/// AES-256-CBC + HMAC-SHA256（encrypt-then-MAC），随机 IV。
/// 密钥派生自机器标识 + 应用盐：文件拷到别的机器无法解密，
/// 本机不落明文。文件权限收紧到 0600（Unix）。
/// Debug 构建用明文随机路径存储（见 debug_secret_storage），本模块只在
/// Release 与测试中编译，避免 debug 下的 dead_code 告警。
#[cfg(any(not(debug_assertions), test))]
mod encrypted_secret_storage {
    use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
    use hmac::{Hmac, Mac};
    use rand::RngCore;
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
    type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
    type HmacSha256 = Hmac<Sha256>;

    const MAGIC: &[u8; 4] = b"NPS1";
    const APP_SALT: &[u8] = b"moe.ouom.neriplayer.desktop/secret-store/v1";

    // 凭据文件读写全程互斥：8 处 save_auth 调用点可能并发，
    // 无锁时两笔写互相覆盖会损坏文件（凭据损坏 = 用户被登出且不可恢复）
    static SECRET_LOCK: Mutex<()> = Mutex::new(());

    fn lock_store() -> MutexGuard<'static, ()> {
        SECRET_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn store_dir() -> PathBuf {
        // 测试注入点：单测绝不写真实用户 data_dir
        #[cfg(test)]
        {
            static TEST_ROOT: OnceLock<PathBuf> = OnceLock::new();
            TEST_ROOT
                .get_or_init(|| {
                    std::env::temp_dir()
                        .join(format!("neri-secure-store-test-{}", std::process::id()))
                })
                .clone()
        }
        #[cfg(not(test))]
        {
            let mut path = dirs_next::data_dir().unwrap_or_else(|| PathBuf::from("."));
            path.push("NeriPlayer");
            path.push("secure");
            path
        }
    }

    fn legacy_keychain_marker_path() -> PathBuf {
        store_dir().join("legacy-keychain-migrated")
    }

    #[cfg_attr(test, allow(dead_code))]
    pub(super) fn legacy_keychain_migrated() -> bool {
        legacy_keychain_marker_path().exists()
    }

    #[cfg_attr(test, allow(dead_code))]
    pub(super) fn mark_legacy_keychain_migrated() {
        let _ = crate::fsutil::atomic_write_private(legacy_keychain_marker_path(), b"1");
    }

    fn secret_path(key: &str) -> PathBuf {
        // key 是本模块内的常量（auth-state-v1 等），不含路径成分；
        // 再做一次散列命名，目录里看不出哪个文件对应哪类凭据
        let digest = Sha256::digest([APP_SALT, key.as_bytes()].concat());
        store_dir().join(format!("{}.bin", hex::encode(&digest[..16])))
    }

    /// 机器标识：跨机器不可复制即可，不要求全球唯一
    ///
    /// OnceLock 缓存首次结果：macOS 每次派生都 spawn `ioreg` 太重，
    /// 且中途一次失败会静默切到 env 回退派生、导致同进程内新旧密文
    /// 密钥漂移——缓存后单进程内密钥必然一致
    fn machine_identity() -> &'static [u8] {
        static IDENTITY: OnceLock<Vec<u8>> = OnceLock::new();
        IDENTITY.get_or_init(compute_machine_identity)
    }

    fn compute_machine_identity() -> Vec<u8> {
        #[cfg(target_os = "linux")]
        {
            if let Ok(id) = std::fs::read_to_string("/etc/machine-id") {
                let id = id.trim();
                if !id.is_empty() {
                    return id.as_bytes().to_vec();
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            // ioreg 偶发启动失败时重试，避免一次瞬时命令错误切换到不稳定的环境变量回退
            for _ in 0..3 {
                if let Ok(output) = std::process::Command::new("ioreg")
                    .args(["-rd1", "-c", "IOPlatformExpertDevice"])
                    .output()
                {
                    if !output.status.success() {
                        continue;
                    }
                    let text = String::from_utf8_lossy(&output.stdout);
                    for line in text.lines() {
                        if let Some(rest) = line.split("\"IOPlatformUUID\" = \"").nth(1) {
                            if let Some(uuid) = rest.split('"').next().filter(|id| !id.is_empty()) {
                                return uuid.as_bytes().to_vec();
                            }
                        }
                    }
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            // 主机名/用户名可变，Windows MachineGuid 才是跨启动稳定的本机标识
            if let Ok(output) = std::process::Command::new("reg")
                .args([
                    "query",
                    r"HKLM\SOFTWARE\Microsoft\Cryptography",
                    "/v",
                    "MachineGuid",
                ])
                .output()
            {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout);
                    if let Some(value) = text
                        .lines()
                        .find(|line| line.to_ascii_lowercase().contains("machineguid"))
                        .and_then(|line| line.split_whitespace().last())
                        .filter(|value| !value.is_empty())
                    {
                        return value.as_bytes().to_vec();
                    }
                }
            }
        }
        fallback_machine_identity()
    }

    // 早期 Windows 构建与非 Windows 兜底使用这一组环境变量派生密钥。
    // MachineGuid 上线后仍需保留它作为旧密文的只读迁移候选
    fn fallback_machine_identity() -> Vec<u8> {
        let mut fallback = Vec::new();
        for var in [
            "COMPUTERNAME",
            "HOSTNAME",
            "USER",
            "USERNAME",
            "HOME",
            "USERPROFILE",
        ] {
            if let Ok(value) = std::env::var(var) {
                fallback.extend_from_slice(value.as_bytes());
                fallback.push(0);
            }
        }
        fallback
    }

    fn legacy_machine_identity_candidates() -> Vec<Vec<u8>> {
        #[cfg(target_os = "windows")]
        {
            vec![fallback_machine_identity()]
        }
        #[cfg(not(target_os = "windows"))]
        {
            Vec::new()
        }
    }

    fn derive_keys_for_identity(identity: &[u8]) -> ([u8; 32], [u8; 32]) {
        let enc = Sha256::digest([APP_SALT, b"/enc/", identity].concat());
        let mac = Sha256::digest([APP_SALT, b"/mac/", identity].concat());
        (enc.into(), mac.into())
    }

    fn derive_keys() -> ([u8; 32], [u8; 32]) {
        derive_keys_for_identity(machine_identity())
    }

    pub(super) fn get(key: &str) -> Option<String> {
        let _guard = lock_store();
        get_inner(key)
    }

    fn get_inner(key: &str) -> Option<String> {
        let path = secret_path(key);
        let blob = match std::fs::read(&path) {
            Ok(blob) => blob,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                // 权限/磁盘等瞬时读取故障不能当作损坏文件隔离，保留现场等待下次重试
                log::warn!(
                    target: "security",
                    "读取加密凭据失败, 保留文件等待重试: {error}"
                );
                return None;
            }
        };
        let legacy_identities = legacy_machine_identity_candidates();
        match decode_blob_with_identity_candidates(
            key,
            &blob,
            machine_identity(),
            &legacy_identities,
        ) {
            Ok((plain, needs_reencrypt)) => {
                if needs_reencrypt {
                    // 旧 MAC 格式或旧 Windows 身份验证通过后立即用当前身份重写，
                    // 之后 blob 改名冒充其它凭据不再可行
                    let _ = set_inner(key, &plain);
                }
                Some(plain)
            }
            Err(reason) => {
                // 校验失败的文件不可再用：隔离现场（保排查线索），
                // 同时让后续读取直接 miss 走 keyring 迁移路径而非反复失败
                let quarantined = crate::fsutil::quarantine_corrupt_file(&path);
                log::warn!(
                    target: "security",
                    "加密凭据校验失败({reason}), 文件已隔离: {quarantined:?}",
                );
                None
            }
        }
    }

    /// 解码 blob，返回 `(明文, 是否旧格式)`
    ///
    /// 布局: MAGIC(4) | IV(16) | MAC(32) | ciphertext。
    /// 新格式 MAC 覆盖 `key 名 + IV + 密文`；旧格式仅 `IV + 密文`。
    /// 先按新格式验，失败退回旧格式（向后兼容存量文件）
    fn decode_blob_with_identity(
        key: &str,
        blob: &[u8],
        identity: &[u8],
    ) -> Result<(String, bool), &'static str> {
        if blob.len() < 4 + 16 + 32 + 16 || &blob[..4] != MAGIC {
            return Err("bad header");
        }
        let (enc_key, mac_key) = derive_keys_for_identity(identity);
        let iv: [u8; 16] = blob[4..20].try_into().map_err(|_| "bad iv")?;
        let stored_mac = &blob[20..52];
        let ciphertext = &blob[52..];

        let verify = |bind_key: bool| -> bool {
            let Ok(mut mac) = HmacSha256::new_from_slice(&mac_key) else {
                return false;
            };
            if bind_key {
                mac.update(key.as_bytes());
            }
            mac.update(&iv);
            mac.update(ciphertext);
            mac.verify_slice(stored_mac).is_ok()
        };
        let legacy = if verify(true) {
            false
        } else if verify(false) {
            true
        } else {
            return Err("mac mismatch");
        };

        let mut buffer = ciphertext.to_vec();
        let plain = Aes256CbcDec::new(&enc_key.into(), &iv.into())
            .decrypt_padded_mut::<Pkcs7>(&mut buffer)
            .map_err(|_| "decrypt failed")?;
        let plain = String::from_utf8(plain.to_vec()).map_err(|_| "invalid utf8")?;
        Ok((plain, legacy))
    }

    #[cfg(test)]
    fn decode_blob(key: &str, blob: &[u8]) -> Result<(String, bool), &'static str> {
        decode_blob_with_identity(key, blob, machine_identity())
    }

    fn decode_blob_with_identity_candidates(
        key: &str,
        blob: &[u8],
        current_identity: &[u8],
        legacy_identities: &[Vec<u8>],
    ) -> Result<(String, bool), &'static str> {
        let current_error = match decode_blob_with_identity(key, blob, current_identity) {
            Ok(result) => return Ok(result),
            Err(error) => error,
        };

        for identity in legacy_identities {
            if identity.as_slice() == current_identity {
                continue;
            }
            if let Ok((plain, _legacy_format)) = decode_blob_with_identity(key, blob, identity) {
                return Ok((plain, true));
            }
        }

        Err(current_error)
    }

    pub(super) fn set(key: &str, value: &str) -> bool {
        let _guard = lock_store();
        set_inner(key, value)
    }

    fn set_inner(key: &str, value: &str) -> bool {
        let (enc_key, mac_key) = derive_keys();
        let mut iv = [0_u8; 16];
        rand::thread_rng().fill_bytes(&mut iv);

        let mut buffer = vec![0_u8; value.len() + 16];
        let Ok(ciphertext) = Aes256CbcEnc::new(&enc_key.into(), &iv.into())
            .encrypt_padded_b2b_mut::<Pkcs7>(value.as_bytes(), &mut buffer)
        else {
            return false;
        };

        let Ok(mut mac) = HmacSha256::new_from_slice(&mac_key) else {
            return false;
        };
        // 新格式：MAC 同时覆盖 key 名，blob 改名后无法冒充其它凭据
        mac.update(key.as_bytes());
        mac.update(&iv);
        mac.update(ciphertext);
        let tag = mac.finalize().into_bytes();

        let mut blob = Vec::with_capacity(4 + 16 + 32 + ciphertext.len());
        blob.extend_from_slice(MAGIC);
        blob.extend_from_slice(&iv);
        blob.extend_from_slice(&tag);
        blob.extend_from_slice(ciphertext);

        // 原子写 + 以 0600 创建：消除旧实现「0644 写入后再 chmod」的
        // 明文权限窗口，也消除崩溃/断电半写损坏（凭据损坏 = 不可恢复登出）
        crate::fsutil::atomic_write_private(secret_path(key), &blob).is_ok()
    }

    pub(super) fn delete(key: &str) -> bool {
        let _guard = lock_store();
        match std::fs::remove_file(secret_path(key)) {
            Ok(()) => true,
            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        }
    }

    #[cfg(test)]
    mod tests {
        /// 密文必须过 MAC 校验：翻转任何一个字节都读不出值
        #[test]
        fn tampered_blob_is_rejected() {
            let key = "test-tamper-v1";
            assert!(super::set(key, "secret-value"));
            let path = super::secret_path(key);
            let mut blob = std::fs::read(&path).unwrap();
            let last = blob.len() - 1;
            blob[last] ^= 0x01;
            std::fs::write(&path, blob).unwrap();
            assert_eq!(super::get(key), None, "篡改后的密文必须拒收");
            assert!(super::delete(key));
        }

        #[test]
        fn roundtrip_and_isolation() {
            let key_a = "test-roundtrip-a";
            let key_b = "test-roundtrip-b";
            assert!(super::set(key_a, "值A：含中文 / emoji 🎵"));
            assert!(super::set(key_b, "value-b"));
            assert_eq!(super::get(key_a).as_deref(), Some("值A：含中文 / emoji 🎵"));
            assert_eq!(super::get(key_b).as_deref(), Some("value-b"));
            assert!(super::delete(key_a));
            assert_eq!(super::get(key_a), None, "删除后不得残留");
            assert_eq!(super::get(key_b).as_deref(), Some("value-b"));
            assert!(super::delete(key_b));
        }

        /// 明文绝不允许出现在落盘文件里
        #[test]
        fn plaintext_never_touches_disk() {
            let key = "test-no-plaintext";
            let secret = "MUSIC_U=super-secret-cookie-value";
            assert!(super::set(key, secret));
            let blob = std::fs::read(super::secret_path(key)).unwrap();
            let haystack = String::from_utf8_lossy(&blob);
            assert!(
                !haystack.contains("super-secret"),
                "密文文件里不得有明文片段"
            );
            assert!(super::delete(key));
        }

        /// 按旧格式（MAC 不含 key 名）手工构造 blob，模拟升级前的存量文件
        fn write_legacy_blob(key: &str, value: &str) {
            use super::{Aes256CbcEnc, HmacSha256, MAGIC};
            use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
            use hmac::Mac;
            use rand::RngCore;

            let (enc_key, mac_key) = super::derive_keys();
            let mut iv = [0_u8; 16];
            rand::thread_rng().fill_bytes(&mut iv);
            let mut buffer = vec![0_u8; value.len() + 16];
            let ciphertext = Aes256CbcEnc::new(&enc_key.into(), &iv.into())
                .encrypt_padded_b2b_mut::<Pkcs7>(value.as_bytes(), &mut buffer)
                .unwrap();
            let mut mac = HmacSha256::new_from_slice(&mac_key).unwrap();
            mac.update(&iv);
            mac.update(ciphertext);
            let tag = mac.finalize().into_bytes();
            let mut blob = Vec::new();
            blob.extend_from_slice(MAGIC);
            blob.extend_from_slice(&iv);
            blob.extend_from_slice(&tag);
            blob.extend_from_slice(ciphertext);
            crate::fsutil::atomic_write_private(super::secret_path(key), &blob).unwrap();
        }

        fn blob_with_identity(key: &str, value: &str, identity: &[u8]) -> Vec<u8> {
            use super::{Aes256CbcEnc, HmacSha256, MAGIC};
            use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
            use hmac::Mac;

            let (enc_key, mac_key) = super::derive_keys_for_identity(identity);
            let iv = [7_u8; 16];
            let mut buffer = vec![0_u8; value.len() + 16];
            let ciphertext = Aes256CbcEnc::new(&enc_key.into(), &iv.into())
                .encrypt_padded_b2b_mut::<Pkcs7>(value.as_bytes(), &mut buffer)
                .unwrap();
            let mut mac = HmacSha256::new_from_slice(&mac_key).unwrap();
            mac.update(key.as_bytes());
            mac.update(&iv);
            mac.update(ciphertext);
            let tag = mac.finalize().into_bytes();
            let mut blob = Vec::new();
            blob.extend_from_slice(MAGIC);
            blob.extend_from_slice(&iv);
            blob.extend_from_slice(&tag);
            blob.extend_from_slice(ciphertext);
            blob
        }

        /// 旧格式 blob 可读，且读取后透明升级为新格式（key 参与 MAC）
        #[test]
        fn legacy_blob_is_readable_and_transparently_upgraded() {
            let key = "test-legacy-upgrade";
            write_legacy_blob(key, "legacy-secret");
            assert_eq!(super::get(key).as_deref(), Some("legacy-secret"));

            let blob = std::fs::read(super::secret_path(key)).unwrap();
            let (plain, legacy) = super::decode_blob(key, &blob).expect("upgraded blob decodes");
            assert_eq!(plain, "legacy-secret");
            assert!(!legacy, "读取后必须已重写为新格式");
            assert!(super::delete(key));
        }

        #[test]
        fn legacy_identity_candidate_is_readable_and_requires_reencryption() {
            let key = "test-legacy-identity";
            let old_identity = b"old-windows-environment-identity";
            let current_identity = b"new-windows-machine-guid";
            let blob = blob_with_identity(key, "legacy-secret", old_identity);

            let (plain, needs_reencrypt) = super::decode_blob_with_identity_candidates(
                key,
                &blob,
                current_identity,
                &[old_identity.to_vec()],
            )
            .expect("legacy identity candidate should decrypt");

            assert_eq!(plain, "legacy-secret");
            assert!(needs_reencrypt, "旧身份密文必须用当前身份重写");
        }

        /// 新格式 MAC 绑定 key 名：blob 拷到别的 key 路径读不出且被隔离
        #[test]
        fn renamed_blob_cannot_impersonate_another_key() {
            let key_src = "test-impersonate-src";
            let key_dst = "test-impersonate-dst";
            assert!(super::set(key_src, "value-src"));
            std::fs::copy(super::secret_path(key_src), super::secret_path(key_dst)).unwrap();

            assert_eq!(super::get(key_dst), None, "改名冒充必须拒收");
            assert!(
                !super::secret_path(key_dst).exists(),
                "校验失败的文件必须被隔离"
            );
            assert_eq!(super::get(key_src).as_deref(), Some("value-src"));
            assert!(super::delete(key_src));
        }

        /// 落盘文件必须以 0600 原子创建，不存在权限宽松窗口
        #[cfg(unix)]
        #[test]
        fn secret_file_is_created_private() {
            use std::os::unix::fs::PermissionsExt;
            let key = "test-private-mode";
            assert!(super::set(key, "v"));
            let mode = std::fs::metadata(super::secret_path(key))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
            assert!(super::delete(key));
        }
    }
}

#[cfg(not(debug_assertions))]
fn get_keyring_secret(key: &str) -> Option<String> {
    let entry = keyring::Entry::new(SERVICE_NAME, key).ok()?;
    entry.get_password().ok()
}

#[cfg(not(debug_assertions))]
fn delete_keyring_secret(key: &str) -> bool {
    let Ok(entry) = keyring::Entry::new(SERVICE_NAME, key) else {
        return false;
    };
    if entry.delete_credential().is_ok() {
        return true;
    }
    // 某些后端只支持覆盖值，清空后也不会再携带可用凭据
    entry.set_password("").is_ok()
}

#[cfg(debug_assertions)]
mod debug_secret_storage {
    use super::{AUTH_STATE_KEY, GITHUB_TOKEN_KEY, SERVICE_NAME, WEBDAV_PASSWORD_KEY};
    use serde::{Deserialize, Serialize};
    use std::fs;
    use std::io::{self, ErrorKind};
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, MutexGuard};
    use uuid::Uuid;

    const AUTH_STORE_DIRECTORY: &str = "debug-auth";
    const GITHUB_STORE_DIRECTORY: &str = "debug-github-token";
    const WEBDAV_STORE_DIRECTORY: &str = "debug-webdav-password";
    const LOCATION_FILE: &str = "location.json";
    static STORE_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Debug, Deserialize, Serialize)]
    struct StorageLocation {
        directory: String,
        file: String,
    }

    impl StorageLocation {
        fn new() -> Self {
            Self {
                directory: Uuid::new_v4().to_string(),
                file: format!("{}.json", Uuid::new_v4()),
            }
        }

        fn is_valid(&self) -> bool {
            if Uuid::parse_str(&self.directory).is_err() {
                return false;
            }

            let Some(file_id) = self.file.strip_suffix(".json") else {
                return false;
            };
            Uuid::parse_str(file_id).is_ok()
        }

        fn credential_path(&self, root: &Path) -> PathBuf {
            root.join(&self.directory).join(&self.file)
        }
    }

    pub(super) fn get(key: &str) -> Option<String> {
        let root = storage_root(key)?;
        let _guard = lock_store();
        match read_secret_from_root(&root) {
            Ok(value) => value,
            Err(error) => {
                // 瞬时 IO 错误（EACCES 等）不再整库删除（会静默丢失登录且不留现场）:
                // 保留数据本次返回 None, 交下次启动重试或人工排查（AU-10, 对齐 Release 侧不销毁）
                log::error!(target: "security", "调试凭据存储读取失败, 已保留现场: {error}");
                None
            }
        }
    }

    pub(super) fn set(key: &str, value: &str) -> bool {
        let Some(root) = storage_root(key) else {
            return false;
        };
        let _guard = lock_store();
        match write_secret_to_root(&root, value) {
            Ok(()) => true,
            Err(error) => {
                log::error!(target: "security", "调试凭据存储写入失败: {error}");
                false
            }
        }
    }

    pub(super) fn delete(key: &str) -> bool {
        let Some(root) = storage_root(key) else {
            return false;
        };
        let _guard = lock_store();
        match delete_store_root(&root) {
            Ok(()) => true,
            Err(error) => {
                log::error!(target: "security", "调试凭据存储删除失败: {error}");
                false
            }
        }
    }

    pub(super) fn exists(key: &str) -> bool {
        let Some(root) = storage_root(key) else {
            return false;
        };
        let _guard = lock_store();
        read_location(&root)
            .ok()
            .flatten()
            .is_some_and(|location| location.credential_path(&root).is_file())
    }

    fn storage_root(key: &str) -> Option<PathBuf> {
        let directory = match key {
            AUTH_STATE_KEY => AUTH_STORE_DIRECTORY,
            GITHUB_TOKEN_KEY => GITHUB_STORE_DIRECTORY,
            WEBDAV_PASSWORD_KEY => WEBDAV_STORE_DIRECTORY,
            _ => return None,
        };
        dirs_next::cache_dir().map(|path| path.join(SERVICE_NAME).join(directory))
    }

    fn lock_store() -> MutexGuard<'static, ()> {
        STORE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn read_secret_from_root(root: &Path) -> io::Result<Option<String>> {
        let Some(location) = read_location(root)? else {
            return Ok(None);
        };
        match fs::read_to_string(location.credential_path(root)) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                delete_store_root(root)?;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn write_secret_to_root(root: &Path, value: &str) -> io::Result<()> {
        let location = match read_location(root) {
            Ok(Some(location)) => location,
            Ok(None) => {
                delete_store_root(root)?;
                StorageLocation::new()
            }
            Err(_) => {
                delete_store_root(root)?;
                StorageLocation::new()
            }
        };

        let credential_path = location.credential_path(root);
        let credential_directory = credential_path
            .parent()
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "调试凭据路径缺少父目录"))?;
        create_private_directory(root)?;
        create_private_directory(credential_directory)?;
        write_private_file(&credential_path, value.as_bytes())?;

        let serialized_location = serde_json::to_vec(&location)
            .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
        if let Err(error) = write_private_file(&root.join(LOCATION_FILE), &serialized_location) {
            let _ = delete_store_root(root);
            return Err(error);
        }
        Ok(())
    }

    fn read_location(root: &Path) -> io::Result<Option<StorageLocation>> {
        let bytes = match fs::read(root.join(LOCATION_FILE)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let location: StorageLocation = serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
        if !location.is_valid() {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "调试凭据位置元数据无效",
            ));
        }
        Ok(Some(location))
    }

    fn create_private_directory(path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    fn write_private_file(path: &Path, contents: &[u8]) -> io::Result<()> {
        // 走仓库统一的原子写（temp + fsync + rename + 0600）, 避免半截写坏凭据文件（AU-10）
        crate::fsutil::atomic_write_private(path, contents)
    }

    fn delete_store_root(root: &Path) -> io::Result<()> {
        match fs::remove_dir_all(root) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn stores_cookie_in_random_directory_and_file() {
            let temp = tempfile::tempdir().expect("create temp directory");
            let root = temp.path().join(AUTH_STORE_DIRECTORY);

            write_secret_to_root(&root, "cookie-json").expect("write debug cookie");

            let location = read_location(&root)
                .expect("read location")
                .expect("location exists");
            assert!(Uuid::parse_str(&location.directory).is_ok());
            let file_id = location
                .file
                .strip_suffix(".json")
                .expect("random cookie file suffix");
            assert!(Uuid::parse_str(file_id).is_ok());
            assert_eq!(
                read_secret_from_root(&root).expect("read debug cookie"),
                Some("cookie-json".to_string())
            );
        }

        #[test]
        fn deleting_cookie_removes_random_storage() {
            let temp = tempfile::tempdir().expect("create temp directory");
            let root = temp.path().join(AUTH_STORE_DIRECTORY);
            write_secret_to_root(&root, "cookie-json").expect("write debug cookie");

            delete_store_root(&root).expect("delete debug cookie storage");

            assert!(!root.exists());
            assert_eq!(
                read_secret_from_root(&root).expect("read deleted debug cookie"),
                None
            );
        }

        #[test]
        fn rejects_location_outside_debug_storage() {
            let temp = tempfile::tempdir().expect("create temp directory");
            let root = temp.path().join(AUTH_STORE_DIRECTORY);
            let outside = temp.path().join("outside.json");
            fs::create_dir_all(&root).expect("create debug storage root");
            fs::write(&outside, "do-not-read").expect("write outside file");
            fs::write(
                root.join(LOCATION_FILE),
                r#"{"directory":"..","file":"outside.json"}"#,
            )
            .expect("write invalid location");

            let error = read_secret_from_root(&root).expect_err("reject invalid location");

            assert_eq!(error.kind(), ErrorKind::InvalidData);
            assert_eq!(
                fs::read_to_string(outside).expect("outside file remains"),
                "do-not-read"
            );
        }

        #[test]
        fn deleting_cookie_storage_keeps_sync_credentials() {
            let temp = tempfile::tempdir().expect("create temp directory");
            let auth_root = temp.path().join(AUTH_STORE_DIRECTORY);
            let github_root = temp.path().join(GITHUB_STORE_DIRECTORY);
            write_secret_to_root(&auth_root, "cookie-json").expect("write debug cookie");
            write_secret_to_root(&github_root, "github-token").expect("write debug token");

            delete_store_root(&auth_root).expect("delete debug cookie storage");

            assert!(!auth_root.exists());
            assert_eq!(
                read_secret_from_root(&github_root).expect("read debug token"),
                Some("github-token".to_string())
            );
        }

        #[test]
        fn known_secrets_use_independent_debug_directories() {
            let auth_root = storage_root(AUTH_STATE_KEY).expect("auth storage root");
            let github_root = storage_root(GITHUB_TOKEN_KEY).expect("GitHub storage root");
            let webdav_root = storage_root(WEBDAV_PASSWORD_KEY).expect("WebDAV storage root");

            assert_ne!(auth_root, github_root);
            assert_ne!(auth_root, webdav_root);
            assert_ne!(github_root, webdav_root);
            assert!(storage_root("unknown-secret").is_none());
        }
    }
}
