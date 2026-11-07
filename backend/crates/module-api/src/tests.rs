use sqlx::migrate::Migrator;

use super::*;

struct Fake(&'static Manifest);

impl Module for Fake {
    fn manifest(&self) -> &'static Manifest {
        self.0
    }

    fn register(&self, _: &mut Registrar) {}
}

const fn manifest(id: &'static str, requires: &'static [&'static str]) -> Manifest {
    Manifest {
        id,
        version: "1.0.0",
        requires,
        permissions: &[],
        settings: &[],
        web: false,
    }
}

static A: Manifest = manifest("a", &[]);
static B: Manifest = manifest("b", &["a"]);
static C: Manifest = manifest("c", &["b", "a"]);
static D: Manifest = manifest("d", &[]);
static LONELY: Manifest = manifest("lonely", &["absent"]);
static LEFT: Manifest = manifest("left", &["right"]);
static RIGHT: Manifest = manifest("right", &["left"]);
static BAD: Manifest = manifest("Not-Valid", &[]);

fn ids(order: Vec<&dyn Module>) -> Vec<&'static str> {
    order.iter().map(|m| m.manifest().id).collect()
}

#[test]
fn modules_start_after_what_they_require_and_ties_go_by_id() {
    let (c, d, b, a) = (Fake(&C), Fake(&D), Fake(&B), Fake(&A));
    let order = init_order(&[&c, &d, &b, &a]).unwrap();
    assert_eq!(ids(order), ["a", "b", "c", "d"]);
    // The same set in any input order gives the same answer.
    let order = init_order(&[&a, &b, &c, &d]).unwrap();
    assert_eq!(ids(order), ["a", "b", "c", "d"]);
}

#[test]
fn a_module_that_requires_one_not_compiled_in_stops_the_start_and_names_both() {
    let lonely = Fake(&LONELY);
    let e = init_order(&[&lonely]).err().unwrap();
    assert_eq!(
        e,
        OrderError::Missing {
            module: "lonely".to_owned(),
            missing: "absent".to_owned()
        }
    );
    let message = e.to_string();
    assert!(
        message.contains("lonely") && message.contains("absent"),
        "{message}"
    );
}

#[test]
fn a_cycle_of_requirements_is_refused_and_names_the_modules() {
    let (left, right, a) = (Fake(&LEFT), Fake(&RIGHT), Fake(&A));
    let e = init_order(&[&left, &right, &a]).err().unwrap();
    assert_eq!(
        e,
        OrderError::Cycle(vec!["left".to_owned(), "right".to_owned()])
    );
}

#[test]
fn an_id_that_cannot_be_a_schema_name_and_a_duplicate_are_refused() {
    let (bad, a, again) = (Fake(&BAD), Fake(&A), Fake(&A));
    assert_eq!(
        init_order(&[&bad]).err().unwrap(),
        OrderError::InvalidId("Not-Valid".to_owned())
    );
    assert_eq!(
        init_order(&[&a, &again]).err().unwrap(),
        OrderError::Duplicate("a".to_owned())
    );
}

#[test]
fn a_scoped_migrator_creates_its_schema_and_keeps_its_history_inside_it() {
    let migrator = scoped_migrator("dummy", Migrator::DEFAULT);
    assert_eq!(migrator.table_name, "mod_dummy._sqlx_migrations");
    assert_eq!(
        migrator
            .create_schemas
            .iter()
            .map(|s| s.as_ref())
            .collect::<Vec<_>>(),
        ["mod_dummy"]
    );
}

#[test]
fn the_spec_carries_what_the_configuration_checks() {
    static WEB: Manifest = Manifest {
        web: true,
        ..manifest("w", &["a"])
    };
    let spec = WEB.spec();
    assert_eq!(
        (spec.name, spec.requires, spec.web),
        ("w", &["a"][..], true)
    );
    assert_eq!(WEB.schema(), "mod_w");
}

// ---------------------------------------------------------------------------------------------
// The outbound client
// ---------------------------------------------------------------------------------------------

use std::time::Duration;

async fn server() -> String {
    use axum::Router;
    use axum::body::Bytes;
    use axum::http::{HeaderMap, Method, header};
    use axum::response::IntoResponse;
    use axum::routing::any;
    let app = Router::new()
        .route(
            "/echo",
            any(
                |method: Method, headers: HeaderMap, body: Bytes| async move {
                    let tag = headers
                        .get("x-tag")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    (
                        [
                            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                            (header::CONTENT_DISPOSITION, "attachment; filename=a.txt"),
                        ],
                        format!("{method} {tag} {}", String::from_utf8_lossy(&body)),
                    )
                },
            ),
        )
        .route("/big", any(|| async { vec![b'x'; 2048].into_response() }))
        .route(
            "/slow",
            any(|| async {
                tokio::time::sleep(Duration::from_secs(3)).await;
                "late"
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    url
}

fn client(max: usize, total: Duration) -> OutboundClient {
    OutboundClient::new(Duration::from_secs(1), total, max, 0).unwrap()
}

#[tokio::test]
async fn a_request_of_any_method_sends_its_headers_and_body_and_returns_the_two_headers_to_pass_on()
{
    let base = server().await;
    let client = client(1024, Duration::from_secs(2));
    for method in ["GET", "POST", "PUT", "PATCH", "DELETE"] {
        let body = if method == "GET" {
            Vec::new()
        } else {
            b"payload".to_vec()
        };
        let reply = client
            .request(
                method,
                &format!("{base}/echo"),
                &[("x-tag", "t1")],
                body.clone(),
            )
            .await
            .unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(
            String::from_utf8(reply.body).unwrap(),
            format!("{method} t1 {}", String::from_utf8(body).unwrap())
        );
        assert_eq!(
            reply.content_type.as_deref(),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(
            reply.content_disposition.as_deref(),
            Some("attachment; filename=a.txt")
        );
    }
    assert!(matches!(
        client
            .request("NOT A METHOD", &format!("{base}/echo"), &[], Vec::new())
            .await,
        Err(OutboundError::Request(_))
    ));
}

#[tokio::test]
async fn an_answer_larger_than_the_limit_and_one_that_takes_too_long_are_refused() {
    let base = server().await;
    assert!(matches!(
        client(1024, Duration::from_secs(2))
            .request("GET", &format!("{base}/big"), &[], Vec::new())
            .await,
        Err(OutboundError::TooLarge { limit: 1024 })
    ));
    assert!(matches!(
        client(1024, Duration::from_millis(300))
            .request("GET", &format!("{base}/slow"), &[], Vec::new())
            .await,
        Err(OutboundError::Timeout)
    ));
    // The old calls still work and say what they did.
    let (status, body) = client(1024, Duration::from_secs(2))
        .get(&format!("{base}/echo"))
        .await
        .unwrap();
    assert_eq!((status, body.starts_with(b"GET")), (200, true));
}
