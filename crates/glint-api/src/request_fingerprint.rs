//! Stable fingerprints of saved, unresolved request configuration.
//!
//! Keep serde names and existing defaults stable. New optional fields belong in
//! Draft::default(); their default values then disappear from the canonical form.
//! Retired fields must deserialize to their inactive/default state. Never hash a
//! cached serialized record: old snapshots must use the same normalization rules.
use crate::model::{BodyFormat, Draft};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Fingerprint {
    pub hash: String,
    canonical: Value,
}

impl Fingerprint {
    pub fn new(draft: &Draft) -> Self {
        let mut draft = draft.clone();
        for rows in [
            &mut draft.params,
            &mut draft.headers,
            &mut draft.cookies,
            &mut draft.form,
            &mut draft.extractors,
        ] {
            rows.retain(|row| row.enabled && !row.key.trim().is_empty());
        }
        draft.plugins.retain(|action| action.enabled);
        draft.triggers.retain(|trigger| trigger.enabled);
        if draft.event_decoder.as_ref().is_some_and(|a| !a.enabled) {
            draft.event_decoder = None;
        }
        if !draft.auth || draft.oauth.is_some() {
            draft.auth = false;
            draft.token.clear();
        }
        match draft.body_format {
            BodyFormat::Raw => draft.form.clear(),
            BodyFormat::UrlEncoded => draft.body.clear(),
        }
        let mut value = serde_json::to_value(&draft).expect("request serializes");
        // Defaults inside repeated and optional settings also need to be
        // omitted, so adding an unset row/action setting is backward compatible.
        let pair_defaults = serde_json::to_value(crate::model::Pair::default()).unwrap();
        let action_defaults = serde_json::to_value(crate::model::BeforeAction::default()).unwrap();
        for field in [
            "params",
            "headers",
            "cookies",
            "form",
            "extractors",
            "plugins",
        ] {
            let baseline = if field == "plugins" {
                &action_defaults
            } else {
                &pair_defaults
            };
            for item in value[field].as_array_mut().unwrap() {
                *item =
                    sparse(item.take(), Some(baseline)).unwrap_or_else(|| serde_json::json!({}));
            }
        }
        if draft.event_decoder.is_some() {
            value["event_decoder"] = sparse(value["event_decoder"].take(), Some(&action_defaults))
                .unwrap_or_else(|| serde_json::json!({}));
        }
        let mut defaults = serde_json::to_value(Draft::default()).unwrap();
        // Optional nested settings need their own baseline when enabled. In
        // particular false overrides of true defaults must remain significant.
        if draft.oauth.is_some() {
            let mut oauth = serde_json::to_value(crate::oauth::Config::default()).unwrap();
            oauth.as_object_mut().unwrap().remove("id");
            defaults["oauth"] = oauth;
        }
        let canonical = sparse(value, Some(&defaults)).unwrap_or_else(|| serde_json::json!({}));
        let bytes = serde_json::to_vec(&canonical).unwrap();
        let hash = format!("{:x}", Sha256::digest(bytes))[..16].to_owned();
        Self { hash, canonical }
    }
}

