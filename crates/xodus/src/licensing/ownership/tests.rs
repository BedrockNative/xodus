use super::*;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const PRODUCT: &str = "9NBLGGH2JHXJ";

fn purchased() -> Value {
    json!({"id": "test-entitlement", "productId": PRODUCT, "status": "Active", "acquisitionType": "Single",
        "startDate": "2020-01-01T00:00:00Z", "endDate": "9999-12-31T23:59:59.9999999Z",
        "trialData": {"isTrial": false, "isInTrialPeriod": false}})
}

fn classify(value: Value) -> ItemStatus {
    serde_json::from_value::<CollectionItem>(value)
        .unwrap()
        .classify(PRODUCT, Utc::now())
}

#[test]
fn only_active_non_trial_single_acquisition_is_purchase() {
    assert_eq!(classify(purchased()), ItemStatus::Purchased);
    for (field, value) in [
        ("productId", "000000000000"),
        ("status", "Revoked"),
        ("status", "Expired"),
        ("status", "Suspended"),
        ("status", "Banned"),
        ("acquisitionType", "Recurring"),
        ("acquisitionType", "Conditional"),
        ("startDate", "9998-01-01T00:00:00Z"),
        ("endDate", "2001-01-01T00:00:00Z"),
    ] {
        let mut item = purchased();
        item[field] = json!(value);
        assert_eq!(classify(item), ItemStatus::Other, "{field}={value}");
    }
    for field in ["isTrial", "isInTrialPeriod"] {
        let mut item = purchased();
        item["trialData"][field] = json!(true);
        assert_eq!(classify(item), ItemStatus::Other);
    }
}

#[test]
fn incomplete_purchase_evidence_is_unavailable() {
    for field in ["acquisitionType", "startDate", "endDate", "trialData"] {
        let mut item = purchased();
        item.as_object_mut().unwrap().remove(field);
        assert_eq!(classify(item), ItemStatus::Ambiguous, "missing {field}");
    }
    let mut item = purchased();
    item["acquisitionType"] = json!("FutureUnknownValue");
    assert_eq!(classify(item), ItemStatus::Ambiguous);
    assert!(serde_json::from_value::<CollectionPage>(json!({})).is_err());
}

#[tokio::test]
async fn no_account_and_invalid_input_do_not_need_network() {
    let tokens = TokenManager::with_memory();
    let response = check(
        &tokens,
        &OwnershipRequest {
            product_id: PRODUCT.into(),
        },
    )
    .await;
    assert_eq!(response.status, OwnershipStatus::NoAccount);
    for product in [
        "",
        "../account",
        "https://evil",
        "9nblggh2jhxj",
        "9NBLGGH2JHXJ\n",
    ] {
        assert_eq!(
            check(
                &tokens,
                &OwnershipRequest {
                    product_id: product.into()
                }
            )
            .await
            .status,
            OwnershipStatus::InvalidRequest
        );
    }
}

async fn mock_pages(pages: Vec<(u16, Value)>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        for (index, (status, body)) in pages.into_iter().enumerate() {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                bytes.push(stream.read_u8().await.unwrap());
                if bytes.ends_with(b"\r\n\r\n") {
                    break bytes.len();
                }
                assert!(bytes.len() < 16384);
            };
            let headers = String::from_utf8_lossy(&bytes);
            assert!(headers.contains("test-device-token"));
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap()
                .parse()
                .unwrap();
            bytes.resize(header_end + length, 0);
            stream.read_exact(&mut bytes[header_end..]).await.unwrap();
            let request: Value = serde_json::from_slice(&bytes[header_end..]).unwrap();
            assert_eq!(request["productSkuIds"][0]["productId"], PRODUCT);
            assert_eq!(request["market"], "neutral");
            assert_eq!(
                request["beneficiaries"][0],
                json!({"identityType":"Msa","identityValue":"test-user-token","localTicketReference":"test-ticket"})
            );
            if index > 0 {
                assert_eq!(request["continuationToken"], "next");
            }
            let json = body.to_string();
            stream.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}", json.len()).as_bytes()).await.unwrap();
        }
    });
    (url, task)
}

#[tokio::test]
async fn queries_follow_continuation_before_denying_purchase() {
    let (url, task) = mock_pages(vec![
        (200, json!({"items": [], "continuationToken": "next"})),
        (200, json!({"items": [purchased()]})),
    ])
    .await;
    assert_eq!(
        query_collections(&reqwest::Client::new(), &url, &credentials(), PRODUCT).await,
        Ok(OwnershipStatus::Purchased)
    );
    task.await.unwrap();
}

#[tokio::test]
async fn failures_are_not_reported_as_not_owned() {
    for status in [401, 403, 429, 500] {
        let (url, task) =
            mock_pages(vec![(status, json!({"error":"private upstream detail"}))]).await;
        assert!(
            query_collections(&reqwest::Client::new(), &url, &credentials(), PRODUCT)
                .await
                .is_err()
        );
        task.await.unwrap();
    }
    let (url, task) = mock_pages(vec![(200, json!({"items":[]}))]).await;
    assert_eq!(
        query_collections(&reqwest::Client::new(), &url, &credentials(), PRODUCT).await,
        Ok(OwnershipStatus::NotOwned)
    );
    task.await.unwrap();
}

#[tokio::test]
async fn repeated_continuation_fails_closed() {
    let page = json!({"items": [], "continuationToken": "next"});
    let (url, task) = mock_pages(vec![(200, page.clone()), (200, page)]).await;
    assert!(
        query_collections(&reqwest::Client::new(), &url, &credentials(), PRODUCT)
            .await
            .is_err()
    );
    task.await.unwrap();
}

#[tokio::test]
async fn a_later_failure_or_revocation_cannot_validate_an_earlier_page() {
    let mut revoked = purchased();
    revoked["status"] = json!("Revoked");
    for second in [(500, json!({})), (200, json!({"items": [revoked]}))] {
        let (url, task) = mock_pages(vec![
            (
                200,
                json!({"items": [purchased()], "continuationToken": "next"}),
            ),
            second,
        ])
        .await;
        assert_ne!(
            query_collections(&reqwest::Client::new(), &url, &credentials(), PRODUCT).await,
            Ok(OwnershipStatus::Purchased)
        );
        task.await.unwrap();
    }
}

fn credentials() -> StoreCredentials {
    StoreCredentials {
        device_token: "test-device-token".into(),
        user_token: "test-user-token".into(),
        ticket_reference: "test-ticket".into(),
    }
}
