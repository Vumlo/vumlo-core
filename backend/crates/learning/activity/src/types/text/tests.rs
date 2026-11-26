use api_types::Feature;
use serde_json::Value;

use super::*;
use crate::types::no_submission::NoSubmission;
use crate::{ActivityType, EffectiveProfiles};

fn features(config: Value) -> Result<Option<Vec<Feature>>, ConfigError> {
    editor_features(config.as_object().unwrap())
}

#[test]
fn the_editor_features_of_a_text_config_are_a_list_of_known_unique_names() {
    assert_eq!(features(serde_json::json!({})), Ok(None));
    assert_eq!(features(serde_json::json!({"editor": {}})), Ok(None));
    assert_eq!(
        features(serde_json::json!({"editor": {"features": ["code", "headings"]}})),
        Ok(Some(vec![Feature::Code, Feature::Headings]))
    );
    assert_eq!(
        features(serde_json::json!({"editor": {"features": []}})),
        Ok(Some(vec![]))
    );
    for (what, config) in [
        ("another key", serde_json::json!({"limit": 1})),
        ("editor not an object", serde_json::json!({"editor": []})),
        (
            "another editor key",
            serde_json::json!({"editor": {"a": 1}}),
        ),
        (
            "features not a list",
            serde_json::json!({"editor": {"features": "code"}}),
        ),
        (
            "unknown feature",
            serde_json::json!({"editor": {"features": ["code", "blink"]}}),
        ),
        (
            "not a string",
            serde_json::json!({"editor": {"features": [1]}}),
        ),
        (
            "twice",
            serde_json::json!({"editor": {"features": ["code", "code"]}}),
        ),
    ] {
        let error = features(config).expect_err(what);
        assert!(!error.to_string().contains("blink"), "{what}: {error}");
    }
    let message = features(serde_json::json!({"editor": {"features": ["code", "blink"]}}))
        .unwrap_err()
        .to_string();
    assert!(message.contains("features[1]"), "{message}");
    // The type validates through the same function.
    let config = serde_json::json!({"editor": {"features": ["blink"]}});
    assert!(
        Text.validate_config(
            config.as_object().unwrap(),
            &EffectiveProfiles::unrestricted()
        )
        .is_err()
    );
    let config = serde_json::json!({"editor": {"features": ["code"]}});
    assert!(
        Text.validate_config(
            config.as_object().unwrap(),
            &EffectiveProfiles::unrestricted()
        )
        .is_ok()
    );
    assert!(
        NoSubmission
            .validate_config(
                config.as_object().unwrap(),
                &EffectiveProfiles::unrestricted()
            )
            .is_err()
    );
}
