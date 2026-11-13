use super::*;

#[test]
fn a_name_cannot_break_out_of_the_disposition() {
    let value = content_disposition("a\"; filename=evil.exe\r\nX: y%.pdf", true);
    let text = value.to_str().expect("an ASCII header value");
    assert_eq!(
        text,
        "inline; filename=\"a__ filename=evil.exe__X: y_.pdf\"; \
         filename*=UTF-8''a%22%3B%20filename%3Devil.exe%0D%0AX%3A%20y%25.pdf"
    );
    let value = content_disposition("Звіт з фізики.pdf", false);
    assert!(
        value
            .to_str()
            .expect("ASCII")
            .starts_with("attachment; filename=\"____ _ ______.pdf\"; filename*=UTF-8''%D0%97")
    );
}
