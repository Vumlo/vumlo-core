use super::*;

fn footer(language: Language) -> Footer<'static> {
    Footer {
        language,
        institution: "Northfield",
        plugin_id: "deadline-change-alert",
        kind: "deadline_changed",
        opt_out_link: "https://lms.test/api/v1/mail/opt-out/T",
    }
}

#[test]
fn the_body_is_text_then_link_then_a_footer_in_both_languages() {
    for language in [Language::En, Language::Uk] {
        let body = plugin_body(
            "The deadline moved.\n\n",
            Some("https://lms.test/courses/1"),
            &footer(language),
        );
        let text = body.find("The deadline moved.").unwrap();
        let link = body.find("https://lms.test/courses/1").unwrap();
        let sig = body.find("-- \n").unwrap();
        let plugin = body.find("deadline-change-alert").unwrap();
        let opt_out = body.find("https://lms.test/api/v1/mail/opt-out/T").unwrap();
        assert!(
            text < link && link < sig && sig < plugin && plugin < opt_out,
            "{body}"
        );
        assert!(body.contains("Northfield") && body.contains("deadline_changed"));
    }
}

#[test]
fn without_a_link_there_is_no_blank_link_line() {
    let body = plugin_body("Hello", None, &footer(Language::En));
    assert!(body.starts_with("Hello\n\n-- \n"), "{body}");
}

#[test]
fn the_answers_name_the_plugin_and_give_the_way_back() {
    for language in [Language::En, Language::Uk] {
        let out = opted_out_answer(language, "p", "https://x.test/in");
        assert!(out.contains("p") && out.contains("https://x.test/in"));
        assert!(opted_in_answer(language, "p").contains('p'));
    }
}
