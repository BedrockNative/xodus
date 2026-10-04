use xodus::{
    licensing::store::{StoreRequest, handle},
    tokens::TokenManager,
};
#[tokio::main]
async fn main() {
    xodus::secrets::init_secrets().expect("keychain initialization");
    let tokens = TokenManager::with_keychain_and_memory();
    for _ in 0..2 {
        let now = std::time::Instant::now();
        let reply = handle(
            &tokens,
            StoreRequest {
                package_family_name: "Microsoft.MinecraftUWP_8wekyb3d8bbwe".into(),
                market: "BR".into(),
                language: "pt-BR".into(),
                operation: "AppReceipt".into(),
                product_id: String::new(),
            },
        )
        .await;
        println!(
            "Store status {:08x}, active {}, trial {}, receipt bytes {}, correct app {}, elapsed {:?}",
            reply.status,
            reply.is_active,
            reply.is_trial,
            reply.receipt.len(),
            reply
                .app_id
                .eq_ignore_ascii_case("d25480ca-36aa-46e6-b76b-39608d49558c"),
            now.elapsed()
        );
        if reply.status != 0 {
            std::process::exit(1);
        }
    }
}
