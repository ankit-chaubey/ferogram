// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
//
// Tests for resumable transfer checkpoint logic.
// These tests cover CheckpointStore, DownloadCheckpoint, and UploadCheckpoint
// without requiring a real Telegram connection.

#![cfg(feature = "experimental")]

use ferogram::resume::{
    CheckpointStore, DownloadCheckpoint, UPLOAD_SESSION_TTL_MS, UploadCheckpoint, download_key,
    now_ms, sha256_hex, upload_key,
};

async fn tmp_store() -> (tempfile::TempDir, CheckpointStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    // CheckpointStore::open expects a *file path* and uses its parent directory.
    // Create a dummy session file path inside the temp dir.
    let session_path = dir.path().join("test.session");
    let store = CheckpointStore::open(&session_path)
        .await
        .expect("open store");
    (dir, store)
}

#[tokio::test]
async fn download_checkpoint_roundtrip() {
    let (_dir, store) = tmp_store().await;

    let cp = DownloadCheckpoint {
        key: "abc123".into(),
        offset: 4 * 1024 * 1024, // 4 MB
        total: 20 * 1024 * 1024,
        sha256_partial: String::new(),
    };

    store.save_download(&cp).await;

    let loaded = store.load_download("abc123").await.expect("should load");
    assert_eq!(loaded.offset, cp.offset);
    assert_eq!(loaded.total, cp.total);
}

#[tokio::test]
async fn download_checkpoint_delete() {
    let (_dir, store) = tmp_store().await;

    let cp = DownloadCheckpoint {
        key: "del_key".into(),
        offset: 1024,
        total: 8192,
        sha256_partial: String::new(),
    };
    store.save_download(&cp).await;
    assert!(store.load_download("del_key").await.is_some());

    store.delete_download("del_key").await;
    assert!(store.load_download("del_key").await.is_none());
}

#[tokio::test]
async fn download_missing_returns_none() {
    let (_dir, store) = tmp_store().await;
    assert!(store.load_download("nonexistent").await.is_none());
}

#[tokio::test]
async fn partial_path_is_deterministic() {
    let (_dir, store) = tmp_store().await;
    let p1 = store.partial_path("mykey");
    let p2 = store.partial_path("mykey");
    assert_eq!(p1, p2);
    assert!(p1.to_str().unwrap().contains("mykey"));
}

#[tokio::test]
async fn partial_file_write_and_read() {
    let (_dir, store) = tmp_store().await;
    let key = "partial_test";
    let bytes: Vec<u8> = (0u8..=255).collect();

    let path = store.partial_path(key);
    tokio::fs::write(&path, &bytes)
        .await
        .expect("write partial");

    let read_back = tokio::fs::read(&path).await.expect("read partial");
    assert_eq!(read_back, bytes);

    // Cleanup removes the file.
    tokio::fs::remove_file(&path).await.expect("remove partial");
    assert!(!path.exists());
}

#[tokio::test]
async fn upload_checkpoint_roundtrip() {
    let (_dir, store) = tmp_store().await;

    let cp = UploadCheckpoint {
        key: "up_key".into(),
        file_id: 123456789,
        last_part: 42,
        total_parts: 100,
        part_size: 512 * 1024,
        total: 50 * 1024 * 1024,
        big: true,
        name: "video.mp4".into(),
        mime_type: "video/mp4".into(),
        started_ms: now_ms(),
    };

    store.save_upload(&cp).await;

    let loaded = store.load_upload("up_key").await.expect("should load");
    assert_eq!(loaded.file_id, cp.file_id);
    assert_eq!(loaded.last_part, 42);
    assert_eq!(loaded.total_parts, 100);
    assert_eq!(loaded.mime_type, "video/mp4");
}

