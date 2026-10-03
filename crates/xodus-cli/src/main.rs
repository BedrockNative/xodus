use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use xodus::tokens::TokenManager;

mod commands;
mod license;
mod package;
mod webview;

#[derive(Subcommand)]
enum SubCommand {
    #[cfg(unix)]
    #[command(about = "Verify purchase over IPC, then install and save the local launch license")]
    InstallOwned {
        product: String,
        source: String,
        destination: String,
        #[arg(short, long)]
        market: Option<String>,
    },
    #[cfg(unix)]
    #[command(about = "Check a Store purchase through the running Xodus service (JSON output)")]
    CheckOwnership {
        product: String,
    },
    #[command(about = "Download msixvc or xsp files fo given game")]
    Download {
        product: String,
        #[arg(short, long)]
        market: Option<String>,
        #[arg(
            long,
            default_value_t = false,
            help = "Display download URLs instead of downloading"
        )]
        dry_run: bool,
    },
    #[command(about = "Dump CIKs for use with XvdTool")]
    License {
        #[clap(help = "Content Id of a license")]
        content_id: String,
        #[clap(help = "A path where to dump CIKs")]
        ciks: String,
        #[arg(short, long)]
        market: Option<String>,
    },
    #[command(about = "Extract locally stored msixvc file")]
    Extract {
        path: String,
        destination: String,
        #[arg(short, long)]
        market: Option<String>,
    },
    #[command(
        about = "Extract a locally stored EAppx/EMSIX package (research task, see issue #91)"
    )]
    ExtractEappx {
        path: String,
        destination: String,
        #[arg(short, long, help = "Path to a keyfile with content decryption keys")]
        key_file: Option<String>,
    },
    Login,
    Logout {
        #[arg(long, default_value_t = false, help = "Remove device license")]
        device: bool,
    },
    #[command(about = "Download and extract the game through streaming algorithm")]
    Streaming {
        source: String,
        destination: String,
        #[arg(
            long,
            default_value_t = false,
            help = "Attempt to skip downloading NTFS metadata to be faste while missing some files"
        )]
        try_skip_ntfs: bool,
        #[arg(short, long)]
        parallel: Option<usize>,
        #[arg(short, long)]
        market: Option<String>,
    },
    #[cfg(unix)]
    #[command(about = "Run a Game with xodus wine")]
    Run {
        source: String,
        wine: String,
        #[arg(short, long)]
        exe: Option<String>,
        #[arg(short, long)]
        market: Option<String>,
        #[arg(
            long,
            help = "Use only the license saved by install-owned; never acquire a new license"
        )]
        offline_license: bool,
        #[arg(
            last = true,
            value_name = "GAME_ARGUMENT",
            help = "Arguments passed literally to the game after -- (no shell expansion)"
        )]
        arguments: Vec<std::ffi::OsString>,
    },
    #[command(about = "Generate or decrypt base64-encoded CLEP challenge data")]
    Clep {
        #[command(subcommand)]
        action: ClepAction,
    },
    #[command(about = "Decode SPLicenseBlock")]
    SpLicense {
        block: String,
    },
}

