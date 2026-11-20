#[cfg(target_os = "windows")]
use crate::client::translate;
#[cfg(not(debug_assertions))]
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::platform::breakdown_callback;
use base::config::keys;
#[cfg(not(debug_assertions))]
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use base::platform::register_breakdown_handler;
use hbb_common::{config, log};
#[cfg(windows)]
use tauri_winrt_notification::{Duration, Sound, Toast};

#[macro_export]
macro_rules! my_println{
    ($($arg:tt)*) => {
        #[cfg(not(windows))]
        println!("{}", format_args!($($arg)*));
        #[cfg(windows)]
        crate::platform::message_box(
            &format!("{}", format_args!($($arg)*))
        );
    };
}

pub enum CoreMainAction {
    StartUi(Vec<String>),
    ExitFailure(hbb_common::anyhow::Error),
}

#[cfg(any(feature = "flutter", test))]
const ENCODED_CONNECTION_URI_PREFIX: &str = "capylink://connect-id/peer/";

#[cfg(target_os = "macos")]
fn finish_macos_update(result: hbb_common::ResultType<()>) -> Option<CoreMainAction> {
    match result {
        Ok(()) => None,
        Err(err) => Some(CoreMainAction::ExitFailure(err)),
    }
}

/// shared by flutter and sciter main function
///
/// [Note]
/// If it returns [`None`], then the process will terminate, and flutter gui will not be started.
/// If it returns [`Some`], the action determines whether to start the UI or report failure.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn core_main() -> Option<CoreMainAction> {
    if !crate::common::global_init() {
        return None;
    }
    crate::load_custom_client();
    #[cfg(windows)]
    if !crate::platform::windows::bootstrap() {
        // return None to terminate the process
        return None;
    }
    let mut args = Vec::new();
    let mut flutter_args = Vec::new();
    let mut i = 0;
    let mut _is_elevate = false;
    let mut _is_run_as_system = false;
    let mut _is_quick_support = false;
    let mut _is_flutter_invoke_new_connection = false;
    let mut no_server = false;
    let mut arg_exe = Default::default();
    for arg in std::env::args() {
        if i == 0 {
            arg_exe = arg;
        } else if i > 0 {
            #[cfg(feature = "flutter")]
            if [
                "--connect",
                "--play",
                "--file-transfer",
                "--view-camera",
                "--port-forward",
                "--terminal",
                "--terminal-admin",
                "--rdp",
            ]
            .contains(&arg.as_str())
            {
                _is_flutter_invoke_new_connection = true;
            }
            if arg == "--elevate" {
                _is_elevate = true;
            } else if arg == "--run-as-system" {
                _is_run_as_system = true;
            } else if arg == "--quick_support" {
                _is_quick_support = true;
            } else if arg == "--no-server" {
                no_server = true;
            } else {
                args.push(arg);
            }
        }
        i += 1;
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if args.is_empty() {
        #[cfg(target_os = "linux")]
        let should_check_start_tray = crate::check_process("--server", false);
        // We can use `crate::check_process("--server", false)` on Windows.
        // Because `--server` process is the System user's process. We can't get the arguments in `check_process()`.
        // We can assume that self service running means the server is also running on Windows.
        #[cfg(target_os = "windows")]
        let should_check_start_tray = crate::platform::is_self_service_running()
            && crate::platform::is_cur_exe_the_installed();
        if should_check_start_tray && !crate::check_process("--tray", true) {
            #[cfg(target_os = "linux")]
            hbb_common::allow_err!(crate::platform::check_autostart_config());
            hbb_common::allow_err!(crate::run_me(vec!["--tray"]));
        }
    }
    #[cfg(not(debug_assertions))]
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    register_breakdown_handler(breakdown_callback);
    #[cfg(target_os = "linux")]
    #[cfg(feature = "flutter")]
    {
        let (k, v) = ("LIBGL_ALWAYS_SOFTWARE", "1");
        if config::option2bool(
            "allow-always-software-render",
            &config::Config::get_option("allow-always-software-render"),
        ) {
            std::env::set_var(k, v);
        } else {
            std::env::remove_var(k);
        }
    }
    #[cfg(windows)]
    if args.contains(&"--connect".to_string()) || args.contains(&"--view-camera".to_string()) {
        base::platform::windows::start_cpu_performance_monitor();
    }
    #[cfg(feature = "flutter")]
    if _is_flutter_invoke_new_connection {
        return core_main_invoke_new_connection(std::env::args()).map(CoreMainAction::StartUi);
    }
    let click_setup = cfg!(windows) && args.is_empty() && crate::common::is_setup(&arg_exe);
    if click_setup && !config::is_disable_installation() {
        args.push("--install".to_owned());
        flutter_args.push("--install".to_string());
    }
    if args.contains(&"--noinstall".to_string()) {
        args.clear();
    }
    // The portable wrapper injects `--install` when its name ends with `install.exe`,
    // including `no-install.exe`. Drop the argument instead of exiting so disabled
    // clients can continue running as portable applications.
    if config::is_disable_installation() {
        args.retain(|arg| arg != "--install");
        flutter_args.retain(|arg| arg != "--install");
    }
    if args.len() > 0 {
        if args[0] == "--version" {
            println!("{}", crate::VERSION);
            return None;
        } else if args[0] == "--build-date" {
            println!("{}", crate::BUILD_DATE);
            return None;
        }
    }
    #[cfg(windows)]
    {
        _is_quick_support |= !crate::platform::is_installed()
            && args.is_empty()
            && (is_quick_support_exe(&arg_exe)
                || config::LocalConfig::get_option("pre-elevate-service") == "Y"
                || (!click_setup && crate::platform::is_elevated(None).unwrap_or(false)));
        crate::portable_service::client::set_quick_support(_is_quick_support);
    }
    let mut log_name = "".to_owned();
    // Keep portable-service logs under a stable directory name.
    let has_portable_service_shmem_arg = args
        .iter()
        .any(|arg| arg.starts_with("--portable-service-shmem-name="));
    if has_portable_service_shmem_arg {
        log_name = "portable-service".to_owned();
    } else if args.len() > 0 && args[0].starts_with("--") {
        let name = args[0].replace("--", "");
        if !name.is_empty() {
            log_name = name;
        }
    }
    hbb_common::init_log(false, &log_name);

    // linux uni (url) go here.
    #[cfg(all(target_os = "linux", feature = "flutter"))]
    if let Some(uri) = args.first().filter(|arg| {
        arg.starts_with(&crate::get_uri_prefix()) || arg.starts_with(ENCODED_CONNECTION_URI_PREFIX)
    }) {
        return try_send_by_dbus(uri.clone()).map(CoreMainAction::StartUi);
    }

    #[cfg(windows)]
    if !crate::platform::is_installed()
        && args.is_empty()
        && _is_quick_support
        && !_is_elevate
        && !_is_run_as_system
    {
        use crate::portable_service::client;
        if let Err(e) = client::start_portable_service(client::StartPara::Direct) {
            log::error!("Failed to start portable service: {:?}", e);
        }
    }
    #[cfg(windows)]
    if !crate::platform::is_installed() && (_is_elevate || _is_run_as_system) {
        crate::platform::elevate_or_run_as_system(click_setup, _is_elevate, _is_run_as_system);
        return None;
    }
    if args.is_empty() || crate::common::is_empty_uni_link(&args[0]) {
        #[cfg(target_os = "macos")]
        {
            crate::platform::macos::try_remove_temp_update_dir(None);
        }

        #[cfg(windows)]
        {
            crate::platform::try_remove_temp_update_files();
            hbb_common::config::PeerConfig::preload_peers();
        }
        std::thread::spawn(move || crate::start_server(false, no_server));
    } else {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        // Root CLI management commands must talk to the user `--server` main IPC.
        // Example: `sudo capylink --option custom-rendezvous-server` should query the
        // user's IPC instead of root's `/tmp/<app>-0/ipc`; `connect()` still limits this
        // routing to empty-postfix main IPC only.
        let _user_main_ipc_scope = if crate::platform::is_installed()
            && is_root()
            && is_user_main_ipc_scope_cli_command(&args)
        {
            Some(crate::ipc::UserMainIpcScope::new())
        } else {
            None
        };

        #[cfg(windows)]
        {
            use crate::platform;
            if args[0] == "--uninstall" {
                if let Err(err) = platform::uninstall_me(true) {
                    log::error!("Failed to uninstall: {}", err);
                }
                return None;
            } else if args[0] == "--update" {
                // The elevated updater cannot delete its running verified copy.
                if let Err(err) = platform::schedule_current_verified_update_file_cleanup() {
                    log::warn!("Failed to schedule verified update file cleanup: {}", err);
                }
                if config::is_disable_installation() {
                    return None;
                }

                let text = match crate::platform::prepare_custom_client_update() {
                    Err(e) => {
                        log::error!("Error preparing custom client update: {}", e);
                        "Update failed!".to_string()
                    }
                    Ok(false) => "Update failed!".to_string(),
                    Ok(true) => match platform::update_me(false) {
                        Ok(_) => "Updated successfully!".to_string(),
                        Err(err) => {
                            log::error!("Failed with error: {err}");
                            "Update failed!".to_string()
                        }
                    },
                };
                Toast::new(Toast::POWERSHELL_APP_ID)
                    .title(&config::APP_NAME.read().unwrap())
                    .text1(&translate(text))
                    .sound(Some(Sound::Default))
                    .duration(Duration::Short)
                    .show()
                    .ok();
                return None;
            } else if args[0] == "--after-install" {
                if let Err(err) = platform::run_after_install() {
                    log::error!("Failed to after-install: {}", err);
                }
                return None;
            } else if args[0] == "--before-uninstall" {
                if let Err(err) = platform::run_before_uninstall() {
                    log::error!("Failed to before-uninstall: {}", err);
                }
                return None;
            } else if args[0] == "--silent-install" {
                if config::is_disable_installation() {
                    return None;
                }
                let (printer_override, debug) = parse_silent_install_args(&args);
                let options = platform::get_silent_install_options(printer_override);
                let res = platform::install_me(options, "".to_owned(), true, debug);
                let text = match res {
                    Ok(_) => translate("Installation Successful!".to_string()),
                    Err(err) => {
                        println!("Failed with error: {err}");
                        translate("Installation failed!".to_string())
                    }
                };
                Toast::new(Toast::POWERSHELL_APP_ID)
                    .title(&config::APP_NAME.read().unwrap())
                    .text1(&text)
                    .sound(Some(Sound::Default))
                    .duration(Duration::Short)
                    .show()
                    .ok();
                return None;
            } else if args[0] == "--uninstall-cert" {
                #[cfg(windows)]
                hbb_common::allow_err!(crate::platform::windows::uninstall_cert());
                return None;
            } else if args[0] == "--install-idd" {
                #[cfg(windows)]
                if crate::virtual_display_manager::is_virtual_display_supported() {
                    hbb_common::allow_err!(
                        crate::virtual_display_manager::capylink_idd::install_update_driver()
                    );
                }
                return None;
            } else if args[0] == "--portable-service" {
                crate::platform::elevate_or_run_as_system(
                    click_setup,
                    _is_elevate,
                    _is_run_as_system,
                );
                return None;
            } else if args[0] == "--uninstall-amyuni-idd" {
                #[cfg(windows)]
                hbb_common::allow_err!(
                    crate::virtual_display_manager::amyuni_idd::uninstall_driver()
                );
                return None;
            } else if args[0] == "--install-remote-printer" {
                #[cfg(windows)]
                if crate::platform::is_win_10_or_greater() {
                    match remote_printer::install_update_printer(&crate::get_app_name()) {
                        Ok(_) => {
                            log::info!("Remote printer installed/updated successfully");
                        }
                        Err(e) => {
                            log::error!("Failed to install/update the remote printer: {}", e);
                        }
                    }
                } else {
                    log::error!("Win10 or greater required!");
                }
                return None;
            } else if args[0] == "--uninstall-remote-printer" {
                #[cfg(windows)]
                if crate::platform::is_win_10_or_greater() {
                    remote_printer::uninstall_printer(&crate::get_app_name());
                    log::info!("Remote printer uninstalled");
                }
                return None;
            }
        }
        #[cfg(target_os = "macos")]
        {
            if args[0] == "--update" {
                return finish_macos_update(crate::manual_update::execute_macos_update(&args));
            }
        }
        if args[0] == "--remove" {
            if args.len() == 2 {
                // sleep a while so that process of removed exe exit
                std::thread::sleep(std::time::Duration::from_secs(1));
                std::fs::remove_file(&args[1]).ok();
                return None;
            }
        } else if args[0] == "--tray" {
            if !crate::check_process("--tray", true) {
                crate::tray::start_tray();
            }
            return None;
        } else if args[0] == "--install-service" {
            log::info!("start --install-service");
            crate::platform::install_service();
            return None;
        } else if args[0] == "--uninstall-service" {
            log::info!("start --uninstall-service");
            crate::platform::uninstall_service(false, true);
            return None;
        } else if args[0] == "--service" {
            log::info!("start --service");
            crate::start_os_service();
            return None;
        } else if args[0] == "--server" {
            log::info!("start --server with user {}", crate::username());
            #[cfg(target_os = "linux")]
            {
                hbb_common::allow_err!(crate::platform::check_autostart_config());
                std::process::Command::new("pkill")
                    .arg("-f")
                    .arg(&format!("{} --tray", crate::get_app_name().to_lowercase()))
                    .status()
                    .ok();
                hbb_common::allow_err!(crate::run_me(vec!["--tray"]));
            }
            #[cfg(windows)]
            crate::privacy_mode::restore_reg_connectivity(true, false);
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            {
                crate::start_server(true, false);
            }
            #[cfg(target_os = "macos")]
            {
                let handler = std::thread::spawn(move || crate::start_server(true, false));
                crate::tray::start_tray();
                // prevent server exit when encountering errors from tray
                hbb_common::allow_err!(handler.join());
            }
            return None;
        } else if args[0] == "--import-config" {
            if args.len() == 2 {
                let filepath;
                let path = std::path::Path::new(&args[1]);
                if !path.is_absolute() {
                    let mut cur = std::env::current_dir().unwrap();
                    cur.push(path);
                    filepath = cur.to_str().unwrap().to_string();
                } else {
                    filepath = path.to_str().unwrap().to_string();
                }
                import_config(&filepath);
            }
            return None;
        } else if args[0] == "--password" {
            if is_cli_setting_change_disabled() {
                println!("Settings are disabled!");
                return None;
            }
            if config::Config::is_disable_change_permanent_password() {
                println!("Changing permanent password is disabled!");
                return None;
            }
            if args.len() == 2 {
                if crate::platform::is_installed() && is_root() {
                    if let Err(err) = crate::ipc::set_permanent_password(args[1].to_owned()) {
                        println!("{err}");
                    } else {
                        println!("Done!");
                    }
                } else {
                    println!("Installation and administrative privileges required!");
                }
            }
            return None;
        } else if args[0] == "--set-unlock-pin" {
            if config::Config::is_disable_unlock_pin() {
                println!("Unlock PIN is disabled!");
                return None;
            }
            #[cfg(feature = "flutter")]
            if args.len() == 2 {
                if crate::platform::is_installed() && is_root() {
                    if let Err(err) = crate::ipc::set_unlock_pin(args[1].to_owned(), false) {
                        println!("{err}");
                    } else {
                        println!("Done!");
                    }
                } else {
                    println!("Installation and administrative privileges required!");
                }
            }
            return None;
        } else if args[0] == "--get-id" {
            println!("{}", crate::ipc::get_id());
            return None;
        } else if args[0] == "--set-id" {
            if is_cli_setting_change_disabled() {
                println!("Settings are disabled!");
                return None;
            }
            if config::Config::is_disable_change_id() {
                println!("Changing ID is disabled!");
                return None;
            }
            if args.len() == 2 {
                if crate::platform::is_installed() && is_root() {
                    let old_id = crate::ipc::get_id();
                    let mut res = crate::ui_interface::change_id_shared(args[1].to_owned(), old_id);
                    if res.is_empty() {
                        res = "Done!".to_owned();
                    }
                    println!("{}", res);
                } else {
                    println!("Installation and administrative privileges required!");
                }
            }
            return None;
        } else if args[0] == "--config" {
            if args.len() == 2 && !args[0].contains("host=") {
                if crate::platform::is_installed() && is_root() {
                    // encrypted string used in renaming exe.
                    let name = if args[1].ends_with(".exe") {
                        args[1].to_owned()
                    } else {
                        format!("{}.exe", args[1])
                    };
                    if let Ok(lic) = crate::custom_server::get_custom_server_from_string(&name) {
                        if !lic.host.is_empty() {
                            crate::ui_interface::set_option("key".into(), lic.key);
                            crate::ui_interface::set_option(
                                "custom-rendezvous-server".into(),
                                lic.host,
                            );
                            crate::ui_interface::set_option("api-server".into(), lic.api);
                            crate::ui_interface::set_option("relay-server".into(), lic.relay);
                        }
                    }
                } else {
                    println!("Installation and administrative privileges required!");
                }
            }
            return None;
        } else if args[0] == "--option" {
            if is_cli_setting_change_disabled() {
                println!("Settings are disabled!");
                return None;
            }
            if crate::platform::is_installed() && is_root() {
                if args.len() == 2 {
                    let options = crate::ipc::get_options();
                    println!("{}", options.get(&args[1]).unwrap_or(&"".to_owned()));
                } else if args.len() == 3 {
                    crate::ipc::set_option(&args[1], &args[2]);
                }
            } else {
                println!("Installation and administrative privileges required!");
            }
            return None;
        } else if args[0] == "--assign" {
            if config::Config::no_register_device() {
                println!("Cannot assign an unregistrable device!");
            } else if crate::platform::is_installed() && is_root() {
                let max = args.len() - 1;
                let pos = args.iter().position(|x| x == "--token").unwrap_or(max);
                if pos < max {
                    let token = args[pos + 1].to_owned();
                    let id = crate::ipc::get_id();
                    let uuid = crate::encode64(hbb_common::get_uuid());
                    let get_value = |c: &str| {
                        let pos = args.iter().position(|x| x == c).unwrap_or(max);
                        if pos < max {
                            Some(args[pos + 1].to_owned())
                        } else {
                            None
                        }
                    };
                    let user_name = get_value("--user_name");
                    let strategy_name = get_value("--strategy_name");
                    let address_book_name = get_value("--address_book_name");
                    let address_book_tag = get_value("--address_book_tag");
                    let address_book_alias = get_value("--address_book_alias");
                    let address_book_password = get_value("--address_book_password");
                    let address_book_note = get_value("--address_book_note");
                    let device_group_name = get_value("--device_group_name");
                    let note = get_value("--note");
                    let device_username = get_value("--device_username");
                    let device_name = get_value("--device_name");
                    let mut body = serde_json::json!({
                        "id": id,
                        "uuid": uuid,
                    });
                    let header = "Authorization: Bearer ".to_owned() + &token;
                    if user_name.is_none()
                        && strategy_name.is_none()
                        && address_book_name.is_none()
                        && device_group_name.is_none()
                        && note.is_none()
                        && device_username.is_none()
                        && device_name.is_none()
                    {
                        println!(
                            r#"At least one of the following options is required:
  --user_name
  --strategy_name
  --address_book_name
  --device_group_name
  --note
  --device_username
  --device_name"#
                        );
                    } else {
                        if let Some(name) = user_name {
                            body["user_name"] = serde_json::json!(name);
                        }
                        if let Some(name) = strategy_name {
                            body["strategy_name"] = serde_json::json!(name);
                        }
                        if let Some(name) = address_book_name {
                            body["address_book_name"] = serde_json::json!(name);
                            if let Some(name) = address_book_tag {
                                body["address_book_tag"] = serde_json::json!(name);
                            }
                            if let Some(name) = address_book_alias {
                                body["address_book_alias"] = serde_json::json!(name);
                            }
                            if let Some(name) = address_book_password {
                                body["address_book_password"] = serde_json::json!(name);
                            }
                            if let Some(name) = address_book_note {
                                body["address_book_note"] = serde_json::json!(name);
                            }
                        }
                        if let Some(name) = device_group_name {
                            body["device_group_name"] = serde_json::json!(name);
                        }
                        if let Some(name) = note {
                            body["note"] = serde_json::json!(name);
                        }
                        if let Some(name) = device_username {
                            body["device_username"] = serde_json::json!(name);
                        }
                        if let Some(name) = device_name {
                            body["device_name"] = serde_json::json!(name);
                        }
                        let url = crate::ui_interface::get_api_server() + "/api/devices/cli";
                        match crate::post_request_sync(url, body.to_string(), &header) {
                            Err(err) => println!("{}", err),
                            Ok(text) => {
                                if text.is_empty() {
                                    println!("Done!");
                                } else {
                                    println!("{}", text);
                                }
                            }
                        }
                    }
                } else {
                    println!("--token is required!");
                }
            } else {
                println!("Installation and administrative privileges required!");
            }
            return None;
        } else if args[0] == "--deploy" {
            if config::Config::no_register_device() {
                println!("Cannot deploy an unregistrable device!");
            } else if config::is_outgoing_only() {
                println!("Cannot deploy Outgoing-only clients.");
            } else if crate::platform::is_installed() && is_root() {
                let max = args.len() - 1;
                let pos = args.iter().position(|x| x == "--token").unwrap_or(max);
                if pos >= max {
                    println!("--token is required!");
                    return None;
                }
                let token = args[pos + 1].to_owned();
                let get_value = |c: &str| {
                    let pos = args.iter().position(|x| x == c).unwrap_or(max);
                    if pos < max {
                        Some(args[pos + 1].to_owned())
                    } else {
                        None
                    }
                };
                // An empty --id (e.g. an unset var) would deploy a blank id; the Android flow guards this too (#15146).
                let new_id = get_value("--id").filter(|s| !s.is_empty());
                match crate::ui_interface::deploy_device(token, new_id) {
                    crate::ui_interface::DeployResult::Ok => {
                        println!("Device deployed.");
                    }
                    crate::ui_interface::DeployResult::NotEnabled => {
                        println!("Server does not require deployment.");
                        std::process::exit(3);
                    }
                    crate::ui_interface::DeployResult::InvalidInput => {
                        println!("Invalid input.");
                        std::process::exit(5);
                    }
                    crate::ui_interface::DeployResult::IdTaken(id) => {
                        println!(
                            "Id `{}` is already used by another machine on the server.",
                            id
                        );
                        std::process::exit(6);
                    }
                    crate::ui_interface::DeployResult::Error(err) => {
                        println!("{}", err);
                        std::process::exit(1);
                    }
                }
            } else {
                println!("Installation and administrative privileges required!");
            }
            return None;
        } else if args[0] == "--check-hwcodec-config" {
            #[cfg(feature = "hwcodec")]
            crate::ipc::hwcodec_process();
            return None;
        } else if args[0] == "--terminal-helper" {
            // Terminal helper process - runs as user to create ConPTY
            // This is needed because ConPTY has compatibility issues with CreateProcessAsUserW
            #[cfg(target_os = "windows")]
            {
                let helper_args: Vec<String> = args[1..].to_vec();
                if let Err(e) = crate::server::terminal_helper::run_terminal_helper(&helper_args) {
                    log::error!("Terminal helper failed: {}", e);
                }
            }
            return None;
        } else if args[0] == "--cm" {
            // call connection manager to establish connections
            // meanwhile, return true to call flutter window to show control panel
            crate::ui_interface::start_option_status_sync();
        } else if args[0] == "--whiteboard" {
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                crate::whiteboard::run();
            }
            return None;
        } else if args[0] == "-gtk-sudo" {
            // capylink service kill `capylink --` processes
            #[cfg(target_os = "linux")]
            if args.len() > 2 {
                crate::platform::gtk_sudo::exec();
            }
            return None;
        }
    }
    //_async_logger_holder.map(|x| x.flush());
    #[cfg(feature = "flutter")]
    return Some(CoreMainAction::StartUi(flutter_args));
    #[cfg(not(feature = "flutter"))]
    return Some(CoreMainAction::StartUi(args));
}