#[tokio::test]
async fn upload_checkpoint_delete() {
    let (_dir, store) = tmp_store().await;

    let cp = UploadCheckpoint {
        key: "del_up".into(),
        file_id: 1,
        last_part: 0,
        total_parts: 10,
        part_size: 1024,
        total: 10240,
        big: false,
        name: "file.bin".into(),
        mime_type: "application/octet-stream".into(),
        started_ms: now_ms(),
    };
    store.save_upload(&cp).await;
    assert!(store.load_upload("del_up").await.is_some());

    store.delete_upload("del_up").await;
    assert!(store.load_upload("del_up").await.is_none());
}

#[tokio::test]
async fn upload_session_ttl_expired_detection() {
    // Simulate an expired checkpoint by backdating started_ms.
    let started_ms = now_ms().saturating_sub(UPLOAD_SESSION_TTL_MS + 1000);
    let age = now_ms().saturating_sub(started_ms);
    assert!(age >= UPLOAD_SESSION_TTL_MS, "should be expired");
}

#[tokio::test]
async fn upload_session_ttl_fresh_detection() {
    let started_ms = now_ms().saturating_sub(60_000); // 1 minute ago
    let age = now_ms().saturating_sub(started_ms);
    assert!(age < UPLOAD_SESSION_TTL_MS, "should still be valid");
}

#[test]
fn upload_key_stable() {
    let data = vec![0u8; 1024];
    let k1 = upload_key(&data, "test.mp4");
    let k2 = upload_key(&data, "test.mp4");
    assert_eq!(k1, k2);
}

#[test]
fn upload_key_differs_by_name() {
    let data = vec![0u8; 1024];
    let k1 = upload_key(&data, "a.mp4");
    let k2 = upload_key(&data, "b.mp4");
    assert_ne!(k1, k2);
}

#[test]
fn upload_key_differs_by_content() {
    let data1 = vec![0u8; 1024];
    let data2 = vec![1u8; 1024];
    let k1 = upload_key(&data1, "file.bin");
    let k2 = upload_key(&data2, "file.bin");
    assert_ne!(k1, k2);
}

#[test]
fn sha256_hex_stable() {
    let h1 = sha256_hex(b"hello ferogram");
    let h2 = sha256_hex(b"hello ferogram");
    assert_eq!(h1, h2);
    assert_eq!(h1.len(), 64); // 32 bytes hex
}

#[test]
fn sha256_hex_differs() {
    let h1 = sha256_hex(b"aaa");
    let h2 = sha256_hex(b"bbb");
    assert_ne!(h1, h2);
}

#[test]
fn download_resume_offset_alignment() {
    // download_resumable aligns restored bytes down to 1 MB boundary.
    // Verify the formula used in the implementation.
    let mb = 1024 * 1024i64;

    let cases = [
        (0i64, 0i64),
        (512 * 1024, 0),                // 512 KB → align to 0
        (1024 * 1024, 1024 * 1024),     // exactly 1 MB → stays
        (1024 * 1024 + 1, 1024 * 1024), // 1 MB + 1 byte → 1 MB
        (3 * mb + mb / 2, 3 * mb),      // 3.5 MB → 3 MB
        (10 * mb, 10 * mb),             // exact boundary
    ];

    for (restored, expected) in cases {
        let aligned = (restored / mb) * mb;
        assert_eq!(
            aligned, expected,
            "restored={restored} expected aligned={expected} got={aligned}"
        );
    }
}

#[test]
fn download_overlap_skip_calculation() {
    // When we resume at aligned_offset < already, we skip the overlap in `tail`.
    // skip = (already - resume_offset).max(0)
    let already = 3 * 1024 * 1024i64 + 200 * 1024; // 3.2 MB in dest
    let mb = 1024 * 1024i64;
    let resume_offset = (already / mb) * mb; // aligned to 3 MB

    // tail starts at resume_offset=3 MB, dest already has 3.2 MB
    // so skip = 3.2 MB - 3 MB = 200 KB
    let skip = (already - resume_offset).max(0) as usize;
    assert_eq!(skip, 200 * 1024);
}
