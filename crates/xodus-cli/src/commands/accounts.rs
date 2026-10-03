use clap::Subcommand;
use std::process::ExitCode;
use xodus::tokens::TokenManager;

#[derive(Subcommand)]
pub enum AccountAction {
    /// List saved accounts and the active identity as JSON, without credentials.
    List,
    /// Select a saved account using its opaque ID from list.
    Select { id: String },
    /// Remove one account; if active, select the next saved account.
    Remove { id: String },
}

pub fn run(tokens: &TokenManager, action: AccountAction) -> ExitCode {
    let result = match action {
        AccountAction::List => Ok(()),
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
