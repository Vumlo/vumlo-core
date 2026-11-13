use std::sync::Arc;

use object_store::ObjectStore;
use object_store::memory::InMemory;
use sqlx::PgPool;
use uuid::Uuid;

use super::*;
use test_support::db_test;

async fn is_empty(store: &InMemory) -> bool {
    let listing = store.list_with_delimiter(None).await.unwrap();
    listing.objects.is_empty() && listing.common_prefixes.is_empty()
}

const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn memory() -> (Storage, Arc<InMemory>) {
    let store = Arc::new(InMemory::new());
    (Storage::from_store(store.clone()), store)
}

async fn stored_file(pool: &PgPool, storage: &Storage, data: &'static [u8]) -> File {
    let stored = put(storage, Uuid::now_v7(), data, 1024).await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    insert(&mut conn, &stored, Uuid::now_v7(), "a.txt", "text/plain")
        .await
        .unwrap()
}

async fn bytes_of(storage: &Storage, file: &File, range: Option<std::ops::Range<u64>>) -> Vec<u8> {
    read(storage, file, range)
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap()
        .to_vec()
}

#[tokio::test]
async fn put_then_insert_then_read_round_trips_bytes() {
    db_test(async |pool| {
        let (storage, _) = memory();
        let file = stored_file(&pool, &storage, b"abc").await;

        assert_eq!(file.size, 3);
        assert_eq!(file.sha256, ABC_SHA256);
        assert_eq!(bytes_of(&storage, &file, None).await, b"abc");
        let mut conn = pool.acquire().await.unwrap();
        assert_eq!(
            get(&mut conn, file.id).await.unwrap().unwrap().name,
            "a.txt"
        );
        assert!(get(&mut conn, Uuid::now_v7()).await.unwrap().is_none());
    })
    .await;
}

#[tokio::test]
async fn put_keeps_the_first_512_bytes_as_head() {
    let (storage, _) = memory();
    let stored = put(&storage, Uuid::now_v7(), vec![7u8; 600], 1024)
        .await
        .unwrap();
    assert_eq!(stored.head, vec![7u8; 512]);
    assert_eq!(stored.size, 600);
}

#[tokio::test]
async fn put_rejects_empty_data() {
    let (storage, _) = memory();
    let error = put(&storage, Uuid::now_v7(), Vec::<u8>::new(), 1024)
        .await
        .unwrap_err();
    assert!(matches!(error, FilesError::Empty), "{error}");
}

#[tokio::test]
async fn put_rejects_more_than_max_size_and_stores_nothing() {
    let (storage, store) = memory();
    let error = put(&storage, Uuid::now_v7(), b"abcd".to_vec(), 3)
        .await
        .unwrap_err();
    assert!(matches!(error, FilesError::TooLarge), "{error}");
    assert!(is_empty(&store).await);
}

#[tokio::test]
async fn read_with_range_returns_only_that_slice() {
    db_test(async |pool| {
        let (storage, _) = memory();
        let file = stored_file(&pool, &storage, b"0123456789").await;

        assert_eq!(bytes_of(&storage, &file, Some(2..5)).await, b"234");
    })
    .await;
}

#[tokio::test]
async fn delete_removes_row_and_object() {
    db_test(async |pool| {
        let (storage, store) = memory();
        let file = stored_file(&pool, &storage, b"abc").await;
        let mut conn = pool.acquire().await.unwrap();

        delete(&mut conn, &storage, file.id).await.unwrap();

        assert!(get(&mut conn, file.id).await.unwrap().is_none());
        assert!(is_empty(&store).await);
    })
    .await;
}

#[tokio::test]
async fn candidates_lists_only_rows_older_than_cutoff() {
    db_test(async |pool| {
        let (storage, _) = memory();
        let old = stored_file(&pool, &storage, b"old").await;
        let fresh = stored_file(&pool, &storage, b"new").await;
        sqlx::query!(
            "UPDATE files.files SET created_at = now() - interval '2 days' WHERE id = $1",
            old.id
        )
        .execute(&pool)
        .await
        .unwrap();
        let mut conn = pool.acquire().await.unwrap();

        let found = candidates(&mut conn, chrono::Utc::now() - chrono::Duration::days(1))
            .await
            .unwrap();

        assert_eq!(found, [old.id]);
        assert!(!found.contains(&fresh.id));
    })
    .await;
}

