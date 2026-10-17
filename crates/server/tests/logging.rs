//! What the request log holds (AGENTS §11: never log email addresses). One test per binary: it
//! installs the global subscriber.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use common::{Env, db_test};
use server::identity::Role;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("log buffer poisoned"))?
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn the_request_log_has_the_path_and_never_the_query() {
    let captured = Captured::default();
    let sink = captured.clone();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_ansi(false)
            .with_writer(move || sink.clone())
            .finish(),
    )
    .expect("the only test of this binary sets the subscriber");

    db_test(async |pool| {
        let env = Env::new(pool).await;
        let admin = env.user("admin@x.edu", Role::Admin).await;
        // The email filter of the user list is a query parameter. Signed in, and not.
        env.ok(
            "GET",
            &admin.token,
            "/admin/users?email=anna.kowalska@school.edu",
            None,
            200,
        )
        .await;
        env.http
            .get("/admin/users?email=secret.person@school.edu&cursor=abc")
            .send()
            .await;
    })
    .await;

    let log = String::from_utf8(captured.0.lock().expect("log buffer").clone()).expect("UTF-8");
    assert!(
        log.contains("path=/api/v1/admin/users"),
        "the requests are logged by path:\n{log}"
    );
    for leaked in ["anna.kowalska", "secret.person", "school.edu", "cursor=abc"] {
        assert!(!log.contains(leaked), "`{leaked}` is in the log:\n{log}");
    }
}
