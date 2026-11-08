use super::*;

#[test]
fn a_route_gets_its_burst_then_one_call_per_interval_and_each_route_has_its_own() {
    let limiter = RouteLimiter::new(3, 2);
    let (a, b) = ((Uuid::now_v7(), 0), (Uuid::now_v7(), 0));
    let start = Instant::now();
    for _ in 0..3 {
        assert!(limiter.check_at(a, start).is_ok());
    }
    let wait = limiter.check_at(a, start).unwrap_err();
    assert_eq!(wait, Duration::from_millis(500), "two calls a second");
    // Another route is not touched by it.
    assert!(limiter.check_at(b, start).is_ok());
    // Half a second later one call has come back, and only one.
    let later = start + Duration::from_millis(500);
    assert!(limiter.check_at(a, later).is_ok());
    assert!(limiter.check_at(a, later).is_err());
    // A long quiet never gives back more than the burst.
    let much_later = start + Duration::from_secs(3600);
    for _ in 0..3 {
        assert!(limiter.check_at(a, much_later).is_ok());
    }
    assert!(limiter.check_at(a, much_later).is_err());
}
