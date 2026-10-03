//! Small, versioned ownership extension of the existing WineGDK XML protocol.
use crate::{
    licensing::ownership::{OwnershipRequest, OwnershipResponse},
    proto::xodus::XodusMessageType,
};
use std::{io, path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

pub const XML_MAGIC: u32 = 0x58445358;

pub fn socket_path() -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("XODUS_SOCKET").filter(|s| !s.is_empty()) {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(io::Error::other("XODUS_SOCKET must be absolute"));
        }
        return Ok(path);
    }
    let name = std::env::var("XODUS_SOCK_NAME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "xodus.sock".into());
    if name.contains('/') || name == "." || name == ".." {
        return Err(io::Error::other("XODUS_SOCK_NAME must be a file name"));
    }
    #[cfg(target_os = "macos")]
    let runtime = PathBuf::from("/tmp");
    #[cfg(not(target_os = "macos"))]
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| io::Error::other("XDG_RUNTIME_DIR must be an absolute path"))?;
    Ok(runtime.join(name))
}

pub fn encode_message(magic: u32, kind: u16, payload: &[u8]) -> io::Result<Vec<u8>> {
    let size =
        u16::try_from(payload.len()).map_err(|_| io::Error::other("IPC payload too large"))?;
    let mut frame = Vec::with_capacity(8 + payload.len());
    frame.extend(magic.to_le_bytes());
    frame.extend(kind.to_le_bytes());
    frame.extend(size.to_le_bytes());
    frame.extend(payload);
    Ok(frame)
}

/// Uses only IPC. Never falls back to a different profile/socket or opens a keychain.
pub async fn check_ownership(request: &OwnershipRequest) -> io::Result<OwnershipResponse> {
    let path = socket_path()?;
    tokio::time::timeout(Duration::from_secs(50), async {
        let mut socket = UnixStream::connect(path).await?;
        exchange_ownership(&mut socket, request).await
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Ownership query timed out"))?
}

async fn exchange_ownership(
    socket: &mut UnixStream,
    request: &OwnershipRequest,
) -> io::Result<OwnershipResponse> {
    if !request.is_valid() {
        return Err(io::Error::other("Invalid Store product ID"));
    }
    let xml = quick_xml::se::to_string(request).map_err(io::Error::other)?;
    socket
        .write_all(&encode_message(
            XML_MAGIC,
            XodusMessageType::OwnershipRequest as u16,
            xml.as_bytes(),
        )?)
        .await?;
    let magic = socket.read_u32_le().await?;
    let kind = socket.read_u16_le().await?;
    let size = socket.read_u16_le().await? as usize;
    if magic != XML_MAGIC
        || kind != XodusMessageType::OwnershipResponse as u16
        || size == 0
        || size > 4096
    {
        return Err(io::Error::other(
            "Invalid ownership response; update xodus-service",
        ));
    }
    let mut bytes = vec![0; size];
    socket.read_exact(&mut bytes).await?;
    let response: OwnershipResponse =
        quick_xml::de::from_reader(bytes.as_slice()).map_err(io::Error::other)?;
    if response.product_id != request.product_id {
        return Err(io::Error::other("Ownership product mismatch"));
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::licensing::ownership::OwnershipStatus;

    #[test]
    fn frame_length_cannot_wrap() {
        assert!(encode_message(XML_MAGIC, 5, &vec![0; 65536]).is_err());
        assert_eq!(
            &encode_message(XML_MAGIC, 5, b"abc").unwrap()[..8],
            &[88, 83, 68, 88, 5, 0, 3, 0]
        );
    }

    async fn serve(mut socket: UnixStream, product: &str, kind: u16) {
        let mut header = [0; 8];
        socket.read_exact(&mut header).await.unwrap();
        assert_eq!(header[4], 5);
        let mut request = vec![0; u16::from_le_bytes([header[6], header[7]]) as usize];
        socket.read_exact(&mut request).await.unwrap();
        let xml = quick_xml::se::to_string(&OwnershipResponse {
            product_id: product.into(),
            status: OwnershipStatus::Purchased,
        })
        .unwrap();
        // Unix sockets are byte streams: exercise fragmented headers/payloads.
        for byte in encode_message(XML_MAGIC, kind, xml.as_bytes()).unwrap() {
            socket.write_all(&[byte]).await.unwrap();
        }
    }

    #[tokio::test]
    async fn fragmented_response_round_trip() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let task = tokio::spawn(async { serve(server, "9NBLGGH2JHXJ", 6).await });
        let response = exchange_ownership(
            &mut client,
            &OwnershipRequest {
                product_id: "9NBLGGH2JHXJ".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(response.status, OwnershipStatus::Purchased);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn response_for_another_product_is_rejected() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let task = tokio::spawn(async { serve(server, "000000000000", 6).await });
        assert!(
            exchange_ownership(
                &mut client,
                &OwnershipRequest {
                    product_id: "9NBLGGH2JHXJ".into()
                }
            )
            .await
            .is_err()
        );
        task.await.unwrap();
    }
}
