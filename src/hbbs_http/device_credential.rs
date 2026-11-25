use hbb_common::{
    anyhow::anyhow,
    bail,
    config::{self, Config, LocalConfig},
    password_security::{self, ApproveMode},
    sodiumoxide::{base64, randombytes},
    tokio::{self, sync::Notify},
    ResultType,
};
use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::ErrorKind,
    net::IpAddr,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
use url::Url;

const LEASE: Duration = Duration::from_secs(90);
const REFRESH: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) static CHANGED: Notify = Notify::const_new();
#[cfg(all(feature = "flutter", not(target_os = "ios")))]
static ACCOUNT_SYNC: tokio::sync::Mutex<
    Option<crate::ipc::ConnectionTmpl<parity_tokio_ipc::ConnectionClient>>,
> = tokio::sync::Mutex::const_new(None);
static HOST_CHANGED: Notify = Notify::const_new();
static HOST_STATE: Mutex<HostState> = Mutex::new(HostState {
    account: None,
    lease: None,
    loaded: false,
});

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Credential {
    pub id: String,
    pub uuid: String,
    pub password_hash: String,
    pub salt: String,
    pub peer_key: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct HostIdentity {
    id: String,
    uuid: String,
    salt: String,
    peer_key: String,
    deployment: String,
    scope: String,
}

struct HostLease {
    identity: HostIdentity,
    password: Vec<u8>,
    valid_until: Instant,
}

struct HostState {
    account: Option<StoredAccount>,
    lease: Option<HostLease>,
    loaded: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredAccount {
    token: String,
    binding: String,
    id: String,
    uuid: String,
    local_config: PathBuf,
    #[cfg(windows)]
    owner_sid: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AccountFile {
    ciphertext: String,
}

#[derive(Clone, PartialEq, Eq)]
struct AccountContext {
    api: String,
    token: String,
    scope: String,
    route: crate::client::ExpectedRendezvous,
    deployment: String,
    binding: String,
}

fn account_file() -> PathBuf {
    Config::file().with_extension("account-credential")
}

fn local_account_file() -> PathBuf {
    let mut path = Config::file();
    let mut name = path.file_stem().unwrap_or_default().to_os_string();
    name.push("_local");
    if let Some(extension) = path.extension() {
        name.push(".");
        name.push(extension);
    }
    path.set_file_name(name);
    path
}

fn account_is_confirmed(account: &StoredAccount) -> ResultType<bool> {
    #[cfg(windows)]
    {
        use crate::platform::windows::{
            account_authorization_file_owner, account_authorization_user,
        };
        if account_authorization_user(None)? != account.owner_sid {
            return Ok(false);
        }
        let owner = account_authorization_file_owner(&account.local_config)?;
        if owner != account.owner_sid && owner != "S-1-5-32-544" {
            return Ok(false);
        }
    }
    #[cfg(not(windows))]
    if account.local_config != fs::canonicalize(local_account_file())? {
        return Ok(false);
    }
    // The GUI's persisted account remains authoritative after an IPC failure or process exit.
    let saved: hbb_common::toml::Value =
        hbb_common::toml::from_str(&fs::read_to_string(&account.local_config)?)?;
    let option = |name| {
        saved
            .get("options")
            .and_then(|v| v.get(name))
            .and_then(|v| v.as_str())
    };
    Ok(option(super::ACCESS_TOKEN) == Some(account.token.as_str())
        && option(super::ACCESS_TOKEN_DEPLOYMENT) == Some(account.binding.as_str()))
}

fn store_host_account(account: Option<&StoredAccount>) -> ResultType<()> {
    let Some(account) = account else {
        return match fs::remove_file(account_file()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        };
    };
    let encrypted = password_security::symmetric_crypt(&serde_json::to_vec(account)?, true)
        .map_err(|_| anyhow!("Could not encrypt background account authorization"))?;
    config::store_path(
        account_file(),
        AccountFile {
            ciphertext: base64::encode(&encrypted, base64::Variant::Original),
        },
    )
}

fn load_host_account(state: &mut HostState) {
    if state.loaded {
        return;
    }
    let result = (|| -> ResultType<Option<StoredAccount>> {
        let value = match fs::read_to_string(account_file()) {
            Ok(value) => value,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let file: AccountFile = hbb_common::toml::from_str(&value)?;
        if file.ciphertext.is_empty() {
            return Ok(None);
        }
        let encrypted = base64::decode(&file.ciphertext, base64::Variant::Original)
            .map_err(|_| anyhow!("Invalid encrypted background account authorization"))?;
        let plaintext = password_security::symmetric_crypt(&encrypted, false)
            .map_err(|_| anyhow!("Could not decrypt background account authorization"))?;
        Ok(Some(serde_json::from_slice(&plaintext)?))
    })();
    match result {
        Ok(account) => {
            state.account = account;
            state.loaded = true;
        }
        Err(error) => {
            hbb_common::throttled_log!(
                crate::common::API_LOG_INTERVAL,
                warn,
                "Background account authorization could not be restored: {error}"
            );
        }
    }
}

fn current_host_account(state: &mut HostState) -> Option<(String, String)> {
    load_host_account(state);
    if !state.loaded {
        return None;
    }
    let account = state.account.as_ref()?;
    let confirmed = match account_is_confirmed(account) {
        Ok(confirmed) => confirmed,
        Err(error) => {
            state.lease = None;
            hbb_common::throttled_log!(
                crate::common::API_LOG_INTERVAL,
                warn,
                "Background account authorization could not be confirmed: {error}"
            );
            return None;
        }
    };
    let id = Config::get_id();
    let uuid = crate::encode64(hbb_common::get_uuid());
    if id.is_empty() || uuid.is_empty() {
        return None;
    }
    let (api, _, _, deployment) = super::account_deployment(&Config::get_options());
    if !confirmed
        || endpoint(&api).is_err()
        || super::bound_account_token(account.token.clone(), account.binding.clone(), &deployment)
            != account.token
        || account.token.is_empty()
        || account.id != id
        || account.uuid != uuid
    {
        state.account = None;
        state.lease = None;
        if let Err(error) = store_host_account(None) {
            hbb_common::throttled_log!(
                crate::common::API_LOG_INTERVAL,
                warn,
                "Background account authorization could not be removed: {error}"
            );
        }
        return None;
    }
    Some((api, account.token.clone()))
}

pub(crate) fn host_account() -> Option<(String, String, PathBuf)> {
    let mut state = HOST_STATE.lock().unwrap();
    let (api, token) = current_host_account(&mut state)?;
    Some((api, token, state.account.as_ref()?.local_config.clone()))
}

fn host_context(state: &mut HostState) -> Option<AccountContext> {
    let (_, token) = current_host_account(state)?;
    account_context_for(
        &Config::get_options(),
        token,
        state.account.as_ref()?.binding.clone(),
    )
}

fn host_identity() -> Option<HostIdentity> {
    if config::is_outgoing_only()
        || config::option2bool("stop-service", &Config::get_option("stop-service"))
        || password_security::approve_mode() == ApproveMode::Click
        || !Config::get_key_confirmed()
    {
        return None;
    }
    let (api, server, key, deployment) = super::account_deployment(&Config::get_options());
    endpoint(&api).ok()?;
    if server.is_empty() || crate::get_rs_pk(&key).is_none() {
        return None;
    }
    let peer_key = Config::get_key_pair().1;
    let id = Config::get_id();
    let uuid = crate::encode64(hbb_common::get_uuid());
    let salt = Config::get_effective_permanent_password_salt();
    if id.is_empty() || uuid.is_empty() || salt.is_empty() || peer_key.len() != 32 {
        return None;
    }
    Some(HostIdentity {
        id,
        uuid,
        salt,
        peer_key: base64::encode(&peer_key, base64::Variant::Original),
        deployment,
        scope: crate::client::password_scope(
            &crate::client::rendezvous_transport_endpoint(&server),
            &key,
        )?,
    })
}

pub(crate) fn ipc_request(
    command: &str,
    value: &str,
    #[cfg(windows)] peer_pid: Option<u32>,
) -> Option<String> {
    if command != "account-credential-sync" {
        return None;
    }
    #[cfg(windows)]
    let owner_sid = {
        let caller = crate::platform::windows::account_authorization_user(Some(peer_pid?)).ok()?;
        if caller != crate::platform::windows::account_authorization_user(None).ok()? {
            return None;
        }
        caller
    };
    let account = match serde_json::from_str::<Option<(String, String, PathBuf)>>(value).ok()? {
        Some((token, binding, local_config)) => {
            let (api, _, _, deployment) = super::account_deployment(&Config::get_options());
            endpoint(&api).ok()?;
            let token = super::bound_account_token(token, binding.clone(), &deployment);
            if token.is_empty() {
                return None;
            }
            let id = Config::get_id();
            let uuid = crate::encode64(hbb_common::get_uuid());
            if id.is_empty() || uuid.is_empty() {
                return None;
            }
            let account = StoredAccount {
                token,
                binding,
                id,
                uuid,
                local_config: fs::canonicalize(local_config).ok()?,
                #[cfg(windows)]
                owner_sid,
            };
            if !account_is_confirmed(&account).ok()? {
                return None;
            }
            Some(account)
        }
        None => None,
    };
    let mut state = HOST_STATE.lock().unwrap();
    load_host_account(&mut state);
    if !state.loaded {
        return None;
    }
    if state.account != account || account.is_none() {
        state.lease = None;
        if let Err(error) = store_host_account(account.as_ref()) {
            state.account = None;
            hbb_common::throttled_log!(
                crate::common::API_LOG_INTERVAL,
                warn,
                "Background account authorization could not be saved: {error}"
            );
            return None;
        }
        state.account = account;
        HOST_CHANGED.notify_one();
    }
    Some("Y".to_owned())
}

pub(crate) fn host_password() -> Option<Vec<u8>> {
    let mut state = HOST_STATE.lock().unwrap();
    current_host_account(&mut state)?;
    let lease = state.lease.as_ref()?;
    if host_identity().as_ref() != Some(&lease.identity) || lease.valid_until <= Instant::now() {
        state.lease = None;
        return None;
    }
    Some(lease.password.clone())
}

fn loopback(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(&['[', ']'][..])
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    })
}

fn endpoint(api: &str) -> ResultType<Url> {
    let url = Url::parse(api)?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.scheme(), "http" | "https")
    {
        bail!("Account device credentials require an HTTP or HTTPS API URL");
    }
    Ok(url)
}

pub(super) fn http_client(api: &str) -> ResultType<Client> {
    let url = endpoint(api)?;
    let mut builder = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(REQUEST_TIMEOUT)
        .timeout(REQUEST_TIMEOUT);
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        builder = builder.use_rustls_tls();
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        builder = builder.use_preconfigured_tls(hbb_common::verifier::client_config(false)?);
    }
    if !loopback(&url) {
        if let Some(conf) = Config::get_socks() {
            use hbb_common::proxy::{Proxy, ProxyScheme};
            let proxy = Proxy::from_conf(&conf, None)?;
            let address = match &proxy.intercept {
                ProxyScheme::Http { host, .. } => format!("http://{host}"),
                ProxyScheme::Https { host, .. } => format!("https://{host}"),
                ProxyScheme::Socks5 { addr, .. } => format!("socks5://{addr}"),
            };
            let mut proxy_config = reqwest::Proxy::all(address)?;
            if let Some(auth) = proxy.intercept.maybe_auth() {
                proxy_config = proxy_config.basic_auth(auth.username(), auth.password());
            }
            builder = builder.proxy(proxy_config);
        }
    }
    Ok(builder.build()?)
}

