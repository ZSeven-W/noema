use noema_core::storage::{FsObjectStore, ObjectStore};

#[tokio::test]
async fn fake_object_store_roundtrips_tenant_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsObjectStore::new(dir.path());
    store
        .put("tenants/personal/deep/user/mem_a.md.zst.enc", b"payload")
        .await
        .unwrap();
    let loaded = store
        .get("tenants/personal/deep/user/mem_a.md.zst.enc")
        .await
        .unwrap();
    assert_eq!(loaded, b"payload");
}
