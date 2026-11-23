mod artifact;
mod download;

use crate::common::do_check_software_update;
use artifact::verified_update_artifact_from_release_page_url;
pub(crate) use artifact::{current_update_arch, current_update_format, current_update_platform};
pub use artifact::{
    current_update_artifact_query, get_update_download_file_from_url,
    verified_update_artifact_for_download_url, verified_update_artifact_for_release_page_url,
};
use base::config::keys;
use download::ensure_verified_update_artifact;
pub(crate) use download::remove_update_file;
use hbb_common::{bail, config, log, ResultType};
use std::{
    path::Path,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
    time::{Duration, Instant},
};

#[cfg(not(all(
    feature = "flutter",
    not(any(target_os = "android", target_os = "ios"))
)))]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(target_os = "windows")]
use std::path::PathBuf;

#[cfg(target_os = "macos")]
use std::os::{
    fd::AsRawFd,
    unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
};

#[cfg(target_os = "macos")]
struct MacUpdateLock {
    _file: std::fs::File,
}

#[cfg(target_os = "macos")]
fn acquire_mac_update_lock() -> ResultType<MacUpdateLock> {
    let path = std::path::PathBuf::from("/var/run/capylink-update.lock");
    let handle = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .custom_flags(hbb_common::libc::O_NOFOLLOW | hbb_common::libc::O_CLOEXEC)
        .open(&path)?;
    let metadata = handle.metadata()?;
    if !metadata.file_type().is_file() || metadata.uid() != 0 {
        bail!("[root-update] update lock is not a root-owned regular file");
    }
    handle.set_permissions(std::fs::Permissions::from_mode(0o600))?;

    // Keep the descriptor open through update preparation and detached-script
    // launch. O_CLOEXEC means this lock does not cover the detached bundle
    // swap; flock is released when this guard is dropped or the process exits.
    let lock_result = unsafe {
        hbb_common::libc::flock(
            handle.as_raw_fd(),
            hbb_common::libc::LOCK_EX | hbb_common::libc::LOCK_NB,
        )
    };
    if lock_result != 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::WouldBlock {
            bail!("[root-update] another update is already running");
        }
        return Err(err.into());
    }
    Ok(MacUpdateLock { _file: handle })
}

enum UpdateMsg {
    CheckUpdate,
    Exit,
}

lazy_static::lazy_static! {
    static ref TX_MSG : Mutex<Sender<UpdateMsg>> = Mutex::new(start_auto_update_check());
}

#[cfg(not(all(
    feature = "flutter",
    not(any(target_os = "android", target_os = "ios"))
)))]
static CONTROLLING_SESSION_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Initial wait after startup before the first update check (30 seconds).
pub const INITIAL_CHECK_DELAY: Duration = Duration::from_secs(30);

/// One full day — default interval between update checks.
pub const DUR_ONE_DAY: Duration = Duration::from_secs(60 * 60 * 24);

/// Minimum interval between consecutive update checks (10 minutes).
pub const MIN_INTERVAL: Duration = Duration::from_secs(60 * 10);

/// Retry interval when an update check fails or a session is active (30 minutes).
pub const RETRY_INTERVAL: Duration = Duration::from_secs(60 * 30);

#[cfg(all(
    feature = "flutter",
    not(any(target_os = "android", target_os = "ios"))
))]
pub fn update_controlling_session_count(pid: Option<u32>, count: usize) -> ResultType<()> {
    controlling_sessions::record(pid, count)
}

#[allow(dead_code)]
pub fn start_auto_update() {
    let _sender = TX_MSG.lock().unwrap();
}

#[allow(dead_code)]
pub fn manually_check_update() -> ResultType<()> {
    let sender = TX_MSG.lock().unwrap();
    sender.send(UpdateMsg::CheckUpdate)?;
    Ok(())
}

#[allow(dead_code)]
pub fn stop_auto_update() {
    let sender = TX_MSG.lock().unwrap();
    sender.send(UpdateMsg::Exit).unwrap_or_default();
}

#[inline]
/// Returns true when there are no active incoming or outgoing connections.
/// Used to avoid updating while a remote session is in progress.
pub fn has_no_active_conns() -> bool {
    let conns = crate::Connection::alive_conns();
    conns.is_empty() && has_no_controlling_conns()
}