fn account_context_for(
    options: &HashMap<String, String>,
    token: String,
    binding: String,
) -> Option<AccountContext> {
    let (api, server, key, deployment) = super::account_deployment(options);
    endpoint(&api).ok()?;
    let token = super::bound_account_token(token, binding, &deployment);
    if token.is_empty() || server.is_empty() || crate::get_rs_pk(&key).is_none() {
        return None;
    }
    let endpoint = crate::client::rendezvous_transport_endpoint(&server);
    Some(AccountContext {
        api,
        binding: super::deployment_digest(&[&deployment, &token]),
        token,
        scope: crate::client::password_scope(&endpoint, &key)?,
        route: crate::client::ExpectedRendezvous {
            server: endpoint,
            key,
            fixed_endpoint: true,
            guard_only: false,
        },
        deployment,
    })
}

async fn daemon_request(
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    connection: &mut crate::ipc::ConnectionTmpl<parity_tokio_ipc::ConnectionClient>,
    command: &str,
    binding: &str,
) -> ResultType<Option<String>> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        connection
            .send(&crate::ipc::Data::Config((
                command.to_owned(),
                Some(binding.to_owned()),
            )))
            .await?;
        match connection.next_timeout(1000).await? {
            Some(crate::ipc::Data::Config((name, result))) if name == command => Ok(result),
            _ => bail!("Invalid account credential synchronization response"),
        }
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        Ok(ipc_request(command, binding))
    }
}

