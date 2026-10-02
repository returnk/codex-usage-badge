use serde_json::{json, Value};
const RELEASE_ROOT: &str = "https://github.com/returnk/codex-usage-badge/releases/tag/";

// An installed copy and a portable copy can coexist: only the exact registered
// executable may launch NSIS. Folder names and uninstall.exe alone are not proof.
pub fn is_registered_install() -> bool {
    use winreg::{
        enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WOW64_64KEY},
        RegKey,
    };
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = root.open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Codex Badge",
        KEY_READ | KEY_WOW64_64KEY,
    ) else {
        return false;
    };
    let Ok(location) = key.get_value::<String, _>("InstallLocation") else {
        return false;
    };
    matches_install_path(&exe, &location)
}
fn matches_install_path(exe: &std::path::Path, location: &str) -> bool {
    let folder = std::path::PathBuf::from(location.trim_matches('"'));
    if !folder.join("uninstall.exe").is_file() {
        return false;
    }
    let expected = folder.join("codex-badge-tauri.exe");
    match (exe.canonicalize(), expected.canonicalize()) {
        (Ok(actual), Ok(expected)) => actual
            .to_string_lossy()
            .eq_ignore_ascii_case(&expected.to_string_lossy()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "read-only validation of locally built signed candidate, no installation"]
    fn signed_candidate_verifies_and_rejects_tampering() {
        use base64::Engine;
        let folder = std::path::PathBuf::from(
            std::env::var_os("BADGE_CANDIDATE_DIR").expect("candidate folder"),
        );
        let bytes = std::fs::read(folder.join("latest.json")).unwrap();
        assert_eq!(bytes[0], b'{', "manifest must have no BOM");
        let manifest: Value = serde_json::from_slice(&bytes).unwrap();
        let config: Value =
            serde_json::from_str(include_str!("../tauri.conf.json").trim_start_matches('\u{feff}'))
                .unwrap();
        let decode = |text: &str| {
            String::from_utf8(
                base64::engine::general_purpose::STANDARD
                    .decode(text.trim())
                    .unwrap(),
            )
            .unwrap()
        };
        let key = minisign_verify::PublicKey::decode(&decode(
            config["plugins"]["updater"]["pubkey"].as_str().unwrap(),
        ))
        .unwrap();
        let signature = minisign_verify::Signature::decode(&decode(
            manifest["platforms"]["windows-x86_64"]["signature"]
                .as_str()
                .unwrap(),
        ))
        .unwrap();
        assert!(signature
            .trusted_comment()
            .split('\t')
            .any(|part| part == format!("version:{}", manifest["version"].as_str().unwrap())));
        let name = format!(
            "Codex Badge_{}_x64-setup.exe",
            manifest["version"].as_str().unwrap()
        );
        let mut installer = std::fs::read(folder.join(name)).unwrap();
        key.verify(&installer, &signature, false).unwrap();
        installer[128] ^= 1;
        assert!(
            key.verify(&installer, &signature, false).is_err(),
            "tampered installer must fail validation"
        );
    }
    #[test]
    fn portable_copy_never_matches_the_installed_executable() {
        let base =
            std::env::temp_dir().join(format!("badge-install-path-test-{}", std::process::id()));
        let installed = base.join("installed");
        let portable = base.join("portable");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::create_dir_all(&portable).unwrap();
        for folder in [&installed, &portable] {
            std::fs::write(
                folder.join("codex-badge-tauri.exe"),
                b"test fixture, never run",
            )
            .unwrap();
            std::fs::write(folder.join("uninstall.exe"), b"test fixture, never run").unwrap();
        }
        let location = format!("\"{}\"", installed.display());
        assert!(matches_install_path(
            &installed.join("codex-badge-tauri.exe"),
            &location
        ));
        assert!(!matches_install_path(
            &portable.join("codex-badge-tauri.exe"),
            &location
        ));
        std::fs::remove_file(installed.join("uninstall.exe")).unwrap();
        assert!(!matches_install_path(
            &installed.join("codex-badge-tauri.exe"),
            &location
        ));
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn installed_version_clears_its_persisted_notice() {
        let file = std::env::temp_dir().join(format!(
            "badge-update-notice-test-{}.json",
            std::process::id()
        ));
        std::fs::write(&file, br#""0.3.3""#).unwrap();
        assert_eq!(load_notice_at(&file, "0.3.2").as_deref(), Some("0.3.3"));
        assert_eq!(load_notice_at(&file, "0.3.3"), None);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "null");
        std::fs::remove_file(file).unwrap();
    }
    #[test]
    fn stored_notice_accepts_only_a_newer_valid_version() {
        assert_eq!(
            decode_notice(br#""0.3.3""#, "0.3.2").as_deref(),
            Some("0.3.3")
        );
        for bytes in [
            br#""0.3.2""#.as_slice(),
            b"null",
            b"corrupt",
            br#""https://other.test""#,
        ] {
            assert!(decode_notice(bytes, "0.3.2").is_none());
        }
    }
    #[test]
    fn update_notice_survives_failure_and_clears_after_success_or_install() {
        let mut known = None;
        let next = Ok(json!({"version":"0.3.3","available":true}));
        remember_release(&mut known, &next, "0.3.2");
        assert!(has_update_notice(known.as_deref(), "0.3.2"));
        remember_release(&mut known, &Err("网络失败".into()), "0.3.2");
        assert_eq!(known.as_deref(), Some("0.3.3"));
        assert!(!has_update_notice(known.as_deref(), "0.3.3"));
        remember_release(
            &mut known,
            &Ok(json!({"version":"0.3.2","available":false})),
            "0.3.2",
        );
        assert_eq!(known, None);
        assert!(!has_update_notice(Some("invalid"), "0.3.2"));
    }
    #[test]
    #[ignore = "manual read-only GitHub release query"]
    fn live_release_readonly() {
        let release = release_state(&fetch_latest().unwrap(), env!("CARGO_PKG_VERSION")).unwrap();
        println!(
            "release_readonly current={} latest={} available={}",
            release["currentVersion"], release["version"], release["available"]
        );
    }
    #[test]
    fn release_versions_compare_numerically_and_untrusted_links_are_rejected() {
        let release = json!({"tag_name":"v0.3.10","html_url":format!("{RELEASE_ROOT}v0.3.10"),"body":"修复说明","draft":false,"prerelease":false});
        let result = release_state(&release, "0.3.9").unwrap();
        assert_eq!(result["available"], true);
        assert_eq!(result["notes"], "修复说明");
        assert_eq!(
            release_state(&release, "0.3.10").unwrap()["available"],
            false
        );
        assert_eq!(
            release_state(&release, "0.4.0").unwrap()["available"],
            false
        );
        for patch in [
            json!({"html_url":"https://evil.test"}),
            json!({"prerelease":true}),
            json!({"tag_name":"v0.3.11-beta.1"}),
        ] {
            let mut bad = release.clone();
            for (k, v) in patch.as_object().unwrap() {
                bad[k] = v.clone();
            }
            assert!(release_state(&bad, "0.3.9").is_err());
        }
    }
}

fn version(value: &str) -> Option<[u64; 3]> {
    let parts: Vec<_> = value
        .strip_prefix('v')
        .unwrap_or(value)
        .split('.')
        .collect();
    if parts.len() != 3 {
        return None;
    }
    let mut result = [0; 3];
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|v| v.is_ascii_digit()) {
            return None;
        }
        result[index] = part.parse().ok()?;
    }
    Some(result)
}

fn release_state(release: &Value, current: &str) -> Result<Value, String> {
    let tag = release
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or("更新响应缺少版本")?;
    let next = version(tag).ok_or("更新版本格式无法识别")?;
    let installed = version(current).ok_or("当前版本格式无法识别")?;
    let url = release
        .get("html_url")
        .and_then(Value::as_str)
        .ok_or("更新响应缺少发布页")?;
    if release.get("draft").and_then(Value::as_bool) != Some(false)
        || release.get("prerelease").and_then(Value::as_bool) != Some(false)
        || url != format!("{RELEASE_ROOT}{tag}")
    {
        return Err("更新响应不是有效的正式发布".into());
    }
    Ok(
        json!({"currentVersion":current,"version":tag.trim_start_matches('v'),"tag":tag,
        "available":next>installed,"notes":release.get("body").and_then(Value::as_str).unwrap_or("暂无更新说明"),"url":url}),
    )
}

// Native HTTPS, OS proxy/certificate handling; no account data or credentials are sent.
fn fetch_latest() -> Result<Value, String> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Networking::WinHttp::*;
    struct Handle(*mut std::ffi::c_void);
    impl Handle {
        fn new(value: *mut std::ffi::c_void) -> Result<Self, String> {
            if value.is_null() {
                Err("无法连接更新服务，请稍后重试".into())
            } else {
                Ok(Self(value))
            }
        }
    }
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
    let network_error = |_| "检查更新失败，请检查网络后重试".to_string();
    unsafe {
        let session = Handle::new(WinHttpOpen(
            w!("CodexBadge/manual-update"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, 5000, 5000, 5000, 10000).map_err(network_error)?;
        let connection = Handle::new(WinHttpConnect(session.0, w!("api.github.com"), 443, 0))?;
        let request = Handle::new(WinHttpOpenRequest(
            connection.0,
            w!("GET"),
            w!("/repos/returnk/codex-usage-badge/releases/latest"),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        let headers: Vec<u16> =
            "Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n"
                .encode_utf16()
                .collect();
        WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0).map_err(network_error)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(network_error)?;
        let mut status = 0u32;
        let mut length = 4;
        let mut index = 0;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut length,
            &mut index,
        )
        .map_err(network_error)?;
        match status {
            200 => {}
            403 | 429 => return Err("更新服务请求过多，请稍后重试".into()),
            404 => return Err("暂未找到正式发布".into()),
            _ => return Err("更新服务暂不可用，请稍后重试".into()),
        }
        let mut bytes = Vec::new();
        let started = std::time::Instant::now();
        loop {
            let mut buffer = [0u8; 8192];
            let mut read = 0;
            WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut read,
            )
            .map_err(network_error)?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read as usize]);
            if bytes.len() > 512 * 1024 || started.elapsed().as_secs() > 20 {
                return Err("更新响应过大或超时，请稍后重试".into());
            }
        }
        serde_json::from_slice(&bytes).map_err(|_| "更新响应无法识别".into())
    }
}

