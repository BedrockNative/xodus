use clap::Subcommand;
use std::process::ExitCode;
use xodus::tokens::TokenManager;

#[derive(Subcommand)]
pub enum AccountAction {
    /// List saved accounts and the active identity as JSON, without credentials.
    List,
    /// Fetch this saved account's Xbox gamertag and picture URL, without credentials.
    Profile { id: String },
    /// Select a saved account using its opaque ID from list.
    Select { id: String },
    /// Remove one account; if active, select the next saved account.
    Remove { id: String },
}

pub async fn run(tokens: &TokenManager, action: AccountAction) -> ExitCode {
    if let AccountAction::Profile { id } = &action {
        return match xodus::api::xbox::profile::fetch(tokens, id).await {
            Ok(profile) => {
                println!(
                    "{}",
                    serde_json::to_string(&profile).expect("profile serializes")
                );
                ExitCode::SUCCESS
            }
            Err(_) => {
                eprintln!("Xbox profile unavailable. Retry later or refresh accounts.");
                ExitCode::FAILURE
            }
        };
    }
    let result = match action {
        AccountAction::List => Ok(()),
        AccountAction::Profile { .. } => unreachable!(),
        AccountAction::Select { id } => tokens.select_account(&id),
        AccountAction::Remove { id } => tokens.remove_account(&id),
    }
    .and_then(|()| tokens.accounts());
    match result {
        Ok(accounts) => {
            println!(
                "{}",
                serde_json::to_string(&accounts).expect("account summaries serialize")
            );
            ExitCode::SUCCESS
        }
        Err(_) => {
            // Backend errors can contain secret-service details. Never print stored tokens.
            eprintln!(
                "Could not access the saved account. Unlock the desktop keyring and refresh accounts."
            );
            ExitCode::FAILURE
        }
    }
}