fn import_config(path: &str) {
    use hbb_common::{config::*, get_exe_time, get_modified_time};
    let path2 = path.replace(".toml", "2.toml");
    let path2 = std::path::Path::new(&path2);
    let path = std::path::Path::new(path);
    log::info!("import config from {:?} and {:?}", path, path2);
    let config: Config = load_path(path.into());
    if config.is_empty() {
        log::info!("Empty source config, skipped");
        return;
    }
    if get_modified_time(&path) > get_modified_time(&Config::file())
        && get_modified_time(&path) < get_exe_time()
    {
        if store_path(Config::file(), config).is_err() {
            log::info!("config written");
        }
    }
    let config2: Config2 = load_path(path2.into());
    if get_modified_time(&path2) > get_modified_time(&Config2::file()) {
        if store_path(Config2::file(), config2).is_err() {
            log::info!("config2 written");
        }
    }
}

/// invoke a new connection
///
/// [Note]
/// this is for invoke new connection from dbus.
/// If it returns [`None`], then the process will terminate, and flutter gui will not be started.
/// If it returns [`Some`], then the process will continue, and flutter gui will be started.
#[cfg(feature = "flutter")]
fn core_main_invoke_new_connection(mut args: std::env::Args) -> Option<Vec<String>> {
    let mut authority = None;
    let mut id = None;
    let mut param_array = vec![];
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--connect" | "--play" | "--file-transfer" | "--view-camera" | "--port-forward"
            | "--terminal" | "--terminal-admin" | "--rdp" => {
                authority = Some((&arg.to_string()[2..]).to_owned());
                id = args.next();
            }
            "--password" => {
                if let Some(password) = args.next() {
                    param_array.push(format!("password={password}"));
                }
            }
            "--relay" => {
                param_array.push(format!("relay=true"));
            }
            "--request-approval" => {
                param_array.push("request-approval=true".to_owned());
            }
            // inner
            "--switch_uuid" => {
                if let Some(switch_uuid) = args.next() {
                    param_array.push(format!("switch_uuid={switch_uuid}"));
                }
            }
            _ => {}
        }
    }
    let authority = authority?;
    let mut id = id?;
    let ext = format!(".{}", crate::get_app_name().to_lowercase());
    let path = std::path::Path::new(&id);
    let recent_connection_file = cfg!(windows)
        && authority == "play"
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(&ext[1..]));
    if recent_connection_file {
        // The runner transfers foreground permission before dispatching this link.
        return Some(vec![recent_connection_uri(path, &param_array)?]);
    }
    if id.ends_with(&ext) {
        id = id.replace(&ext, "");
    }
    let uri_prefix = crate::get_uri_prefix();
    let params = param_array.join("&");
    let params_flag = if params.is_empty() { "" } else { "?" };
    let uni_links = format!("{uri_prefix}{authority}/{id}{params_flag}{params}");

    #[cfg(target_os = "linux")]
    return try_send_by_dbus(uni_links);

    #[cfg(windows)]
    {
        use winapi::um::winuser::WM_USER;
        let res = crate::platform::send_message_to_hnwd(
            &crate::platform::FLUTTER_RUNNER_WIN32_WINDOW_CLASS,
            &crate::get_app_name(),
            (WM_USER + 2) as _, // referred from unilinks desktop pub
            uni_links.as_str(),
            false,
        );
        return if res { None } else { Some(Vec::new()) };
    }
    #[cfg(target_os = "macos")]
    {
        return if let Err(_) = crate::ipc::send_url_scheme(uni_links) {
            Some(Vec::new())
        } else {
            None
        };
    }
}