#[tauri::command]
pub fn open_release(tag: String) -> Result<(), String> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    version(&tag).ok_or("版本格式无法识别")?;
    let url: Vec<u16> = format!("{RELEASE_ROOT}{tag}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err("无法打开发布页".into())
    }
}

pub fn latest() -> Result<Value, String> {
    release_state(&fetch_latest()?, env!("CARGO_PKG_VERSION"))
}
pub fn has_update_notice(known: Option<&str>, current: &str) -> bool {
    known
        .and_then(version)
        .zip(version(current))
        .is_some_and(|(next, current)| next > current)
}
pub fn remember_release(known: &mut Option<String>, result: &Result<Value, String>, current: &str) {
    if let Ok(release) = result {
        *known = release["version"]
            .as_str()
            .filter(|next| release["available"] == true && has_update_notice(Some(next), current))
            .map(str::to_owned);
    }
}

fn notice_path() -> Option<std::path::PathBuf> {
    Some(
        std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
            .join("CodexBadge/tauri-update-notice.json"),
    )
}
pub fn load_notice() -> Option<String> {
    load_notice_at(&notice_path()?, env!("CARGO_PKG_VERSION"))
}
fn load_notice_at(path: &std::path::Path, current: &str) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() > 256 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let known = decode_notice(&bytes, current);
    if known.is_none() {
        if let Ok(Some(previous)) = serde_json::from_slice::<Option<String>>(&bytes) {
            if version(&previous).is_some() {
                if let Err(error) = save_notice_at(path, &None) {
                    crate::diagnostics::record(format!("update_notice_clear_failed {error}"));
                }
            }
        }
    }
    known
}
fn decode_notice(bytes: &[u8], current: &str) -> Option<String> {
    if bytes.len() > 256 {
        return None;
    }
    serde_json::from_slice::<Option<String>>(bytes)
        .ok()
        .flatten()
        .filter(|v| has_update_notice(Some(v), current))
}
pub fn save_notice(known: &Option<String>) -> Result<(), String> {
    save_notice_at(&notice_path().ok_or("LOCALAPPDATA missing")?, known)
}
fn save_notice_at(path: &std::path::Path, known: &Option<String>) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().ok_or("update cache directory missing")?)
        .map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec(known).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}
