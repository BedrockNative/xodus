use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::process::ExitCode;
use std::sync::Arc;

use tokio::net::UnixListener;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use xodus::ipc::XML_MAGIC;
use xodus::tokens::TokenManager;

mod connection;
mod simple_context;

const PROTO_MAGIC: u32 = 0x58445350;

#[tokio::main]
async fn main() -> ExitCode {
    let filter = tracing_subscriber::EnvFilter::from_env("XODUS_LOG");
    let registry =
        tracing_subscriber::registry().with(tracing_subscriber::fmt::layer().with_filter(filter));

    #[cfg(feature = "tokio_console")]
    {
        use tracing::level_filters::LevelFilter;
        use tracing_subscriber::filter::Targets;

        let console_filter = Targets::new()
            .with_target("tokio", LevelFilter::TRACE)
            .with_target("runtime", LevelFilter::TRACE);
        let console_layer = console_subscriber::spawn().with_filter(console_filter);
        registry.with(console_layer).init();
    }
    #[cfg(not(feature = "tokio_console"))]
    {
        registry.init();
    }

    // Do not mistake an inaccessible keyring for a missing device identity.
    // The desktop owns the password dialog; Xodus never receives the password.
    match tokio::task::spawn_blocking(xodus::secrets::prepare_service).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            eprintln!("Xodus service could not access the keyring: {error}");
            return ExitCode::FAILURE;
        }
        Err(_) => {
            eprintln!("Xodus service could not finish preparing the desktop keyring.");
            return ExitCode::FAILURE;
        }
    }
    if xodus::secrets::init_secrets().is_err() {
        eprintln!(
            "Xodus service could not initialize credential storage after keyring preparation."
        );
        return ExitCode::FAILURE;
    }
    let profile = TokenManager::with_keychain_and_memory();
    let tokens = match std::env::var("XODUS_ACCOUNT_ID") {
        Ok(id) if !id.is_empty() => match profile.session_for_account(&id) {
            Ok(session) => session,
            Err(_) => {
                eprintln!(
                    "The selected session account is unavailable. Refresh the launcher's accounts; no other account was selected."
                );
                return ExitCode::FAILURE;
            }
        },
        Err(std::env::VarError::NotUnicode(_)) => {
            eprintln!("Invalid XODUS_ACCOUNT_ID.");
            return ExitCode::FAILURE;
        }
        _ => profile,
    };
    let tokens = Arc::new(tokens);
    xodus::tokens::device::ensure_device_credentials(&reqwest::Client::new(), &tokens).await;
    let xodus::models::secrets::Token::Legacy(device_token) =
        tokens.get_device_sts_token().unwrap()
    else {
        panic!("Device token isnt legacy")
    };

    let cancellation = CancellationToken::new();
    let socket_path = xodus::ipc::socket_path().expect("Invalid Xodus socket configuration");
    let trigger = cancellation.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("Failure to handle ctrl_c");
        trigger.cancel();
    });
    {
        let listener = UnixListener::bind(&socket_path).expect("Unable to bind to socket");
        let mode = 0o600;
        let perms = Permissions::from_mode(mode);
        tokio::fs::set_permissions(&socket_path, perms)
            .await
            .expect("Unable to restrict socket permissions");
        loop {
            let accept = tokio::select! {
                r = listener.accept() => r,
                _ = cancellation.cancelled() => break,
            }
            .expect("Failed to accept");

            let token = cancellation.clone();
            let device_token = device_token.clone();
            let tokens = tokens.clone();
            tokio::spawn(async move {
                connection::router::route(accept.0, token, device_token, tokens).await
            });
        }
    }

    _ = tokio::fs::remove_file(socket_path).await;
    ExitCode::SUCCESS
}