#[cfg(any(feature = "flutter", test))]
fn recent_connection_uri(path: &std::path::Path, params: &[String]) -> Option<String> {
    let recent = crate::ui_session_interface::recent_connection_target(path)?;
    let direct = !recent.target.contains('@') && crate::common::is_direct_ip_access(&recent.target);
    let ipv6_authority = (direct && recent.target.starts_with('['))
        .then(|| format!("{}{}", crate::get_uri_prefix(), recent.target));
    let mut link = if let Some(original) = &ipv6_authority {
        url::Url::parse(original).ok()?
    } else {
        let prefix = if direct {
            format!("{}connect/", crate::get_uri_prefix())
        } else {
            ENCODED_CONNECTION_URI_PREFIX.to_owned()
        };
        let mut link = url::Url::parse(&prefix).ok()?;
        link.path_segments_mut().ok()?.pop_if_empty().push(&recent.target);
        link
    };
    if !direct || !params.is_empty() {
        let mut query = link.query_pairs_mut();
        if !recent.key.is_empty() {
            query.append_pair("key", &recent.key);
        }
        if recent.prefer_current {
            query.append_pair("prefer_current", "1");
        }
        if !recent.endpoint.is_empty() {
            query.append_pair("endpoint", &recent.endpoint);
        }
        if !direct && (params.is_empty() || recent.prefer_current || !recent.endpoint.is_empty()) {
            query.append_pair("recent", "true");
        }
        query.extend_pairs(params.iter().filter_map(|param| param.split_once('=')));
    }
    if let Some(mut original) = ipv6_authority {
        // Serializing Url would compress the IPv6 address the legacy direct parser reads.
        if let Some(query) = link.query() {
            original.push('?');
            original.push_str(query);
        }
        return Some(original);
    }
    Some(link.into())
}

