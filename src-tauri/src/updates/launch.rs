//! Launching the signature-verified NSIS installer as an independent process.
//!
//! tauri-plugin-updater 2.12.0 hands off to the installer through
//! `ShellExecuteW` and immediately calls `std::process::exit(0)`, so the app
//! never performs its own shutdown: the running process is hard-killed by the
//! NSIS bundle's built-in app check, losing the graceful teardown. This
//! module replicates the plugin's handoff *without* the forced exit:
//!
//! 1. [`prepare_installer_file`] stages the verified bytes into the same temp
//!    layout the plugin uses (`%TEMP%\{app}-{version}-updater-{random}\{app}-{version}-installer.exe`);
//! 2. [`nsis_parameters`] builds the exact parameter string of the plugin's
//!    `Update::updater_parameters` — install-mode switch, `/UPDATE`, `/R
//!    /ARGS` with the plugin's NSIS argument escaping and the custom
//!    `installerArgs` tail;
//! 3. [`launch_nsis_installer`] starts the installer through the same
//!    `ShellExecuteW` call, so a per-machine installer still raises its UAC
//!    prompt and a declined prompt is an ordinary launch error.
//!
//! The caller owns the rest of the flow: only a *successful* launch proceeds
//! into the graceful shutdown ([`crate::shutdown::coordinate_shutdown`]),
//! while any earlier failure leaves the app fully alive for a retry from the
//! modal. On the installer side the PREINSTALL hook in
//! `src-tauri/windows/installer-hooks.nsh` (wired through
//! `bundle > windows > nsis > installerHooks` in `tauri.conf.json`) waits for
//! the installed exe to be released before the template's
//! `CheckIfAppIsRunning` would kill the process, giving the graceful
//! shutdown a bounded window.
//!
//! The launcher compares the staged file with the signature-verified bytes
//! while holding a handle that denies writes and deletion through launch.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Install mode from `plugins > updater > windows > installMode` in
/// `tauri.conf.json`, with the plugin's default (`passive`) for a missing or
/// unknown value. Recognized values: `passive`, `quiet`, `basicUi`.
fn updater_install_mode(config: &tauri::Config) -> &str {
    config
        .plugins
        .0
        .get("updater")
        .and_then(|updater| updater.get("windows"))
        .and_then(|windows| windows.get("installMode"))
        .and_then(|mode| mode.as_str())
        .unwrap_or("passive")
}

/// NSIS switch for the install mode — the plugin's
/// `WindowsUpdateInstallMode::nsis_args` (config.rs): `/P` passive, `/S`
/// silent, basicUi has no switch. Unknown values fall through to the plugin
/// default (passive).
fn nsis_install_mode_args(install_mode: &str) -> &'static [&'static str] {
    match install_mode {
        "quiet" => &["/S"],
        "basicUi" => &[],
        _ => &["/P"],
    }
}

