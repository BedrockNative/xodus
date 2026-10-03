use xodus::api::live::store;
use xodus::licensing::splicense::{DeviceKey, SPLicense};
use xodus::models::secrets::SavedAccount;
use xodus::tokens::TokenManager;

pub async fn get_license(
    client: &reqwest::Client,
    tokens: &TokenManager,
    content_id: String,
    market: String,
) -> Result<(DeviceKey, SPLicense), String> {
    let accounts = tokens
        .ownership_accounts()
        .map_err(|_| "Unable to read the profile accounts")?;
    for account in accounts {
        if let Ok(license) =
            get_account_license(client, tokens, account, &content_id, &market).await
        {
            return Ok(license);
        }
    }
    Err("No signed-in account could acquire a license for this content".into())
}

async fn get_account_license(
    client: &reqwest::Client,
    tokens: &TokenManager,
    account: SavedAccount,
    content_id: &str,
    market: &str,
) -> Result<(DeviceKey, SPLicense), ()> {
    let credentials = store::authenticate(client, tokens, account)
        .await
        .map_err(|_| ())?;
    let (_, game_license) = xodus::licensing::content::get_license_content(
        client,
        credentials.device_token,
        credentials.user_token,
        credentials.ticket_reference,
        content_id.into(),
        market.into(),
    )
    .await
    .map_err(|_| ())?;
    let game = SPLicense::parse_base64(&game_license.splicense_block).map_err(|_| ())?;
    let device = tokens.get_device_license().map_err(|_| ())?;
    let device = SPLicense::parse_base64(&device.splicense).map_err(|_| ())?;
    let key = device.encrypted_device_key.ok_or(())?.derive_device_key();
    Ok((key, game))
}
