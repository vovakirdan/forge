use super::*;
use std::sync::mpsc;
use tokio::sync::oneshot;

#[tokio::test]
async fn aborted_capture_keeps_provisioning_and_journal_pinned_until_reader_exits() {
    let directory =
        std::env::temp_dir().join(format!("forge-capture-lifetime-{}", crate::new_id()));
    let registry =
        RunRegistry::new(Journal::open(&directory, "capture-test", 1024 * 1024).unwrap());
    let provisioning = Arc::new(Mutex::new(()));
    let reservation = Arc::clone(&provisioning).try_lock_owned().unwrap();
    let journal = Arc::clone(&registry.journal);
    let (started, reader_started) = oneshot::channel();
    let (release, reader_release) = mpsc::channel();
    let worker = tokio::spawn(protected_capture(reservation, journal, move || {
        started.send(()).unwrap();
        // Bound even a failing test: no permanently blocked runtime worker.
        reader_release.recv_timeout(Duration::from_secs(5)).unwrap();
        Ok(())
    }));
    drop(registry);
    tokio::time::timeout(Duration::from_secs(2), reader_started)
        .await
        .unwrap()
        .unwrap();

    worker.abort();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .unwrap()
            .unwrap_err()
            .is_cancelled()
    );
    assert!(provisioning.try_lock().is_err());
    assert!(matches!(
        Journal::open(&directory, "capture-test", 1024 * 1024),
        Err(SupervisorError::JournalInUse)
    ));

    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(reservation) = provisioning.try_lock() {
                let replacement = Journal::open(&directory, "capture-test", 1024 * 1024).unwrap();
                drop(replacement);
                drop(reservation);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