#[cfg(any(not(target_os = "windows"), feature = "flutter"))]
fn has_no_controlling_conns() -> bool {
    #[cfg(all(
        feature = "flutter",
        not(any(target_os = "android", target_os = "ios"))
    ))]
    return controlling_sessions::is_idle();
    #[cfg(not(all(
        feature = "flutter",
        not(any(target_os = "android", target_os = "ios"))
    )))]
    {
        CONTROLLING_SESSION_COUNT.load(Ordering::SeqCst) == 0
    }
}

#[cfg(all(
    feature = "flutter",
    not(any(target_os = "android", target_os = "ios"))
))]
mod controlling_sessions {
    use hbb_common::{
        anyhow::anyhow,
        bail,
        sysinfo::{Pid, Process, ProcessRefreshKind, ProcessStatus, System},
        ResultType,
    };
    use std::{collections::HashMap, path::Path, sync::Mutex};

    struct Count {
        start_time: u64,
        sessions: usize,
    }

    lazy_static::lazy_static! {
        static ref COUNTS: Mutex<HashMap<Pid, Count>> = Default::default();
    }

    fn gui_command(command: &[String]) -> bool {
        let args = &command[command.len().min(1)..];
        if args.iter().any(|arg| {
            matches!(
                arg.as_str(),
                "--connect"
                    | "--play"
                    | "--file-transfer"
                    | "--view-camera"
                    | "--port-forward"
                    | "--terminal"
                    | "--terminal-admin"
                    | "--rdp"
                    | "--noinstall"
            )
        }) {
            return true;
        }
        let argument = args.iter().find(|arg| {
            !matches!(
                arg.as_str(),
                "--no-server" | "--quick_support" | "--elevate" | "--run-as-system"
            )
        });
        match argument.map(String::as_str) {
            Some(arg) if arg.starts_with("XPC_SERVICE_NAME=") && arg.ends_with("_server") => false,
            Some(
                "--server"
                | "--service"
                | "--tray"
                | "--cm"
                | "--update"
                | "--install"
                | "--silent-install"
                | "--uninstall"
                | "--remove"
                | "--install-service"
                | "--uninstall-service"
                | "--after-install"
                | "--before-uninstall"
                | "--uninstall-cert"
                | "--install-idd"
                | "--uninstall-amyuni-idd"
                | "--install-remote-printer"
                | "--uninstall-remote-printer"
                | "--portable-service"
                | "--option"
                | "--config"
                | "--assign"
                | "--deploy"
                | "--password"
                | "--set-unlock-pin"
                | "--get-id"
                | "--set-id"
                | "--import-config"
                | "--check-hwcodec-config"
                | "--terminal-helper"
                | "--whiteboard"
                | "--version"
                | "--build-date"
                | "-gtk-sudo",
            ) => false,
            _ => true,
        }
    }

