use hbb_common::{
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
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};
use url::Url;

const LEASE: Duration = Duration::from_secs(90);
const REFRESH: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) static CHANGED: Notify = Notify::const_new();
static HOST_LEASE: Mutex<Option<HostLease>> = Mutex::new(None);

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
    binding: String,
    identity: HostIdentity,
    password: Vec<u8>,
    valid_until: Instant,
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

pub(crate) fn ipc_request(command: &str, binding: &str) -> Option<String> {
    if command == "account-credential-confirm" {
        let (binding, identity, password): (String, HostIdentity, String) =
            serde_json::from_str(binding).ok()?;
        if binding.len() != 64
            || !binding.bytes().all(|byte| byte.is_ascii_hexdigit())
            || host_identity().as_ref() != Some(&identity)
        {
            return None;
        }
        let password = base64::decode(&password, base64::Variant::Original).ok()?;
        if password.len() != 32 {
            return None;
        }
        *HOST_LEASE.lock().unwrap() = Some(HostLease {
            binding,
            identity,
            password,
            valid_until: Instant::now() + LEASE,
        });
        return Some("Y".to_owned());
    }
    if binding.len() != 64 || !binding.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    match command {
        "account-credential-prepare" => serde_json::to_string(&host_identity()?).ok(),
        "account-credential-clear" => {
            let mut current = HOST_LEASE.lock().unwrap();
            if current
                .as_ref()
                .is_some_and(|lease| lease.binding == binding)
            {
                *current = None;
            }
            Some("Y".to_owned())
        }
        _ => None,
    }
}

pub(crate) fn host_password() -> Option<Vec<u8>> {
    let mut current = HOST_LEASE.lock().unwrap();
    let lease = current.as_ref()?;
    if host_identity().as_ref() != Some(&lease.identity) || lease.valid_until <= Instant::now() {
        *current = None;
        return None;
    }
    Some(lease.password.clone())
}

fn loopback(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
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
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback(&url)))
    {
        bail!("Account device credentials require HTTPS or loopback HTTP");
    }
    Ok(url)
}

fn http_client(api: &str) -> ResultType<Client> {
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

async fn account_context() -> Option<AccountContext> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let options = crate::ipc::get_options_async().await;
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let options = Config::get_options();
    let (api, server, key, deployment) = super::account_deployment(&options);
    endpoint(&api).ok()?;
    let [token, binding] =
        LocalConfig::get_options([super::ACCESS_TOKEN, super::ACCESS_TOKEN_DEPLOYMENT]);
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

async fn daemon_request(command: &str, binding: &str) -> Option<String> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let mut connection = crate::ipc::connect(1000, "").await.ok()?;
        connection
            .send(&crate::ipc::Data::Config((
                command.to_owned(),
                Some(binding.to_owned()),
            )))
            .await
            .ok()?;
        match connection.next_timeout(1000).await.ok()? {
            Some(crate::ipc::Data::Config((name, result))) if name == command => result,
            _ => None,
        }
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        ipc_request(command, binding)
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
    let _ = daemon_request("account-credential-clear", &context.binding).await;
    let _ = request(
        context,
        Method::DELETE,
        "",
        &serde_json::json!({"id":identity.id,"uuid":identity.uuid}),
    )
    .await;
}

#[cfg(all(feature = "flutter", not(target_os = "ios")))]
pub(crate) async fn run() {
    let mut interval = tokio::time::interval(REFRESH);
    let mut uploaded: Option<(AccountContext, HostIdentity, String)> = None;
    loop {
        tokio::select! { _ = interval.tick() => {}, _ = CHANGED.notified() => {} }
        let context = if crate::flutter::get_global_event_channels()
            .iter()
            .any(|app| app == crate::flutter::APP_TYPE_MAIN)
        {
            account_context().await
        } else {
            None
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
        let prepared = daemon_request("account-credential-prepare", &context.binding)
            .await
            .and_then(|value| serde_json::from_str::<HostIdentity>(&value).ok());
        let Some(identity) = prepared.filter(|identity| {
            identity.scope == context.scope && identity.deployment == context.deployment
        }) else {
            if let Some((old, identity, _)) = uploaded.take() {
                revoke(&old, &identity).await;
            }
            continue;
        };
        let password_hash = match uploaded.as_ref() {
            Some((old, old_identity, hash)) if old == &context && old_identity == &identity => {
                hash.clone()
            }
            _ => base64::encode(&randombytes::randombytes(32), base64::Variant::Original),
        };
        let credential = Credential {
            id: identity.id.clone(),
            uuid: identity.uuid.clone(),
            password_hash: password_hash.clone(),
            salt: identity.salt.clone(),
            peer_key: identity.peer_key.clone(),
        };
        let result = request(&context, Method::PUT, "", &credential).await;
        let accepted = result.is_ok_and(|response| response.status() == StatusCode::NO_CONTENT)
            && account_context().await.as_ref() == Some(&context);
        let confirmation =
            serde_json::to_string(&(&context.binding, &identity, &password_hash)).ok();
        if accepted
            && daemon_request(
                "account-credential-confirm",
                confirmation.as_deref().unwrap_or_default(),
            )
            .await
            .as_deref()
                == Some("Y")
        {
            uploaded = Some((context, identity, password_hash));
        } else {
            let _ = daemon_request("account-credential-clear", &context.binding).await;
            uploaded = None;
        }
    }
}

pub(crate) async fn resolve(
    id: &str,
    salt: &str,
    peer_key: &[u8],
    scope: Option<&str>,
) -> Option<(Vec<u8>, crate::client::ExpectedRendezvous)> {
    if peer_key.len() != 32 {
        return None;
    }
    let context = account_context().await?;
    if scope != Some(context.scope.as_str()) {
        return None;
    }
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
        || account_context().await.as_ref() != Some(&context)
    {
        return None;
    }
    let password = base64::decode(&resolved.password_hash, base64::Variant::Original).ok()?;
    (password.len() == 32).then_some((password, context.route))
}