// Sort every object, including plugin configuration; retain array order because
// repeated parameters and action execution order can change request behavior.
fn sparse(value: Value, default: Option<&Value>) -> Option<Value> {
    if default == Some(&value) {
        return None;
    }
    match value {
        Value::Object(object) => {
            let mut fields = object.into_iter().collect::<Vec<_>>();
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            let object = fields
                .into_iter()
                .filter_map(|(key, value)| {
                    let baseline = default.and_then(|d| d.get(&key));
                    sparse(value, baseline).map(|value| (key, value))
                })
                .collect::<serde_json::Map<_, _>>();
            Some(Value::Object(object))
        }
        Value::Array(values) => Some(Value::Array(
            values
                .into_iter()
                .map(|value| sparse(value, None).unwrap_or_else(|| serde_json::json!({})))
                .collect(),
        )),
        value => Some(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Pair;

    #[test]
    fn fingerprints_ignore_defaults_disabled_rows_and_inactive_settings() {
        let original = Draft::default();
        let mut draft = original.clone();
        draft.token = "inactive token".into();
        draft.form.push(Pair::new("unused", "form", ""));
        let mut disabled = Pair::new("unused", "value", "");
        disabled.enabled = false;
        draft.params.push(disabled);
        draft.headers.push(Pair::new("", "", ""));
        let disabled_action = crate::model::BeforeAction {
            environment_bindings: Default::default(),
            enabled: false,
            action_id: "inactive".into(),
            ..Default::default()
        };
        draft.plugins.push(disabled_action.clone());
        draft.event_decoder = Some(disabled_action);
        assert_eq!(Fingerprint::new(&original), Fingerprint::new(&draft));
        assert_eq!(Fingerprint::new(&original).hash, "44136fa355b3678a");
        draft.skip_tls_verification = true;
        assert_ne!(Fingerprint::new(&original), Fingerprint::new(&draft));
        draft = original.clone();
        draft.websocket_follow_latest = false;
        assert_ne!(Fingerprint::new(&original), Fingerprint::new(&draft));
    }

    #[test]
    fn canonical_fields_are_sorted_and_new_defaults_do_not_change_old_hashes() {
        let old = serde_json::json!({"url":"{{baseurl}}/v1", "method":"POST"});
        let mut extended = old.clone();
        extended["future_feature"] = serde_json::json!({"enabled":false,"options":[]});
        let defaults = serde_json::json!({"future_feature":{"enabled":false,"options":[]}});
        assert_eq!(sparse(old, None), sparse(extended, Some(&defaults)));
        let left: Value = serde_json::from_str(r#"{"z":{"b":2,"a":1},"a":true}"#).unwrap();
        let right: Value = serde_json::from_str(r#"{"a":true,"z":{"a":1,"b":2}}"#).unwrap();
        assert_eq!(
            serde_json::to_string(&sparse(left, None)).unwrap(),
            serde_json::to_string(&sparse(right, None)).unwrap()
        );
    }

    #[test]
    fn active_configuration_and_parameter_order_remain_significant() {
        let draft = Draft {
            params: vec![Pair::new("a", "1", ""), Pair::new("a", "2", "")],
            ..Draft::default()
        };
        let fingerprint = Fingerprint::new(&draft);
        let mut changed = draft.clone();
        changed.params.reverse();
        assert_ne!(fingerprint, Fingerprint::new(&changed));
        for field in ["url", "method", "body"] {
            let mut json = serde_json::to_value(&draft).unwrap();
            json[field] = Value::String("changed".into());
            let changed: Draft = serde_json::from_value(json).unwrap();
            assert_ne!(fingerprint, Fingerprint::new(&changed));
        }
        let mut changed = draft;
        changed.oauth = Some(crate::oauth::Config::default());
        let before = Fingerprint::new(&changed);
        changed.oauth.as_mut().unwrap().pkce = false;
        assert_ne!(before, Fingerprint::new(&changed));
    }

    #[test]
    fn legacy_records_fill_optional_defaults_and_keep_the_same_fingerprint() {
        let legacy = serde_json::json!({
            "method":"POST", "url":"{{baseurl}}/v1", "body":"",
            "params":[{"key":"q","value":"{{query}}"}],
            "headers":[], "cookies":[], "token":"", "auth":false
        });
        let draft: Draft = serde_json::from_value(legacy).unwrap();
        let fingerprint = Fingerprint::new(&draft);
        assert_eq!(
            fingerprint.canonical,
            serde_json::json!({
                "method":"POST", "url":"{{baseurl}}/v1",
                "params":[{"key":"q","value":"{{query}}"}]
            })
        );
        let roundtrip: Draft =
            serde_json::from_value(serde_json::to_value(draft).unwrap()).unwrap();
        assert_eq!(fingerprint, Fingerprint::new(&roundtrip));
        let mut collision = Fingerprint::new(&Draft::default());
        collision.hash = fingerprint.hash.clone();
        assert_ne!(
            fingerprint, collision,
            "a short hash collision must not match a different configuration"
        );
    }

    #[test]
    fn all_active_sections_change_the_fingerprint() {
        let draft = Draft::default();
        let fingerprint = Fingerprint::new(&draft);
        for (field, value) in [
            ("auth", serde_json::json!(true)),
            ("body_format", serde_json::json!("url_encoded")),
            ("skip_tls_verification", serde_json::json!(true)),
            ("websocket_raw", serde_json::json!(true)),
            ("websocket_follow_latest", serde_json::json!(false)),
            ("params", serde_json::json!([Pair::new("q", "1", "")])),
            ("headers", serde_json::json!([Pair::new("x-test", "1", "")])),
            (
                "cookies",
                serde_json::json!([Pair::new("session", "1", "")]),
            ),
            (
                "extractors",
                serde_json::json!([Pair::new("token", "$.token", "")]),
            ),
            (
                "plugins",
                serde_json::json!([crate::model::BeforeAction {
                    environment_bindings: Default::default(),
                    action_id: "run".into(),
                    ..Default::default()
                }]),
            ),
            (
                "event_decoder",
                serde_json::json!(crate::model::BeforeAction {
                    environment_bindings: Default::default(),
                    action_id: "decode".into(),
                    ..Default::default()
                }),
            ),
        ] {
            let mut value_draft = serde_json::to_value(&draft).unwrap();
            value_draft[field] = value;
            let changed: Draft = serde_json::from_value(value_draft).unwrap();
            assert_ne!(fingerprint, Fingerprint::new(&changed), "{field}");
        }
        let mut bearer = Draft {
            auth: true,
            token: "a".into(),
            ..Draft::default()
        };
        let before = Fingerprint::new(&bearer);
        bearer.token = "b".into();
        assert_ne!(before, Fingerprint::new(&bearer));
        let mut form = Draft {
            body_format: BodyFormat::UrlEncoded,
            form: vec![Pair::new("a", "1", "")],
            ..Draft::default()
        };
        let before = Fingerprint::new(&form);
        form.body = "inactive raw body".into();
        assert_eq!(before, Fingerprint::new(&form));
        form.form[0].value = "2".into();
        assert_ne!(before, Fingerprint::new(&form));
    }
}
