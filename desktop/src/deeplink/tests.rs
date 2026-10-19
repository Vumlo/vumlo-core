use super::*;

const ID: &str = "0190f1c2-7b1e-7a3c-9d4e-123456789abc";
const OTHER: &str = "0190f1c2-7b1e-7a3c-9d4e-aaaaaaaaaaaa";

#[test]
fn a_link_names_one_of_the_screens_of_the_app() {
    let route = |link: &str| route_of(link, "vumlo-ucu");
    assert_eq!(route("vumlo-ucu://home").as_deref(), Some("/"));
    assert_eq!(route("vumlo-ucu://calendar/").as_deref(), Some("/calendar"));
    assert_eq!(
        route("vumlo-ucu://course?id=0190f1c2-7b1e-7a3c-9d4e-123456789abc").as_deref(),
        Some("/course?id=0190f1c2-7b1e-7a3c-9d4e-123456789abc")
    );
    assert_eq!(
        route(&format!("vumlo-ucu://item?course={ID}&id={OTHER}")).as_deref(),
        Some(format!("/course/item?id={ID}&item={OTHER}").as_str())
    );
}

#[test]
fn anything_else_is_dropped() {
    let route = |link: &str| route_of(link, "vumlo-ucu");
    for link in [
        "vumlo://home",
        "https://example.edu/course?id=x",
        "vumlo-ucu://course",
        "vumlo-ucu://course?id=../admin",
        "vumlo-ucu://course?id=0190f1c2-7b1e-7a3c-9d4e-123456789abc/../x",
        "vumlo-ucu://course?id=%2e%2e",
        "vumlo-ucu://admin/people",
        "vumlo-ucu://item?id=0190f1c2-7b1e-7a3c-9d4e-123456789abc",
        "vumlo-ucu:/home",
        "vumlo-ucuX://home",
    ] {
        assert_eq!(route(link), None, "{link}");
    }
}

#[test]
fn the_code_of_a_sign_in_link_is_taken_and_nothing_else() {
    let code = |link: &str| google_code(link, "vumlo-ucu");
    assert_eq!(
        code("vumlo-ucu://auth/google?code=AbC-123_x").as_deref(),
        Some("AbC-123_x")
    );
    assert_eq!(
        code("vumlo-ucu://auth/google/?code=a#frag").as_deref(),
        Some("a")
    );
    for link in [
        "vumlo-ucu://auth/google",
        "vumlo-ucu://auth/google?code=",
        "vumlo-ucu://auth/google?code=a%2Fb",
        "vumlo-ucu://auth/google?code=a b",
        "vumlo-ucu://auth/other?code=a",
        "vumlo://auth/google?code=a",
        "vumlo-ucu://home?code=a",
    ] {
        assert_eq!(code(link), None, "{link}");
    }
    let long = format!("vumlo-ucu://auth/google?code={}", "a".repeat(129));
    assert_eq!(code(&long), None);
    assert_eq!(
        route_of("vumlo-ucu://auth/google?code=abc", "vumlo-ucu"),
        None,
        "not a screen"
    );
}