async fn request(
    context: &AccountContext,
    method: Method,
    suffix: &str,
    body: &impl Serialize,
) -> ResultType<reqwest::Response> {
    Ok(http_client(&context.api)?
        .request(
            method,
            format!(
                "{}/api/device-credential{suffix}",
                context.api.trim_end_matches('/')
            ),
        )
        .bearer_auth(&context.token)
        .json(body)
        .send()
        .await?)
}

async fn revoke(context: &AccountContext, identity: &HostIdentity) {
    let _ = request(
        context,
        Method::DELETE,
        "",
        &serde_json::json!({"id":identity.id,"uuid":identity.uuid}),
    )
    .await;
}

#[cfg(all(feature = "flutter", not(target_os = "ios")))]
pub(crate) async fn sync_account(clear_if_empty: bool) -> ResultType<()> {
    let mut connection = ACCOUNT_SYNC.lock().await;
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    if connection.is_none() {
        *connection = Some(crate::ipc::connect(1000, "").await?);
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let options = crate::ipc::get_options_from_connection(
        connection.as_mut().ok_or_else(|| {
            anyhow!("Account credential synchronization connection is unavailable")
        })?,
        1000,
    )
    .await
    .map_err(|error| {
        *connection = None;
        error
    })?;
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let options = Config::get_options();
    let [token, binding] =
        LocalConfig::get_options([super::ACCESS_TOKEN, super::ACCESS_TOKEN_DEPLOYMENT]);
    let deployment = super::account_deployment(&options).3;
    let token = super::bound_account_token(token, binding.clone(), &deployment);
    if token.is_empty() && !clear_if_empty {
        return Ok(());
    }
    let value = serde_json::to_string(&(!token.is_empty()).then_some((
        token,
        binding,
        local_account_file(),
    )))?;
    let response = daemon_request(
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        connection.as_mut().ok_or_else(|| {
            anyhow!("Account credential synchronization connection is unavailable")
        })?,
        "account-credential-sync",
        &value,
    )
    .await
    .map_err(|error| {
        *connection = None;
        error
    })?;
    if response.as_deref() != Some("Y") {
        bail!("Background service did not acknowledge account credential synchronization");
    }
    Ok(())
}

#[cfg(all(feature = "flutter", not(target_os = "ios")))]
pub(crate) async fn run() {
    let mut interval = tokio::time::interval(REFRESH);
    loop {
        tokio::select! { _ = interval.tick() => {}, _ = CHANGED.notified() => {} }
        if !crate::flutter::get_global_event_channels()
            .iter()
            .any(|app| app == crate::flutter::APP_TYPE_MAIN)
        {
            continue;
        }
        if let Err(error) = sync_account(true).await {
            hbb_common::throttled_log!(
                crate::common::API_LOG_INTERVAL,
                warn,
                "Account credential synchronization failed: {error}"
            );
        }
    }
}

#[cfg(all(feature = "flutter", not(target_os = "ios")))]
pub(crate) async fn run_host() {
    let mut interval = tokio::time::interval(REFRESH);
    let mut uploaded: Option<(AccountContext, HostIdentity, Vec<u8>)> = None;
    loop {
        tokio::select! { _ = interval.tick() => {}, _ = HOST_CHANGED.notified() => {} }
        let context = {
            let mut state = HOST_STATE.lock().unwrap();
            let context = host_context(&mut state);
            if context.is_none() {
                state.lease = None;
            }
            context
        };
        if uploaded
            .as_ref()
            .is_some_and(|(old, _, _)| Some(old) != context.as_ref())
        {
            if let Some((old, identity, _)) = uploaded.take() {
                revoke(&old, &identity).await;
            }
        }
        let Some(context) = context else {
            continue;
        };
        let Some(identity) = host_identity().filter(|identity| {
            identity.scope == context.scope && identity.deployment == context.deployment
        }) else {
            HOST_STATE.lock().unwrap().lease = None;
            if let Some((old, identity, _)) = uploaded.take() {
                revoke(&old, &identity).await;
            }
            continue;
        };
        let password = match uploaded.as_ref() {
            Some((old, old_identity, password)) if old == &context && old_identity == &identity => {
                password.clone()
            }
            _ => randombytes::randombytes(32),
        };
        let credential = Credential {
            id: identity.id.clone(),
            uuid: identity.uuid.clone(),
            password_hash: base64::encode(&password, base64::Variant::Original),
            salt: identity.salt.clone(),
            peer_key: identity.peer_key.clone(),
        };
        let result = request(&context, Method::PUT, "", &credential).await;
        let revoked = result
            .as_ref()
            .is_ok_and(|response| response.status() == StatusCode::UNAUTHORIZED);
        let accepted = result.is_ok_and(|response| response.status() == StatusCode::NO_CONTENT);
        let mut state = HOST_STATE.lock().unwrap();
        let same_account = host_context(&mut state).as_ref() == Some(&context);
        if revoked && same_account {
            state.account = None;
            if let Err(error) = store_host_account(None) {
                hbb_common::throttled_log!(
                    crate::common::API_LOG_INTERVAL,
                    warn,
                    "Background account authorization could not be removed: {error}"
                );
            }
        }
        if accepted && same_account && host_identity().as_ref() == Some(&identity) {
            state.lease = Some(HostLease {
                identity: identity.clone(),
                password: password.clone(),
                valid_until: Instant::now() + LEASE,
            });
            uploaded = Some((context, identity, password));
        } else {
            state.lease = None;
            uploaded = None;
        }
    }
}

async fn resolve_context(route: &crate::client::ExpectedRendezvous) -> Option<AccountContext> {
    let options = super::account_options_async().await;
    let [token, binding] = super::confirmed_account_options()?;
    let mut context = account_context_for(&options, token, binding)?;
    if context.route.key != route.key
        || super::password_deployment_for_route(&options, &route.server, &route.key)
            .as_deref()
            != Some(context.deployment.as_str())
    {
        return None;
    }
    context.scope = crate::client::password_scope(&route.server, &route.key)?;
    context.route = route.clone();
    Some(context)
}

pub(crate) async fn resolve(
    id: &str,
    salt: &str,
    peer_key: &[u8],
    route: &crate::client::ExpectedRendezvous,
) -> Option<(Vec<u8>, crate::client::ExpectedRendezvous, String)> {
    if peer_key.len() != 32 {
        return None;
    }
    let context = resolve_context(route).await?;
    let peer_key = base64::encode(peer_key, base64::Variant::Original);
    let response = request(
        &context,
        Method::POST,
        "/resolve",
        &serde_json::json!({"id":id,"salt":salt,"peer_key":peer_key}),
    )
    .await
    .ok()?;
    if response.status() != StatusCode::OK {
        return None;
    }
    #[derive(Deserialize)]
    struct Resolved {
        password_hash: String,
        salt: String,
        peer_key: String,
    }
    let resolved: Resolved = response.json().await.ok()?;
    if resolved.salt != salt
        || resolved.peer_key != peer_key
        || resolve_context(route).await.as_ref() != Some(&context)
    {
        return None;
    }
    let password = base64::decode(&resolved.password_hash, base64::Variant::Original).ok()?;
    (password.len() == 32).then_some((password, context.route, context.binding))
}