#[derive(Subcommand)]
enum ClepAction {
    #[command(
        about = "Generate a base64-encoded CLEP challenge (V2 and V4) from SMBIOS/disk serial data"
    )]
    Generate {
        #[arg(
            long,
            help = "Base64-encoded SMBIOS data (up to 256 bytes, zero-padded)"
        )]
        smbios: Option<String>,
        #[arg(
            long,
            help = "Base64-encoded disk serial (up to 64 bytes, zero-padded)"
        )]
        disk_serial: Option<String>,
    },
    #[command(about = "Decrypt a base64-encoded CLEP challenge back into its plaintext fields")]
    Decrypt {
        #[clap(help = "Base64-encoded, obfuscated CLEP challenge data (2048 bytes)")]
        data: String,
    },
}

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct CliArgs {
    #[command(subcommand)]
    command: SubCommand,
}

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
    let client = reqwest::ClientBuilder::new()
        .user_agent(format!("xodus-cli/{}", env!("CARGO_PKG_VERSION")))
        .connection_verbose(true)
        .build()
        .unwrap();
    let args = CliArgs::parse();

    #[cfg(unix)]
    if let SubCommand::InstallOwned { product, .. } = &args.command {
        use xodus::licensing::ownership::{OwnershipRequest, OwnershipStatus};
        match xodus::ipc::check_ownership(&OwnershipRequest {
            product_id: product.clone(),
        })
        .await
        {
            Ok(response) if response.status == OwnershipStatus::Purchased => {}
            Ok(response) => {
                eprintln!(
                    "Purchase not verified: {:?}. No game data was downloaded or decrypted.",
                    response.status
                );
                return ExitCode::FAILURE;
            }
            Err(_) => {
                eprintln!(
                    "Purchase verification unavailable. Start the matching isolated Xodus service."
                );
                return ExitCode::FAILURE;
            }
        }
    }

    #[cfg(unix)]
    if let SubCommand::CheckOwnership { product } = &args.command {
        use xodus::licensing::ownership::{OwnershipRequest, OwnershipStatus};
        return match xodus::ipc::check_ownership(&OwnershipRequest {
            product_id: product.clone(),
        })
        .await
        {
            Ok(response) => {
                println!(
                    "{}",
                    serde_json::to_string(&response).expect("Serializable ownership response")
                );
                match response.status {
                    OwnershipStatus::Purchased => ExitCode::SUCCESS,
                    OwnershipStatus::NotOwned => ExitCode::from(2),
                    OwnershipStatus::NoAccount => ExitCode::from(3),
                    _ => ExitCode::from(4),
                }
            }
            Err(_) => {
                eprintln!(
                    "Ownership query failed. Check the service version and isolated socket configuration."
                );
                ExitCode::from(4)
            }
        };
    }

    if matches!(args.command, SubCommand::Login)
        && let Err(error) = xodus::secrets::prepare_login()
    {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    if xodus::secrets::init_secrets().is_err() {
        eprintln!(
            "Unable to access secure credential storage. Retry Sign in to prepare or unlock your desktop keyring."
        );
        return ExitCode::FAILURE;
    }
    let tokens = TokenManager::with_keychain_and_memory();

    // Clep/SpLicense are pure local data transforms and Logout only removes
    // stored credentials - none of them need a device identity, so don't
    // force provisioning (network + keychain access) just to run them. This
    // matters in practice: on a session with no usable secret-service
    // keychain, provisioning fails outright, which previously meant even
    // these fully offline commands were unusable.
    let mut needs_device_credentials = !matches!(
        args.command,
        SubCommand::Clep { .. } | SubCommand::SpLicense { .. } | SubCommand::Logout { .. }
    );
    #[cfg(unix)]
    if matches!(
        args.command,
        SubCommand::Run {
            offline_license: true,
            ..
        }
    ) {
        needs_device_credentials = false;
    }
    if needs_device_credentials {
        xodus::tokens::device::ensure_device_credentials(&client, &tokens).await;
    }

    let code = match args.command {
        #[cfg(unix)]
        SubCommand::InstallOwned {
            product: _,
            source,
            destination,
            market,
        } => {
            commands::streaming::run(
                &client,
                &tokens,
                commands::streaming::InstallOptions {
                    source,
                    destination,
                    try_skip_ntfs: false,
                    parallel: None,
                    market,
                    save_launch_license: true,
                },
            )
            .await
        }
        #[cfg(unix)]
        SubCommand::CheckOwnership { .. } => {
            unreachable!("IPC commands return before initialization")
        }
        SubCommand::Download {
            product,
            market,
            dry_run,
        } => commands::download::run(&client, &tokens, product, market, dry_run).await,
        SubCommand::License {
            content_id,
            market,
            ciks,
        } => {
            commands::license::run(
                &client,
                &tokens,
                content_id,
                market.unwrap_or("neutral".to_string()),
                ciks,
            )
            .await
        }
        SubCommand::Login => commands::login::run(&client, &tokens).await,
        SubCommand::Logout { device } => commands::logout::run(&tokens, device).await,
        SubCommand::Extract {
            path,
            destination,
            market,
        } => {
            commands::extract::run(
                &client,
                &tokens,
                path,
                destination,
                market.unwrap_or("neutral".to_string()),
            )
            .await
        }
        SubCommand::ExtractEappx {
            path,
            destination,
            key_file,
        } => commands::extract_eappx::run(path, destination, key_file).await,
        SubCommand::Streaming {
            source,
            destination,
            try_skip_ntfs,
            market,
            parallel,
        } => {
            commands::streaming::run(
                &client,
                &tokens,
                commands::streaming::InstallOptions {
                    source,
                    destination,
                    try_skip_ntfs,
                    parallel,
                    market,
                    save_launch_license: false,
                },
            )
            .await
        }
        #[cfg(unix)]
        SubCommand::Run {
            source,
            wine,
            exe,
            market,
            offline_license,
            arguments,
        } => {
            commands::run::run(
                &client,
                &tokens,
                commands::run::RunOptions {
                    source,
                    wine,
                    exe,
                    market,
                    offline_license,
                    arguments,
                },
            )
            .await
        }
        SubCommand::Clep { action } => match action {
            ClepAction::Generate {
                smbios,
                disk_serial,
            } => commands::clep::generate(smbios, disk_serial),
            ClepAction::Decrypt { data } => commands::clep::decrypt(data),
        },
        SubCommand::SpLicense { block } => commands::splicense::run(block),
    };

    xodus::secrets::destroy_secrets();

    code
}

#[cfg(all(test, unix))]
mod launch_arguments_tests {
    use super::*;

    #[test]
    fn arguments_after_separator_are_not_xodus_options() {
        let args = CliArgs::try_parse_from([
            "xodus-cli",
            "run",
            "/game",
            "/wine",
            "--offline-license",
            "--",
            "--help",
            "two words",
            "",
            "$(echo nope)",
        ])
        .unwrap();
        let SubCommand::Run {
            offline_license,
            arguments,
            ..
        } = args.command
        else {
            panic!("Expected run");
        };
        assert!(offline_license);
        assert_eq!(
            arguments,
            ["--help", "two words", "", "$(echo nope)"].map(std::ffi::OsString::from)
        );
    }

    #[test]
    fn existing_run_without_game_arguments_is_unchanged() {
        let args =
            CliArgs::try_parse_from(["xodus-cli", "run", "/game", "/wine", "--offline-license"])
                .unwrap();
        let SubCommand::Run {
            offline_license,
            arguments,
            ..
        } = args.command
        else {
            panic!("Expected run");
        };
        assert!(offline_license);
        assert!(arguments.is_empty());
    }
}