#[tokio::test]
async fn lock_returns_the_rows_that_exist_in_id_order() {
    db_test(async |pool| {
        let (storage, _) = memory();
        let first = stored_file(&pool, &storage, b"one").await;
        let second = stored_file(&pool, &storage, b"two").await;
        let mut tx = pool.begin().await.unwrap();

        let locked = lock(&mut tx, &[second.id, Uuid::now_v7(), first.id])
            .await
            .unwrap();

        assert_eq!(
            locked.iter().map(|file| file.id).collect::<Vec<_>>(),
            [first.id, second.id],
            "ascending id, the unknown id absent"
        );
    })
    .await;
}

#[tokio::test]
async fn lock_takes_the_rows_in_id_order_even_when_they_are_stored_in_another() {
    db_test(async |pool| {
        let (storage, _) = memory();
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let (low, high) = if a < b { (a, b) } else { (b, a) };
        // The row with the higher id is stored first, so a scan without an order meets it first.
        let mut conn = pool.acquire().await.unwrap();
        for id in [high, low] {
            let stored = put(&storage, id, b"x".as_slice(), 1024).await.unwrap();
            insert(&mut conn, &stored, Uuid::now_v7(), "a.txt", "text/plain")
                .await
                .unwrap();
        }
        drop(conn);
        let mut holder = pool.begin().await.unwrap();
        lock(&mut holder, &[low]).await.unwrap();

        let waiting = tokio::spawn({
            let pool = pool.clone();
            async move {
                let mut tx = pool.begin().await.unwrap();
                lock(&mut tx, &[high, low]).await.unwrap().len()
            }
        });
        test_support::wait_until_blocked(&pool, &waiting).await;

        // It waits for `low`, the first of its rows, and has not taken `high` yet: two callers
        // that name the same files in opposite orders cannot hold one each.
        let mut probe = pool.begin().await.unwrap();
        let free = sqlx::query_scalar!(
            "SELECT id FROM files.files WHERE id = $1 FOR UPDATE NOWAIT",
            high
        )
        .fetch_one(&mut *probe)
        .await;
        assert!(free.is_ok(), "{free:?}");
        probe.rollback().await.unwrap();
        holder.commit().await.unwrap();
        assert_eq!(waiting.await.unwrap(), 2);
    })
    .await;
}

#[tokio::test]
async fn a_second_lock_waits_until_the_first_transaction_ends() {
    db_test(async |pool| {
        let (storage, _) = memory();
        let file = stored_file(&pool, &storage, b"one").await;
        let mut first = pool.begin().await.unwrap();
        lock(&mut first, &[file.id]).await.unwrap();

        let second = tokio::spawn({
            let pool = pool.clone();
            async move {
                let mut tx = pool.begin().await.unwrap();
                lock(&mut tx, &[file.id]).await.unwrap().len()
            }
        });
        test_support::wait_until_blocked(&pool, &second).await;
        first.commit().await.unwrap();

        assert_eq!(second.await.unwrap(), 1);
    })
    .await;
}

#[tokio::test]
async fn try_lock_skips_a_locked_row() {
    db_test(async |pool| {
        let (storage, _) = memory();
        let file = stored_file(&pool, &storage, b"one").await;
        let mut holder = pool.begin().await.unwrap();
        lock(&mut holder, &[file.id]).await.unwrap();
        let mut other = pool.begin().await.unwrap();

        assert!(try_lock(&mut other, file.id).await.unwrap().is_none());
        assert!(
            try_lock(&mut other, Uuid::now_v7())
                .await
                .unwrap()
                .is_none()
        );
        holder.commit().await.unwrap();
        assert!(try_lock(&mut other, file.id).await.unwrap().is_some());
    })
    .await;
}

#[tokio::test]
async fn local_storage_creates_its_directory_and_keeps_the_bytes() {
    db_test(async |pool| {
        let dir = std::env::temp_dir().join(format!("vumlo-files-{}", Uuid::now_v7()));
        let storage = Storage::local(&dir.join("files")).unwrap();
        let file = stored_file(&pool, &storage, b"abc").await;

        assert_eq!(bytes_of(&storage, &file, None).await, b"abc");
        std::fs::remove_dir_all(&dir).unwrap();
    })
    .await;
}
