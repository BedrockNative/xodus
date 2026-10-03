#![cfg(unix)]
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UnixListener},
    process::Command,
};
use xodus::licensing::ownership::{OwnershipResponse, OwnershipStatus};

#[tokio::test]
async fn unverified_purchase_never_downloads_or_creates_destination() {
    for status in [
        OwnershipStatus::NotOwned,
        OwnershipStatus::NoAccount,
        OwnershipStatus::Unavailable,
        OwnershipStatus::InvalidRequest,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("test.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let source = format!("http://{}/game.msixvc", http.local_addr().unwrap());
        let destination = temp.path().join("game");
        let service = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = [0; 8];
            stream.read_exact(&mut header).await.unwrap();
            let mut bytes = vec![0; u16::from_le_bytes([header[6], header[7]]) as usize];
            stream.read_exact(&mut bytes).await.unwrap();
            let response = OwnershipResponse {
                product_id: "9NBLGGH2JHXJ".into(),
                status,
            };
            let xml = quick_xml::se::to_string(&response).unwrap();
            stream
                .write_all(
                    &xodus::ipc::encode_message(xodus::ipc::XML_MAGIC, 6, xml.as_bytes()).unwrap(),
                )
                .await
                .unwrap();
        });
        let output = Command::new(env!("CARGO_BIN_EXE_xodus-cli"))
            .args(["install-owned", "9NBLGGH2JHXJ", &source])
            .arg(&destination)
            .env("XODUS_SOCKET", &socket)
            .env("XODUS_CONFIG_DIR", temp.path().join("profile"))
            .env("XODUS_LOG", "off")
            .output()
            .await
            .unwrap();
        assert!(!output.status.success());
        assert!(!destination.exists());
        assert!(
            !temp.path().join("profile").exists(),
            "Gate must precede keychain initialization"
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), http.accept())
                .await
                .is_err()
        );
        service.await.unwrap();
    }
}
