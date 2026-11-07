use chrono::TimeZone;
use uuid::Uuid;

use super::*;

#[test]
fn every_persona_has_its_own_token_and_permission() {
    let tokens: Vec<_> = Persona::ALL.iter().map(|p| p.token()).collect();
    assert_eq!(tokens, ["student", "teacher", "admin"]);
    for p in Persona::ALL {
        assert_eq!(p.permission(), format!("chat.{}", p.token()));
    }
}

#[test]
fn the_prompt_names_the_time_the_page_and_the_ids_and_a_student_gets_a_coach() {
    let now = Utc.with_ymd_and_hms(2026, 11, 6, 9, 30, 0).unwrap();
    let course = Uuid::now_v7();
    let ctx = PageContext {
        page: Some("/courses".to_owned()),
        course_id: Some(course),
        item_id: None,
    };
    let prompt = Persona::Student.system_prompt(now, &ctx);
    assert!(prompt.contains("2026-11-06T09:30:00+00:00"), "{prompt}");
    assert!(prompt.contains("/courses") && prompt.contains(&course.to_string()));
    assert!(prompt.contains("Never write the answer"), "{prompt}");
    assert!(!prompt.contains("item they are looking at"));
    assert!(
        !Persona::Teacher
            .system_prompt(now, &ctx)
            .contains("Never write the answer")
    );
}
