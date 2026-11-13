use super::*;

#[test]
fn the_search_text_is_matched_as_it_is() {
    assert_eq!(like_pattern("maya"), "%maya%");
    assert_eq!(like_pattern("100%_x\\"), "%100\\%\\_x\\\\%");
}