#[cfg(test)]
mod recent_connection_tests {
    #[test]
    fn direct_recent_shortcuts_use_the_legacy_direct_connection() {
        let path = std::path::Path::new("Recent Connections/aux.example.com:21118.capylink");
        assert_eq!(
            super::recent_connection_uri(path, &[]).as_deref(),
            Some("capylink://connect/aux.example.com:21118")
        );
    }

    #[test]
    fn recent_connection_uri_preserves_the_device_id_and_parameters() {
        let path = std::path::Path::new("Recent Connections/OfficeMünchen@server.example.CAPYLINK");
        let params = ["password=a+b&c=d".to_owned(), "relay=true".to_owned()];
        assert_eq!(
            super::recent_connection_uri(path, &params).as_deref(),
            Some("capylink://connect-id/peer/OfficeM%C3%BCnchen@server.example?password=a%2Bb%26c%3Dd&relay=true")
        );
    }

    #[test]
    fn recent_connection_uri_opens_the_long_endpoint() {
        use crate::ui_session_interface::{
            recent_connection_file_name, store_recent_connection, RecentConnection,
        };
        use std::path::PathBuf;

        struct Directory(PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let directory = Directory(std::env::temp_dir().join(uuid::Uuid::new_v4().to_string()));
        std::fs::create_dir(&directory.0).unwrap();
        let id = format!(
            "OfficePC@{}.{}.{}.example:21116",
            "A".repeat(62), "B".repeat(62), "C".repeat(62)
        );
        let key = crate::encode64([255u8; 32]);
        let recent = RecentConnection {
            target: id.clone(),
            key: key.clone(),
            prefer_current: true,
            endpoint: String::new(),
        };
        let path = directory.0.join(recent_connection_file_name(&recent, "capylink").unwrap());
        store_recent_connection(&path, &recent).unwrap();
        let uri = super::recent_connection_uri(&path, &[]).unwrap();
        let parsed = url::Url::parse(&uri).unwrap();
        assert_eq!(parsed.path(), format!("/peer/{id}"));
        let query: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
        assert_eq!(query.get("key").map(|value| value.as_ref()), Some(key.as_str()));
        assert_eq!(query.get("prefer_current").map(|value| value.as_ref()), Some("1"));
        assert_eq!(query.get("recent").map(|value| value.as_ref()), Some("true"));
        std::fs::write(&path, "capylink recent connection 2\nOfficePC@other.example\n\n1").unwrap();
        assert!(store_recent_connection(&path, &recent).is_err());
        assert!(super::recent_connection_uri(&path, &[]).is_none());
        assert!(std::fs::read_to_string(&path).unwrap().contains("OfficePC@other.example"));
    }
}

#[cfg(all(target_os = "linux", feature = "flutter"))]
fn try_send_by_dbus(uni_links: String) -> Option<Vec<String>> {
    use crate::dbus::invoke_new_connection;

    match invoke_new_connection(uni_links) {
        Ok(()) => {
            return None;
        }
        Err(err) => {
            log::error!("{}", err.as_ref());
            // return Some to invoke this url by self
            return Some(Vec::new());
        }
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn is_root() -> bool {
    #[cfg(windows)]
    {
        return crate::platform::is_elevated(None).unwrap_or_default()
            || crate::platform::is_root();
    }
    #[allow(unreachable_code)]
    crate::platform::is_root()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_user_main_ipc_scope_cli_command(args: &[String]) -> bool {
    matches!(
        args.first().map(String::as_str),
        Some("--password")
            | Some("--set-unlock-pin")
            | Some("--get-id")
            | Some("--set-id")
            | Some("--config")
            | Some("--option")
            | Some("--assign")
            | Some("--deploy")
    )
}

#[inline]
fn is_cli_setting_change_disabled() -> bool {
    let option = keys::OPTION_ALLOW_COMMAND_LINE_SETTINGS_WHEN_SETTINGS_DISABLED;
    let allow_command_line_settings =
        config::option2bool(option, &crate::get_builtin_option(option));
    config::is_disable_settings() && !allow_command_line_settings
}

#[cfg(windows)]
fn parse_silent_install_args(args: &[String]) -> (Option<bool>, bool) {
    let mut printer_override = None;
    let mut debug = false;

    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "printer=1" => printer_override = Some(true),
            "printer=0" => printer_override = Some(false),
            "debug" => debug = true,
            _ => {}
        }
    }

    (printer_override, debug)
}

/// Check if the executable is a Quick Support version.
/// Note: This function must be kept in sync with `libs/portable/src/main.rs`.
#[cfg(windows)]
#[inline]
fn is_quick_support_exe(exe: &str) -> bool {
    let exe = exe.to_lowercase();
    exe.contains("-qs-") || exe.contains("-qs.exe") || exe.contains("_qs.exe")
}
