use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::*;

fn v6(text: &str) -> IpAddr {
    IpAddr::V6(text.parse::<Ipv6Addr>().unwrap())
}

#[test]
fn an_ipv6_client_is_its_64_and_an_ipv4_client_is_itself() {
    let four = IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3));
    assert_eq!(client_key(four), four);
    assert_eq!(
        client_key(v6("2001:db8:1:2:aaaa:bbbb:cccc:dddd")),
        client_key(v6("2001:db8:1:2:1:2:3:4")),
        "two addresses of one /64 are one client"
    );
    assert_ne!(
        client_key(v6("2001:db8:1:2::1")),
        client_key(v6("2001:db8:1:3::1")),
        "another /64 is another client"
    );
    assert_eq!(
        client_key(v6("::ffff:10.1.2.3")),
        four,
        "IPv4-mapped is IPv4"
    );
}

#[test]
fn the_key_comes_from_the_trusted_header_else_from_the_peer() {
    let with_header = ClientIp {
        header: Some(header::HeaderName::from_static("x-real-ip")),
    };
    let request =
        |header: Option<&str>| {
            let mut builder = axum::http::Request::builder();
            if let Some(value) = header {
                builder = builder.header("x-real-ip", value);
            }
            let mut request = builder.body(()).unwrap();
            request.extensions_mut().insert(axum::extract::ConnectInfo(
                std::net::SocketAddr::from(([192, 0, 2, 9], 5000)),
            ));
            request
        };
    assert_eq!(
        with_header.extract(&request(Some("2001:db8::7"))).unwrap(),
        v6("2001:db8::")
    );
    assert_eq!(
        with_header.extract(&request(None)).unwrap(),
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 9)),
        "no header: the peer"
    );
    assert_eq!(
        with_header
            .extract(&request(Some("not an address")))
            .unwrap(),
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 9))
    );
    let peer_only = ClientIp { header: None };
    assert_eq!(
        peer_only.extract(&request(Some("2001:db8::7"))).unwrap(),
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 9)),
        "without server.client_ip_header the header is ignored"
    );
}

#[tokio::test]
async fn the_cleanup_runs_until_the_limiter_is_gone() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    spawn_cleanup(Duration::from_millis(5), move || {
        seen.fetch_add(1, Ordering::SeqCst) < 2
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "it stopped after answering false"
    );
}

#[test]
fn retain_recent_forgets_clients_whose_allowance_is_full_again() {
    let config = GovernorConfigBuilder::default()
        .per_millisecond(5)
        .burst_size(2)
        .key_extractor(ClientIp { header: None })
        .finish()
        .unwrap();
    let limiter = config.limiter().clone();
    for i in 0..1_000u32 {
        let address = IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + i));
        let _ = limiter.check_key(&address);
    }
    assert_eq!(limiter.len(), 1_000);
    std::thread::sleep(Duration::from_millis(50));
    limiter.retain_recent();
    assert_eq!(limiter.len(), 0);
}