/// Custom installer arguments from `plugins > updater > windows >
/// installerArgs`. The plugin appends them after its own flags
/// (`WindowsConfig::installer_args`), so the handoff must keep them.
fn updater_installer_args(config: &tauri::Config) -> Vec<OsString> {
    config
        .plugins
        .0
        .get("updater")
        .and_then(|updater| updater.get("windows"))
        .and_then(|windows| windows.get("installerArgs"))
        .and_then(|args| args.as_array())
        .map(|args| {
            args.iter()
                .filter_map(|value| value.as_str().map(OsString::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Build the NSIS argument list for the installer handoff — the exact order
/// of the plugin's `Update::updater_parameters` for the NSIS branch:
///
/// `{mode switch} /UPDATE /R /ARGS {escaped current exe args} {installerArgs}`
///
/// `restart_after_install` is the plugin's builder default (`true`); there is
/// no config key for it, and the plugin default is what the update flow has
/// always used, so `/R /ARGS` is unconditional here and the new version
/// relaunches after a successful install.
pub fn nsis_parameters(config: &tauri::Config, current_exe_args: &[OsString]) -> Vec<OsString> {
    let mut arguments: Vec<OsString> = nsis_install_mode_args(updater_install_mode(config))
        .iter()
        .map(OsString::from)
        .collect();
    arguments.push(OsString::from("/UPDATE"));
    arguments.push(OsString::from("/R"));
    arguments.push(OsString::from("/ARGS"));
    arguments.extend(current_exe_args.iter().map(escape_nsis_current_exe_arg));
    arguments.extend(updater_installer_args(config));
    arguments
}

/// Space-join the argument list into the single parameter string
/// `ShellExecuteW` receives (`updater_parameters` joins the same way).
pub fn join_parameters(arguments: &[OsString]) -> OsString {
    let mut joined = OsString::new();
    for (index, argument) in arguments.iter().enumerate() {
        if index > 0 {
            joined.push(" ");
        }
        joined.push(argument);
    }
    joined
}

/// Escape one argument of the running executable for the NSIS `/ARGS` list.
///
/// Verbatim port of the plugin's private `escape_nsis_current_exe_arg`
/// (updater.rs, adapted from the std Windows argv quoter): quote when the
/// argument is empty or contains whitespace or `/` (NSIS would treat a bare
/// `/` as its own switch), backslash-escape inner quotes and trailing
/// backslashes per the CRT rules.
fn escape_nsis_current_exe_arg(arg: impl AsRef<OsStr>) -> OsString {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    let arg = arg.as_ref();
    let mut cmd: Vec<u16> = Vec::new();

    let quote = arg
        .as_encoded_bytes()
        .iter()
        .any(|c| *c == b' ' || *c == b'\t' || *c == b'/')
        || arg.is_empty();
    if quote {
        cmd.push('"' as u16);
    }
    let mut backslashes: usize = 0;
    for x in arg.encode_wide() {
        if x == '\\' as u16 {
            backslashes += 1;
        } else {
            if x == '"' as u16 {
                // Add n+1 backslashes to total 2n+1 before an inner '"'.
                cmd.extend((0..=backslashes).map(|_| '\\' as u16));
            }
            backslashes = 0;
        }
        cmd.push(x);
    }
    if quote {
        // Add n backslashes to total 2n before the closing '"'.
        cmd.extend((0..backslashes).map(|_| '\\' as u16));
        cmd.push('"' as u16);
    }
    OsString::from_wide(&cmd)
}

/// Stage the verified installer bytes for the launch.
///
/// Mirrors the plugin's staging (`Update::write_to_temp` + `make_temp_dir`):
/// a unique directory per attempt under `%TEMP%` and the installer written
/// once, in full, from the already-verified bytes. The directory is *not*
/// deleted on success — the launched installer runs from this file while the
/// app exits; on a launch failure [`discard_staged_installer`] cleans it up.
pub fn prepare_installer_file(
    app_name: &str,
    version: &str,
    bytes: &[u8],
) -> std::io::Result<PathBuf> {
    use std::io::Write;

    let dir = std::env::temp_dir().join(format!(
        "{app_name}-{version}-updater-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir(&dir)?;
    let path = dir.join(format!("{app_name}-{version}-installer.exe"));
    if let Err(error) = std::fs::File::create_new(&path).and_then(|mut file| file.write_all(bytes))
    {
        discard_staged_installer(&path);
        return Err(error);
    }
    Ok(path)
}

/// Best-effort cleanup of a staged installer that never launched (the plugin
/// deletes its `TempPath` on the same failure path). The file may already be
/// gone or locked — both are ignored.
pub fn discard_staged_installer(path: &Path) {
    let _ = std::fs::remove_file(path);
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir(dir);
    }
}

/// Launch the staged NSIS installer as an independent process.
///
/// Same handoff as the plugin's `install_inner`: `ShellExecuteW` with the
/// `open` verb and `SW_SHOW` — a per-machine installer raises its UAC prompt
/// and a declined prompt (or any other start failure, shell result code
/// ≤ 32) comes back as an error while the app stays alive.
pub fn launch_nsis_installer(
    installer: &Path,
    parameters: &OsStr,
    verified_bytes: &[u8],
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;

    /// Null-terminated UTF-16 for the `PCWSTR` parameters.
    fn encode_wide(value: impl AsRef<OsStr>) -> Vec<u16> {
        value.as_ref().encode_wide().chain(Some(0)).collect()
    }

    let verified_file = lock_verified_installer(installer, verified_bytes)?;
    let operation = encode_wide("open");
    let file = encode_wide(installer.as_os_str());
    let parameters = encode_wide(parameters);

    let result = unsafe {
        ShellExecuteW(
            HWND::default(),
            PCWSTR::from_raw(operation.as_ptr()),
            PCWSTR::from_raw(file.as_ptr()),
            PCWSTR::from_raw(parameters.as_ptr()),
            PCWSTR::null(),
            SW_SHOW,
        )
    };
    if (result.0 as isize) <= 32 {
        return Err(std::io::Error::last_os_error());
    }
    drop(verified_file);
    Ok(())
}

fn lock_verified_installer(path: &Path, expected: &[u8]) -> std::io::Result<std::fs::File> {
    use std::io::{Error, ErrorKind, Read};
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 1;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)?;
    let changed = || {
        Error::new(
            ErrorKind::InvalidData,
            "staged installer differs from verified download",
        )
    };
    if file.metadata()?.len() != expected.len() as u64 {
        return Err(changed());
    }
    let mut buffer = [0; 8192];
    for chunk in expected.chunks(buffer.len()) {
        let actual = &mut buffer[..chunk.len()];
        file.read_exact(actual)?;
        if actual != chunk {
            return Err(changed());
        }
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `tauri::Config` with the given `plugins.updater` object (same
    /// construction as the `updater_configured` tests in `install.rs`).
    fn updater_config(updater: serde_json::Value) -> tauri::Config {
        let mut plugins = std::collections::HashMap::new();
        plugins.insert("updater".to_string(), updater);
        tauri::Config {
            plugins: tauri::utils::config::PluginConfig(plugins),
            ..Default::default()
        }
    }

    // ── install mode ──

    #[test]
    fn install_mode_reads_config_with_plugin_default() {
        let missing = updater_config(serde_json::json!({ "pubkey": "k" }));
        assert_eq!(updater_install_mode(&missing), "passive");

        let passive = updater_config(serde_json::json!({
            "pubkey": "k",
            "windows": { "installMode": "passive" },
        }));
        assert_eq!(updater_install_mode(&passive), "passive");

        let quiet = updater_config(serde_json::json!({
            "pubkey": "k",
            "windows": { "installMode": "quiet" },
        }));
        assert_eq!(updater_install_mode(&quiet), "quiet");

        let basic_ui = updater_config(serde_json::json!({
            "pubkey": "k",
            "windows": { "installMode": "basicUi" },
        }));
        assert_eq!(updater_install_mode(&basic_ui), "basicUi");

        let unknown = updater_config(serde_json::json!({
            "pubkey": "k",
            "windows": { "installMode": "mystery" },
        }));
        assert_eq!(updater_install_mode(&unknown), "mystery");
        assert_eq!(
            nsis_install_mode_args(updater_install_mode(&unknown)),
            nsis_install_mode_args("passive"),
            "unknown mode falls through to the plugin default"
        );
    }

    // ── parameters ──

    /// The full NSIS parameter order must match the plugin's
    /// `updater_parameters`: mode switch, `/UPDATE`, `/R /ARGS` with escaped
    /// current-exe args, then the custom `installerArgs` tail.
    #[test]
    fn nsis_parameters_mirror_plugin_order() {
        let config = updater_config(serde_json::json!({
            "pubkey": "k",
            "windows": {
                "installMode": "passive",
                "installerArgs": ["--lang=ru"],
            },
        }));
        let arguments = nsis_parameters(
            &config,
            &[
                r"C:\Program Files\TTSBard\ttsbard.exe".into(),
                "--flag".into(),
            ],
        );
        let expected: Vec<OsString> = [
            "/P",
            "/UPDATE",
            "/R",
            "/ARGS",
            // A path with spaces is quoted; single backslashes pass through
            // (the quoter only doubles backslashes before an inner quote).
            "\"C:\\Program Files\\TTSBard\\ttsbard.exe\"",
            "--flag",
            "--lang=ru",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        assert_eq!(arguments, expected);
        assert_eq!(
            join_parameters(&arguments).to_string_lossy(),
            r#"/P /UPDATE /R /ARGS "C:\Program Files\TTSBard\ttsbard.exe" --flag --lang=ru"#
        );
    }

    #[test]
    fn nsis_parameters_cover_all_install_modes() {
        let current_exe_args: Vec<OsString> = Vec::new();

        let quiet = updater_config(serde_json::json!({
            "windows": { "installMode": "quiet" },
        }));
        assert_eq!(
            nsis_parameters(&quiet, &current_exe_args),
            vec![
                OsString::from("/S"),
                OsString::from("/UPDATE"),
                OsString::from("/R"),
                OsString::from("/ARGS"),
            ]
        );

        let basic_ui = updater_config(serde_json::json!({
            "windows": { "installMode": "basicUi" },
        }));
        assert_eq!(
            nsis_parameters(&basic_ui, &current_exe_args),
            vec![
                OsString::from("/UPDATE"),
                OsString::from("/R"),
                OsString::from("/ARGS"),
            ]
        );
    }

    // ── escaping (plugin test vectors, updater.rs 2.12.0) ──

    #[test]
    fn escaping_matches_plugin_vectors() {
        let cases = [
            "something",
            "--flag",
            "--empty=",
            "--arg=value",
            "some space",            // `./my-app "some string"`.
            "--arg value",           // `./my-app "--arg value"`.
            "--arg=unwrapped space", // `./my-app --arg="unwrapped space"`.
            "--arg=\"wrapped\"",     // `./my-app --args=""wrapped""`.
            "--arg=\"wrapped space\"",
            "--arg=midword\"wrapped space\"",
            "", // `./my-app '""'`
        ];
        // The plugin pins these exact outputs ("may not be what we actually
        // want — monitor"); our port must not drift from them.
        let cases_escaped = [
            "something",
            "--flag",
            "--empty=",
            "--arg=value",
            "\"some space\"",
            "\"--arg value\"",
            "\"--arg=unwrapped space\"",
            "--arg=\\\"wrapped\\\"",
            "\"--arg=\\\"wrapped space\\\"\"",
            "\"--arg=midword\\\"wrapped space\\\"\"",
            "\"\"",
        ];
        assert_eq!(cases.len(), cases_escaped.len());
        for (orig, escaped) in cases.iter().zip(cases_escaped) {
            assert_eq!(escape_nsis_current_exe_arg(orig).to_string_lossy(), escaped);
        }
    }

    // ── staging ──

    #[test]
    fn prepare_installer_file_writes_verified_bytes_once() {
        let bytes: Vec<u8> = (0..=u8::MAX).cycle().take(4096).collect();
        let first = prepare_installer_file("TTSBard", "9.9.0", &bytes).expect("staging succeeds");
        assert_eq!(std::fs::read(&first).unwrap(), bytes, "byte-for-byte");
        assert_eq!(first.extension().unwrap(), "exe");

        // A second attempt stages a fresh file, never overwriting the first.
        let second =
            prepare_installer_file("TTSBard", "9.9.0", &bytes).expect("second staging succeeds");
        assert_ne!(first, second, "each attempt gets its own directory");

        discard_staged_installer(&first);
        discard_staged_installer(&second);
    }

    /// Failure branch: a staged installer that never launched is cleaned up
    /// (file and its staging directory), mirroring the plugin's `TempPath`
    /// drop on the same path.
    #[test]
    fn discard_staged_installer_removes_file_and_directory() {
        let staged =
            prepare_installer_file("TTSBard", "9.9.1", b"installer").expect("staging succeeds");
        assert!(staged.is_file());
        let dir = staged.parent().unwrap().to_path_buf();

        discard_staged_installer(&staged);
        assert!(!staged.exists(), "staged file removed");
        assert!(!dir.exists(), "staging directory removed");
    }

    #[test]
    fn launch_lock_rejects_changed_bytes_and_blocks_writes() {
        let path = prepare_installer_file("TTSBard", "9.9.2", b"verified").unwrap();
        assert!(lock_verified_installer(&path, b"modified").is_err());
        let guard = lock_verified_installer(&path, b"verified").unwrap();
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
        assert!(std::fs::remove_file(&path).is_err());
        drop(guard);
        discard_staged_installer(&path);
    }
}
