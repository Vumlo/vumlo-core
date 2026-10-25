use super::*;

#[test]
fn page_params_default_clamp_and_parse_the_cursor() {
    let default = PageParams::new(None, None).unwrap();
    assert_eq!((default.after, default.limit), (None, 50));
    assert_eq!(PageParams::new(None, Some(1000)).unwrap().limit, 100);
    assert_eq!(PageParams::new(None, Some(0)).unwrap().limit, 1);
    let id = Uuid::now_v7();
    assert_eq!(
        PageParams::new(Some(&id.to_string()), Some(5))
            .unwrap()
            .after,
        Some(id)
    );
    assert_eq!(PageParams::new(Some("junk"), None), Err(InvalidCursor));
}

#[test]
fn page_from_rows_reports_a_next_cursor_only_when_there_is_more() {
    let ids: Vec<Uuid> = (0..3).map(|_| Uuid::now_v7()).collect();
    let full = Page::from_rows(ids.clone(), 2, |id| *id);
    assert_eq!(full.items, ids[..2]);
    assert_eq!(full.next, Some(ids[1]));
    let last = Page::from_rows(ids[..2].to_vec(), 2, |id| *id);
    assert_eq!(last.items.len(), 2);
    assert_eq!(last.next, None);
    assert_eq!(Page::from_rows(Vec::<Uuid>::new(), 2, |id| *id).next, None);
}

#[test]
fn a_bad_cursor_is_a_422_on_the_cursor_field() {
    use axum::response::IntoResponse;
    let response = ApiError::from(InvalidCursor).into_response();
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
}
