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
