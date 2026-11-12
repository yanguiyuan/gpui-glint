use std::sync::Mutex;

static ENVIRONMENT: Mutex<Option<glint_vault::Environment>> = Mutex::new(None);

pub fn bind(environment: glint_vault::Environment) {
    *ENVIRONMENT.lock().unwrap() = Some(environment);
}

pub fn with_variables(mut apply: impl FnMut(&str, &str)) {
    let environment = ENVIRONMENT.lock().unwrap().clone();
    if let Some(environment) = environment {
        for (name, value) in environment.snapshot().iter() {
            apply(name, value);
        }
    }
}

pub fn list_entries(arguments: &serde_json::Value) -> Result<String, String> {
    let environment = ENVIRONMENT.lock().unwrap().clone();
    list_from(environment.as_ref(), arguments)
}

fn list_from(
    environment: Option<&glint_vault::Environment>,
    arguments: &serde_json::Value,
) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        query: Option<String>,
        offset: Option<usize>,
        limit: Option<usize>,
    }
    let args: Args = serde_json::from_value(arguments.clone()).map_err(|_| "密码箱查询参数无效")?;
    let limit = args.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err("limit 必须在 1 到 100 之间".into());
    }
    let environment = environment.ok_or("密码箱不可用")?;
    let Some(entries) = environment.entries() else {
        return Ok(
            serde_json::json!({"status":"locked","message":"请先在应用中解锁密码箱"}).to_string(),
        );
    };
    Ok(page(
        entries,
        &args.query.unwrap_or_default(),
        args.offset.unwrap_or(0),
        limit,
    )
    .to_string())
}

fn page(
    entries: Vec<glint_vault::EntrySummary>,
    query: &str,
    offset: usize,
    limit: usize,
) -> serde_json::Value {
    let query = query.trim().to_lowercase();
    let entries: Vec<_> = entries
        .into_iter()
        .filter(|entry| {
            [
                &entry.id,
                &entry.title,
                &entry.username,
                &entry.website,
                &entry.env_identifier,
                entry.env_name.as_deref().unwrap_or(""),
            ]
            .iter()
            .any(|value| value.to_lowercase().contains(&query))
        })
        .collect();
    let total = entries.len();
    let offset = offset.min(total);
    let end = offset.saturating_add(limit).min(total);
    serde_json::json!({"status":"unlocked","entries":&entries[offset..end],"total":total,"next_offset":(end < total).then_some(end)})
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn search_and_pagination_include_entries_without_environment_names() {
        let entries: Vec<_> = (0..3)
            .map(|index| glint_vault::EntrySummary {
                id: format!("id-{index}"),
                title: format!("Work {index}"),
                username: "alice@example.com".into(),
                website: "https://example.com".into(),
                env_identifier: if index == 0 {
                    "MAIL".into()
                } else {
                    String::new()
                },
                env_name: (index == 0).then(|| "GLINT_MAIL_PWD".into()),
                updated_at: "1".into(),
            })
            .collect();
        let first = page(entries.clone(), " WORK ", 0, 2);
        assert_eq!(first["total"], 3);
        assert_eq!(first["entries"].as_array().unwrap().len(), 2);
        assert_eq!(first["next_offset"], 2);
        let last = page(entries.clone(), "work", 2, 2);
        assert_eq!(last["entries"][0]["id"], "id-2");
        assert!(last["next_offset"].is_null());
        for query in ["glint_mail_pwd", "id-0", "MAIL"] {
            assert_eq!(page(entries.clone(), query, 0, 50)["total"], 1);
        }
        assert_eq!(page(entries.clone(), "ALICE", 0, 50)["total"], 3);
        assert_eq!(page(entries.clone(), "missing", 0, 50)["total"], 0);
        assert!(
            page(entries, "", usize::MAX, 50)["entries"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn unavailable_locked_and_invalid_queries_are_explicit() {
        assert!(list_from(None, &json!({})).is_err());
        let environment = glint_vault::Environment::default();
        let result: serde_json::Value =
            serde_json::from_str(&list_from(Some(&environment), &json!({})).unwrap()).unwrap();
        assert_eq!(result["status"], "locked");
        for args in [
            json!({"limit":0}),
            json!({"limit":101}),
            json!({"offset":-1}),
            json!({"include_secrets":true}),
        ] {
            assert!(list_from(Some(&environment), &args).is_err());
        }
    }
}
