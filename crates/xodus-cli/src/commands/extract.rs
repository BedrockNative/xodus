use std::process::ExitCode;

use xodus::tokens::TokenManager;

use crate::commands::streaming;

pub async fn run(
    client: &reqwest::Client,
    tokens: &TokenManager,
    path: String,
    destination: String,
    market: String,
) -> ExitCode {
    streaming::run(
        client,
        tokens,
        streaming::InstallOptions {
            source: "file://".to_owned() + &path,
            destination,
            try_skip_ntfs: false,
            parallel: None,
            market: Some(market),
            save_launch_license: false,
        },
    )
    .await
}
