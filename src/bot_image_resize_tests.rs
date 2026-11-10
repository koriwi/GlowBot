use super::*;
use serde_json::json;

#[test]
fn validates_reference_limits() {
    assert_eq!(reference_limit(&json!({})).unwrap(), None);
    assert_eq!(
        reference_limit(&json!({"reference_max_dimension":null})).unwrap(),
        None
    );
    assert_eq!(
        reference_limit(&json!({"reference_max_dimension":1024})).unwrap(),
        Some(1024)
    );
    for value in [json!(0), json!(-1), json!(16385), json!(1.5), json!("1024")] {
        assert!(reference_limit(&json!({"reference_max_dimension":value})).is_err());
    }
}

fn dimensions(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    (
        u32::from_be_bytes(png[16..20].try_into().unwrap()),
        u32::from_be_bytes(png[20..24].try_into().unwrap()),
    )
}

#[tokio::test]
async fn resizes_without_upscaling_or_modifying_original() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reference.ppm");
    let mut original = b"P6\n80 40\n255\n".to_vec();
    original.extend(vec![128; 80 * 40 * 3]);
    std::fs::write(&path, &original).unwrap();
    assert_eq!(read_reference(&path, None).await.unwrap(), original);
    assert_eq!(
        dimensions(&read_reference(&path, Some(20)).await.unwrap()),
        (20, 10)
    );
    assert_eq!(
        dimensions(&read_reference(&path, Some(100)).await.unwrap()),
        (80, 40)
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::write(&path, b"not an image").unwrap();
    assert!(read_reference(&path, Some(20)).await.is_err());
    assert!(read_reference(&dir.path().join("missing"), Some(20))
        .await
        .is_err());
}