    fn gui_process(
        process: &Process,
        current: &Process,
        executable: &Path,
        registered: bool,
    ) -> ResultType<bool> {
        if process.exe().as_os_str().is_empty() {
            if registered
                || executable
                    .file_name()
                    .is_some_and(|name| name == process.name())
                || executable
                    .file_stem()
                    .is_some_and(|name| name == process.name())
            {
                bail!("GUI process executable is unavailable");
            }
            return Ok(false);
        }
        #[cfg(windows)]
        let same_executable = process
            .exe()
            .to_string_lossy()
            .eq_ignore_ascii_case(&executable.to_string_lossy());
        #[cfg(not(windows))]
        let same_executable = process.exe() == executable;
        if !same_executable {
            return Ok(false);
        }
        #[cfg(unix)]
        {
            let uid = current
                .user_id()
                .ok_or_else(|| anyhow!("Server process owner is unavailable"))?;
            if process
                .user_id()
                .ok_or_else(|| anyhow!("GUI process owner is unavailable"))?
                != uid
            {
                return Ok(false);
            }
        }
        if process.cmd().is_empty() {
            bail!("GUI process command is unavailable");
        }
        if !registered && !gui_command(process.cmd()) {
            return Ok(false);
        }
        #[cfg(windows)]
        {
            let session =
                crate::platform::windows::get_session_id_of_process(current.pid().as_u32())
                    .ok_or_else(|| anyhow!("Server process session is unavailable"))?;
            if crate::platform::windows::get_session_id_of_process(process.pid().as_u32())
                .ok_or_else(|| anyhow!("GUI process session is unavailable"))?
                != session
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn exited(pid: Pid) -> bool {
        #[cfg(unix)]
        {
            (unsafe { hbb_common::libc::kill(pid.as_u32() as _, 0) }) != 0
                && std::io::Error::last_os_error().raw_os_error() == Some(hbb_common::libc::ESRCH)
        }
        #[cfg(windows)]
        {
            use winapi::{
                shared::winerror::ERROR_INVALID_PARAMETER,
                um::{
                    errhandlingapi::GetLastError,
                    handleapi::CloseHandle,
                    minwinbase::STILL_ACTIVE,
                    processthreadsapi::{GetExitCodeProcess, OpenProcess},
                    winnt::PROCESS_QUERY_LIMITED_INFORMATION,
                },
            };
            unsafe {
                let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid.as_u32());
                if handle.is_null() {
                    return GetLastError() == ERROR_INVALID_PARAMETER;
                }
                let mut code = STILL_ACTIVE;
                let exited = GetExitCodeProcess(handle, &mut code) != 0 && code != STILL_ACTIVE;
                CloseHandle(handle);
                exited
            }
        }
    }

    pub(super) fn record(pid: Option<u32>, sessions: usize) -> ResultType<()> {
        let pid = Pid::from_u32(pid.ok_or_else(|| anyhow!("IPC peer process is unavailable"))?);
        // Keep a trusted IPC peer busy while process metadata is unavailable.
        COUNTS
            .lock()
            .unwrap()
            .entry(pid)
            .and_modify(|count| {
                if sessions > 0 {
                    count.sessions = sessions;
                }
            })
            .or_insert(Count {
                start_time: 0,
                sessions,
            });
        let mut system = System::new();
        #[cfg(unix)]
        let refresh = ProcessRefreshKind::new().with_user();
        #[cfg(windows)]
        let refresh = ProcessRefreshKind::new();
        let own_pid = Pid::from_u32(std::process::id());
        system.refresh_process_specifics(own_pid, refresh);
        system.refresh_process_specifics(pid, refresh);
        let current = system
            .process(own_pid)
            .ok_or_else(|| anyhow!("Server process is unavailable"))?;
        let process = system
            .process(pid)
            .ok_or_else(|| anyhow!("IPC peer process is unavailable"))?;
        let executable = std::env::current_exe()?;
        if !gui_process(process, current, &executable, true)? {
            COUNTS.lock().unwrap().remove(&pid);
            return Ok(());
        }
        if process.start_time() == 0 {
            bail!("GUI process start time is unavailable");
        }
        COUNTS.lock().unwrap().insert(
            pid,
            Count {
                start_time: process.start_time(),
                sessions,
            },
        );
        Ok(())
    }

    fn check_idle() -> ResultType<bool> {
        let mut system = System::new();
        #[cfg(unix)]
        let refresh = ProcessRefreshKind::new().with_user();
        #[cfg(windows)]
        let refresh = ProcessRefreshKind::new();
        system.refresh_processes_specifics(refresh);
        let current = system
            .process(Pid::from_u32(std::process::id()))
            .ok_or_else(|| anyhow!("Process snapshot is unavailable"))?;
        let executable = std::env::current_exe()?;
        let mut counts = COUNTS.lock().unwrap();
        counts.retain(|pid, count| match system.process(*pid) {
            Some(process) => {
                !matches!(
                    process.status(),
                    ProcessStatus::Zombie | ProcessStatus::Dead
                ) && (count.start_time == 0
                    || process.start_time() == 0
                    || process.start_time() == count.start_time)
            }
            None => !exited(*pid),
        });
        for pid in counts.keys() {
            if system.process(*pid).is_none() {
                bail!("Reported GUI process could not be inspected");
            }
        }
        for (pid, process) in system.processes() {
            if matches!(
                process.status(),
                ProcessStatus::Zombie | ProcessStatus::Dead
            ) || !gui_process(process, current, &executable, counts.contains_key(pid))?
            {
                continue;
            }
            if process.start_time() == 0
                || !counts.get(pid).is_some_and(|count| {
                    count.start_time == process.start_time() && count.sessions == 0
                })
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn is_idle() -> bool {
        match check_idle() {
            Ok(idle) => idle,
            Err(error) => {
                hbb_common::throttled_log!(
                    crate::common::API_LOG_INTERVAL,
                    warn,
                    "Active controlling sessions could not be verified: {error}"
                );
                false
            }
        }
    }
}

#[cfg(not(any(not(target_os = "windows"), feature = "flutter")))]
fn has_no_controlling_conns() -> bool {
    let app_exe = format!("{}.exe", crate::get_app_name().to_lowercase());
    for arg in [
        "--connect",
        "--play",
        "--file-transfer",
        "--view-camera",
        "--port-forward",
        "--rdp",
    ] {
        if !crate::platform::get_pids_of_process_with_first_arg(&app_exe, arg).is_empty() {
            return false;
        }
    }
    true
}

fn start_auto_update_check() -> Sender<UpdateMsg> {
    let (tx, rx) = channel();
    std::thread::spawn(move || start_auto_update_check_(rx));
    return tx;
}

fn start_auto_update_check_(rx_msg: Receiver<UpdateMsg>) {
    std::thread::sleep(INITIAL_CHECK_DELAY);
    if let Err(e) = check_update(false) {
        log::error!("Error checking for updates: {}", e);
    }

    let mut last_check_time = Instant::now();
    let mut check_interval = DUR_ONE_DAY;
    loop {
        let recv_res = rx_msg.recv_timeout(check_interval);
        match &recv_res {
            Ok(UpdateMsg::CheckUpdate) | Err(_) => {
                if last_check_time.elapsed() < MIN_INTERVAL {
                    // log::debug!("Update check skipped due to minimum interval.");
                    continue;
                }
                // Don't check update if there are alive connections.
                if !has_no_active_conns() {
                    check_interval = RETRY_INTERVAL;
                    continue;
                }
                if let Err(e) = check_update(matches!(recv_res, Ok(UpdateMsg::CheckUpdate))) {
                    log::error!("Error checking for updates: {}", e);
                    check_interval = RETRY_INTERVAL;
                } else {
                    last_check_time = Instant::now();
                    check_interval = DUR_ONE_DAY;
                }
            }
            Ok(UpdateMsg::Exit) => break,
        }
    }
}

fn check_update(manually: bool) -> ResultType<()> {
    // On macOS, auto-update is handled by check_update_as_root() in the service process.
    // The shared check_update() path is only used for manual update checks from the GUI.
    #[cfg(target_os = "macos")]
    if !manually {
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    let update_msi = crate::platform::is_msi_installed()? && !crate::is_custom_client();
    #[cfg(not(target_os = "windows"))]
    let update_msi = false;
    if !(manually || config::Config::get_bool_option(keys::OPTION_ALLOW_AUTO_UPDATE)) {
        return Ok(());
    }
    do_check_software_update()?;

    let update_url = crate::common::SOFTWARE_UPDATE_URL.lock().unwrap().clone();
    if update_url.is_empty() {
        log::debug!("No update available.");
    } else {
        let update_format = current_update_format(update_msi);
        if update_format == "unknown" {
            log::debug!("Automatic update is not supported on this platform.");
            return Ok(());
        }
        let query = crate::update_metadata::UpdateArtifactQuery {
            platform: current_update_platform(),
            arch: current_update_arch(),
            format: update_format,
        };
        let artifact = verified_update_artifact_from_release_page_url(&update_url, &query)?;
        let download_url = artifact.url.as_str();
        #[cfg(target_os = "windows")]
        let version = artifact.version.as_str();
        #[cfg(target_os = "windows")]
        log::debug!("New version available: {}", &version);
        let Some(file_path) = get_update_download_file_from_url(download_url) else {
            bail!("Failed to get the file path from the URL: {}", download_url);
        };
        ensure_verified_update_artifact(download_url, &file_path, artifact.size, &artifact.sha256)?;
        // We have checked if the `conns` is empty before, but we need to check again.
        // No need to care about the downloaded file here, because it's rare case that the `conns` are empty
        // before the download, but not empty after the download.
        if has_no_active_conns() {
            #[cfg(target_os = "windows")]
            update_new_version(update_msi, version, &file_path, &artifact.sha256);
            #[cfg(target_os = "macos")]
            {
                let Some(file_path) = file_path.to_str() else {
                    bail!("Invalid UTF-8 path: {}", file_path.display());
                };
                crate::platform::macos::update_to_verified_dmg(
                    file_path,
                    &artifact.sha256,
                    Some(artifact.size),
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn verified_update_path(
    p: &str,
    expected_sha256: &str,
    kind: &str,
    file_path: &Path,
) -> Option<(crate::platform::VerifiedUpdateFile, String)> {
    let update_file = match crate::platform::copy_and_verify_update_file_sha256(p, expected_sha256)
    {
        Ok(update_file) => update_file,
        Err(e) => {
            log::error!("Refusing to update from invalid {}: {}", kind, e);
            remove_update_file(file_path);
            return None;
        }
    };
    let update_path = match update_file.path_str() {
        Ok(path) => path.to_owned(),
        Err(e) => {
            log::error!("Failed to get verified {} path: {}", kind, e);
            update_file.cleanup();
            remove_update_file(file_path);
            return None;
        }
    };
    Some((update_file, update_path))
}

#[cfg(target_os = "windows")]
fn update_new_version(update_msi: bool, version: &str, file_path: &PathBuf, expected_sha256: &str) {
    log::debug!(
        "New version is downloaded, update begin, update msi: {update_msi}, version: {version}, file: {:?}",
        file_path.to_str()
    );
    if let Some(p) = file_path.to_str() {
        if let Some(session_id) = crate::platform::get_current_process_session_id() {
            if update_msi {
                let Some((update_file, update_path)) =
                    verified_update_path(p, expected_sha256, "msi", file_path)
                else {
                    return;
                };
                let result = crate::platform::update_me_msi(&update_path, expected_sha256, true);
                update_file.cleanup();
                match result {
                    Ok(_) => {
                        log::debug!("New version \"{}\" updated.", version);
                    }
                    Err(e) => {
                        log::error!(
                            "Failed to install the new msi version  \"{}\": {}",
                            version,
                            e
                        );
                        remove_update_file(file_path);
                    }
                }
            } else {
                let Some((update_file, update_path)) =
                    verified_update_path(p, expected_sha256, "exe", file_path)
                else {
                    return;
                };
                let custom_client_staging_dir = if crate::is_custom_client() {
                    let custom_client_staging_dir =
                        crate::platform::get_custom_client_staging_dir();
                    if let Err(e) = crate::platform::handle_custom_client_staging_dir_before_update(
                        &custom_client_staging_dir,
                    ) {
                        log::error!(
                            "Failed to handle custom client staging dir before update: {}",
                            e
                        );
                        update_file.cleanup();
                        remove_update_file(file_path);
                        return;
                    }
                    Some(custom_client_staging_dir)
                } else {
                    // Clean up any residual staging directory from previous custom client
                    let staging_dir = crate::platform::get_custom_client_staging_dir();
                    hbb_common::allow_err!(crate::platform::remove_custom_client_staging_dir(
                        &staging_dir
                    ));
                    None
                };
                let update_launched = match crate::platform::launch_privileged_process(
                    session_id,
                    &format!("\"{}\" --update", update_path),
                ) {
                    Ok(h) => {
                        if h.is_null() {
                            log::error!("Failed to update to the new version: {}", version);
                            false
                        } else {
                            log::debug!("New version \"{}\" is launched.", version);
                            unsafe {
                                winapi::um::handleapi::CloseHandle(h);
                            }
                            true
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to run the new version: {}", e);
                        false
                    }
                };
                if !update_launched {
                    if let Some(dir) = custom_client_staging_dir {
                        hbb_common::allow_err!(crate::platform::remove_custom_client_staging_dir(
                            &dir
                        ));
                    }
                    update_file.cleanup();
                    remove_update_file(file_path);
                }
            }
        } else {
            log::error!(
                "Failed to get the current process session id, Error {}",
                std::io::Error::last_os_error()
            );
            remove_update_file(file_path);
        }
    } else {
        // unreachable!()
        log::error!(
            "Failed to convert the file path to string: {}",
            file_path.display()
        );
    }
}

/// Queries all active connections (remote, file-transfer, port-forward, camera, terminal)
/// from every logged-in user's --server process via IPC.
/// The root service cannot read connection state directly since connections
/// live in user --server processes. Handles fast user switching by querying
/// all GUI users, including the login-window server at UID 0. Falls back to
/// false (assumes sessions active) on any IPC error to avoid updating during
/// an unknown session state.
#[cfg(target_os = "macos")]
pub fn has_no_active_conns_ipc() -> bool {
    let result = match hbb_common::tokio::runtime::Runtime::new() {
        Ok(rt) => rt.block_on(query_no_active_conns_ipc()),
        Err(err) => Err(hbb_common::anyhow::anyhow!(
            "failed to create IPC runtime: {err}"
        )),
    };
    match result {
        Ok(no_active_conns) => no_active_conns,
        Err(err) => {
            log::warn!(
                "[root-update] Unable to determine active connection state; deferring update: {}",
                err
            );
            false
        }
    }
}

#[cfg(target_os = "macos")]
async fn query_no_active_conns_ipc() -> ResultType<bool> {
    const IPC_TIMEOUT_MS: u64 = 1_000;
    // An empty GUI user set maps to UID 0 so LoginWindow must also confirm it is idle.
    let uids = crate::platform::get_logged_in_uids();
    for uid in uids {
        let mut conn = crate::ipc::connect_for_uid(IPC_TIMEOUT_MS, uid, "")
            .await
            .map_err(|err| {
                hbb_common::anyhow::anyhow!("IPC connection failed for uid {uid}: {err}")
            })?;
        conn.send(&crate::ipc::Data::HasNoActiveConns(None))
            .await
            .map_err(|err| hbb_common::anyhow::anyhow!("IPC send failed for uid {uid}: {err}"))?;
        match conn.next_timeout(IPC_TIMEOUT_MS).await {
            Ok(Some(crate::ipc::Data::HasNoActiveConns(Some(true)))) => {}
            Ok(Some(crate::ipc::Data::HasNoActiveConns(Some(false)))) => return Ok(false),
            Ok(Some(_)) => bail!("unexpected active-connection IPC response for uid {uid}"),
            Ok(None) => bail!("active-connection IPC closed for uid {uid}"),
            Err(err) => bail!("active-connection IPC failed for uid {uid}: {err}"),
        }
    }
    Ok(true)
}

#[cfg(target_os = "macos")]
fn wait_for_failed_update_retry() {
    const FAILURE_MARKER: &str = "/var/root/.capylinkupdate_failed";
    let marker = std::path::Path::new(FAILURE_MARKER);
    if !marker.exists() {
        return;
    }

    // The updater script records failure immediately before launchd restarts
    // the old daemon. Preserve the retry deadline across that restart instead
    // of consuming the marker and retrying the same broken release in 30 sec.
    let remaining = std::fs::metadata(marker)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| {
            std::time::SystemTime::now()
                .duration_since(modified)
                .ok()
        })
        .map(|elapsed| RETRY_INTERVAL.saturating_sub(elapsed))
        .unwrap_or(RETRY_INTERVAL);
    if !remaining.is_zero() {
        log::info!(
            "[root-update] Previous update failed; retrying in {} seconds.",
            remaining.as_secs()
        );
        std::thread::sleep(remaining);
    }
    match std::fs::remove_file(marker) {
        Ok(()) => log::info!("[root-update] Previous update retry interval elapsed."),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => log::warn!("[root-update] Failed to clear failure marker: {}", err),
    }
}

/// Starts the background silent auto-update scheduler for macOS.
/// Called from `start_os_service()` which runs as root via LaunchDaemon.
#[cfg(target_os = "macos")]
pub fn start_auto_update_macos() {
    let spawn_result = std::thread::Builder::new()
        .name("capylink-auto-update".to_owned())
        .spawn(|| {
            log::info!("[root-update] Auto-update scheduler thread started.");
            std::thread::sleep(INITIAL_CHECK_DELAY);
            wait_for_failed_update_retry();
            let mut interval = DUR_ONE_DAY;
            loop {
                log::info!("[root-update] Running scheduled update check...");
                let no_active_conns = has_no_active_conns_ipc();
                if !no_active_conns {
                    log::info!("[root-update] Active session in progress, retrying in 10 min.");
                    interval = MIN_INTERVAL;
                } else {
                    match check_update_as_root() {
                        Ok(update_started) => {
                            if update_started {
                                // The replacement script is detached and may fail
                                // after this process returns. Always retry at the
                                // failure interval until the new daemon replaces us.
                                interval = RETRY_INTERVAL;
                            } else {
                                interval = DUR_ONE_DAY;
                            }
                        }
                        Err(e) => {
                            log::error!("[root-update] Update check failed: {}", e);
                            interval = RETRY_INTERVAL;
                        }
                    }
                }
                std::thread::sleep(interval);
            }
        });
    if let Err(err) = spawn_result {
        log::error!("[root-update] Failed to start scheduler thread: {}", err);
    }
}

#[cfg(target_os = "macos")]
pub fn check_update_as_root() -> ResultType<bool> {
    let _update_lock = acquire_mac_update_lock()?;
    // Allow-auto-update setting
    if !config::Config::get_bool_option(keys::OPTION_ALLOW_AUTO_UPDATE) {
        log::info!("[root-update] Auto update is disabled, skipping.");
        return Ok(false);
    }
    if crate::is_custom_client() {
        log::info!("[root-update] Custom client detected, skipping stock update.");
        return Ok(false);
    }
    // Clean up only old temp dirs from previous failed updates. The detached
    // installer keeps using its update directory after this process exits and
    // releases the advisory lock, so a newly-started daemon must not remove a
    // directory that still belongs to the active transaction.
    if let Ok(entries) = std::fs::read_dir("/tmp") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(".capylinkupdate-root-")
                || name_str.starts_with(".capylinkdownload-")
            {
                let path = entry.path();
                let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                let mode = metadata.mode() & 0o7777;
                let is_stale = metadata
                    .modified()
                    .ok()
                    .and_then(|modified| std::time::SystemTime::now().duration_since(modified).ok())
                    .is_some_and(|age| age >= RETRY_INTERVAL);
                if metadata.file_type().is_dir() && metadata.uid() == 0 && mode == 0o700 && is_stale
                {
                    if let Err(err) = std::fs::remove_dir_all(&path) {
                        log::warn!(
                            "[root-update] Failed to remove stale temp dir {}: {}",
                            path.display(),
                            err
                        );
                    }
                }
            }
        }
    }
    if let Err(e) = do_check_software_update() {
        bail!("[root-update] Failed to check for software update: {}", e);
    }
    let update_url = crate::common::SOFTWARE_UPDATE_URL.lock().unwrap().clone();
    if update_url.is_empty() {
        log::info!("[root-update] No update available.");
        return Ok(false);
    }
    let query = current_update_artifact_query(false);
    let artifact = verified_update_artifact_from_release_page_url(&update_url, &query)?;
    let dmg_url = artifact.url.as_str();
    log::info!(
        "[root-update] New version: {}, downloading from {}",
        artifact.version,
        dmg_url
    );
    // Validate URL against GitHub release allowlist before downloading as root
    let Some(file_path_validated) = get_update_download_file_from_url(dmg_url) else {
        bail!("[root-update] URL failed allowlist check: {}", dmg_url);
    };
    drop(file_path_validated);
    // Use mktemp so a local user cannot pre-create a predictable path and
    // permanently deny updates for a reused service PID.
    let private_tmp_output = std::process::Command::new("/usr/bin/mktemp")
        .args(["-d", "/tmp/.capylinkdownload-XXXXXX"])
        .output()?;
    if !private_tmp_output.status.success() {
        bail!(
            "[root-update] Failed to create private download directory: {}",
            String::from_utf8_lossy(&private_tmp_output.stderr).trim()
        );
    }
    let private_tmp = String::from_utf8(private_tmp_output.stdout)
        .map_err(|err| hbb_common::anyhow::anyhow!("[root-update] mktemp output error: {}", err))?
        .trim()
        .to_owned();
    if private_tmp.is_empty() {
        bail!("[root-update] mktemp returned an empty download directory");
    }
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&private_tmp, std::fs::Permissions::from_mode(0o700))?;
    }
    let file_path = Path::new(&private_tmp).join(&artifact.file_name);
    let tmp_path = file_path.to_string_lossy().to_string();
    if let Err(err) =
        ensure_verified_update_artifact(dmg_url, &file_path, artifact.size, &artifact.sha256)
    {
        if let Err(cleanup_err) = std::fs::remove_dir_all(&private_tmp) {
            log::warn!(
                "[root-update] Failed to remove temp dir {}: {}",
                private_tmp,
                cleanup_err
            );
        }
        return Err(err);
    }
    log::info!("[root-update] Downloaded and verified at {}", tmp_path);
    // Recheck active sessions before installing — download can take minutes
    if !has_no_active_conns_ipc() {
        if let Err(e) = std::fs::remove_dir_all(&private_tmp) {
            log::warn!("[root-update] Failed to remove temp dir {}: {}", private_tmp, e);
        }
        bail!("[root-update] Active session started during download, deferring update.");
    }
    // Install silently as root
    let result = crate::platform::update_from_dmg_as_root(&tmp_path, &artifact.version);
    // Clean up download directory
    if let Err(e) = std::fs::remove_dir_all(&private_tmp) {
        log::warn!(
            "[root-update] Failed to remove temp dir {}: {}",
            private_tmp,
            e
        );
    }
    result.map(|_| true)
}
