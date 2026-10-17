//! Parse cURL as data. Never invoke a shell or read referenced files.
use super::model::{BodyFormat, Draft, Pair};
use base64::Engine;

pub fn quote(value: &str) -> Result<String, String> {
    if value.contains('\0') {
        return Err("cURL 命令不能包含 NUL 字符".into());
    }
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}
fn header(draft: &mut Draft, value: &str) -> Result<(), String> {
    let (name, value, empty_literal) =
        if let Some(name) = value.strip_suffix(';').filter(|_| !value.contains(':')) {
            (name, "", true)
        } else {
            let (name, value) = value.split_once(':').ok_or("请求头应为 名称: 值 格式")?;
            (name, value.trim_start(), false)
        };
    if name.trim().is_empty() || value.contains(['\r', '\n']) {
        return Err("请求头格式无效".into());
    }
    reqwest::header::HeaderName::from_bytes(name.trim().as_bytes())
        .map_err(|_| "请求头名称无效")?;
    if value.is_empty() && !empty_literal {
        if name.trim().eq_ignore_ascii_case("User-Agent") {
            return Ok(());
        }
        return Err("暂不支持移除默认请求头；如需发送空值，请使用 -H '名称;'".into());
    }
    draft.headers.push(Pair::new(name.trim(), value, ""));
    Ok(())
}
fn default_header(draft: &mut Draft, name: &str, value: &str) {
    if !draft
        .headers
        .iter()
        .any(|p| p.key.eq_ignore_ascii_case(name))
    {
        draft.headers.push(Pair::new(name, value, ""));
    }
}

// Remove shell line continuations before tokenization, while preserving quoted body bytes.
fn normalize(source: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut chars = source.chars().peekable();
    let mut single = false;
    let mut double = false;
    while let Some(ch) = chars.next() {
        if ch == '$' && !single && !double && chars.peek() == Some(&'\'') {
            return Err("暂不支持 $'…' 引号，请改用单引号或双引号".into());
        }
        if ch == '\\' && !single {
            if chars.peek() == Some(&'\n') {
                chars.next();
                continue;
            }
            if chars.peek() == Some(&'\r') {
                let mut look = chars.clone();
                look.next();
                if look.peek() == Some(&'\n') {
                    chars.next();
                    chars.next();
                    continue;
                }
            }
            out.push(ch);
            if let Some(next) = chars.next() {
                out.push(next);
            }
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
        }
        if ch == '"' && !single {
            double = !double;
        }
        out.push(ch);
    }
    Ok(out)
}

pub fn parse(source: &str) -> Result<Draft, String> {
    if source.len() > 2 * 1024 * 1024 {
        return Err("命令超过 2 MiB，请缩小请求内容".into());
    }
    if source.contains('\0') {
        return Err("命令含有 NUL 字符".into());
    }
    let normalized = normalize(source)?;
    let args = shlex::split(&normalized).ok_or("引号或反斜杠未闭合，请粘贴完整的 cURL 命令")?;
    if !matches!(args.first().map(String::as_str), Some("curl" | "curl.exe")) {
        return Err("请粘贴以 curl 开头的命令".into());
    }
    let mut draft = Draft::default();
    let mut url = None;
    let mut method = None;
    let mut parts = Vec::new();
    let mut form = Vec::new();
    let mut only_encoded = true;
    let mut has_body = false;
    let mut get = false;
    let mut head = false;
    let mut json = false;
    let mut end = false;
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        i += 1;
        if !end && arg == "--" {
            end = true;
            continue;
        }
        if end || !arg.starts_with('-') {
            if url.replace(arg.clone()).is_some() {
                return Err("一次只能导入一个请求 URL，不能包含其他命令".into());
            }
            continue;
        }
        let (flag, attached) = if arg.starts_with("--") {
            arg.split_once('=')
                .map_or((arg.as_str(), None), |(a, b)| (a, Some(b.to_owned())))
        } else if arg.len() > 2
            && matches!(
                arg.get(..2).unwrap_or(""),
                "-X" | "-H" | "-d" | "-b" | "-u" | "-A" | "-e"
            )
        {
            (&arg[..2], Some(arg[2..].to_owned()))
        } else {
            (arg.as_str(), None)
        };
        let needs_value = matches!(
            flag,
            "-X" | "--request"
                | "-H"
                | "--header"
                | "--url"
                | "-d"
                | "--data"
                | "--data-ascii"
                | "--data-raw"
                | "--data-binary"
                | "--data-urlencode"
                | "--json"
                | "-b"
                | "--cookie"
                | "-u"
                | "--user"
                | "--oauth2-bearer"
                | "-A"
                | "--user-agent"
                | "-e"
                | "--referer"
                | "--noproxy"
        );
        let value = if needs_value {
            match attached {
                Some(value) => value,
                None => {
                    let value = args
                        .get(i)
                        .ok_or_else(|| format!("{flag} 缺少参数"))?
                        .clone();
                    i += 1;
                    value
                }
            }
        } else {
            if attached.is_some() {
                return Err(format!("{flag} 不接受参数"));
            }
            String::new()
        };
        match flag {
            "--url" => {
                if url.replace(value).is_some() {
                    return Err("一次只能导入一个 URL".into());
                }
            }
            "-X" | "--request" => method = Some(value.to_ascii_uppercase()),
            "-H" | "--header" => header(&mut draft, &value)?,
            "-A" | "--user-agent" => draft.headers.push(Pair::new("User-Agent", &value, "")),
            "-e" | "--referer" => draft.headers.push(Pair::new("Referer", &value, "")),
            "-b" | "--cookie" => {
                if !value.contains('=') {
                    return Err("Cookie 文件导入暂不支持，请使用 name=value 格式".into());
                }
                draft.headers.push(Pair::new("Cookie", &value, ""));
            }
            "-u" | "--user" => {
                if !value.contains(':') {
                    return Err("Basic 鉴权请提供 user:password，不支持交互输入密码".into());
                }
                let encoded = base64::engine::general_purpose::STANDARD.encode(value.as_bytes());
                draft
                    .headers
                    .push(Pair::new("Authorization", &format!("Basic {encoded}"), ""));
            }
            "--oauth2-bearer" => {
                draft.auth = true;
                draft.token = value;
            }
            "--data-urlencode" => {
                has_body = true;
                let (name, content) = value.split_once('=').unwrap_or(("", &value));
                if !value.contains('=') && value.contains('@') {
                    return Err("不支持从文件读取表单，请直接粘贴字段内容".into());
                }
                let encoded: String = form_urlencoded::byte_serialize(content.as_bytes()).collect();
                parts.push(if name.is_empty() {
                    encoded
                } else {
                    format!("{name}={encoded}")
                });
                form.push(Pair::new(name, content, ""));
            }
            "-d" | "--data" | "--data-ascii" | "--data-raw" | "--data-binary" | "--json" => {
                if flag != "--data-raw" && value.starts_with('@') {
                    return Err("不支持从文件或 stdin 读取正文，请直接粘贴正文内容".into());
                }
                if flag == "--json" {
                    json = true;
                }
                has_body = true;
                only_encoded = false;
                parts.push(value);
            }
            "-G" | "--get" => get = true,
            "-I" | "--head" => head = true,
            "-k" | "--insecure" => draft.skip_tls_verification = true,
            "--compressed" | "-s" | "--silent" | "-S" | "--show-error" | "-v" | "--verbose"
            | "-i" | "--include" | "--globoff" | "-g" => {}
            "--noproxy" if value == "*" => {}
            "-F" | "--form" | "--form-string" => {
                return Err("暂不支持 multipart/form-data，请使用表单或原始请求体".into());
            }
            "-L" | "--location" => {
                return Err("当前应用不会自动跟随重定向，请移除 --location / -L 后导入".into());
            }
            _ => return Err(format!("暂不支持 cURL 参数 {flag}，请移除后重试")),
        }
    }
    let mut url = url.ok_or("命令中缺少请求 URL")?;
    if !url.starts_with("http://") && !url.starts_with("https://") && !url.starts_with("{{") {
        return Err("URL 需使用 http://、https:// 或 {{环境变量}}".into());
    }
    draft.method = method.unwrap_or_else(|| {
        if head {
            "HEAD"
        } else if get || !has_body {
            "GET"
        } else {
            "POST"
        }
        .into()
    });
    if !matches!(
        draft.method.as_str(),
        "GET" | "HEAD" | "OPTIONS" | "POST" | "PUT" | "PATCH" | "DELETE"
    ) {
        return Err("暂不支持此 HTTP 方法".into());
    }
    if get && has_body {
        let (base, fragment) = url
            .split_once('#')
            .map_or((url.as_str(), String::new()), |(base, fragment)| {
                (base, format!("#{fragment}"))
            });
        url = format!(
            "{base}{}{}{fragment}",
            if base.contains('?') {
                if base.ends_with(['?', '&']) { "" } else { "&" }
            } else {
                "?"
            },
            parts.join("&")
        );
    } else if has_body {
        if !matches!(draft.method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE") {
            return Err("当前应用不支持此方法携带正文，请检查 -X / --request 参数".into());
        }
        if only_encoded && form.iter().all(|p| !p.key.is_empty()) {
            draft.body_format = BodyFormat::UrlEncoded;
            draft.form = form;
        } else {
            draft.body = parts.join(if json { "" } else { "&" });
        }
        default_header(
            &mut draft,
            "Content-Type",
            if json {
                "application/json"
            } else {
                "application/x-www-form-urlencoded"
            },
        );
    }
    if json {
        default_header(&mut draft, "Accept", "application/json");
    }
    draft.url = url;
    let auth: Vec<_> = draft
        .headers
        .iter()
        .enumerate()
        .filter(|(_, h)| h.key.eq_ignore_ascii_case("Authorization"))
        .collect();
    if !draft.auth && auth.len() == 1 {
        let (index, header) = auth[0];
        if let Some((scheme, token)) = header
            .value
            .split_once(' ')
            .filter(|(s, _)| s.eq_ignore_ascii_case("Bearer"))
        {
            let _ = scheme;
            draft.auth = true;
            draft.token = token.into();
            draft.headers.remove(index);
        }
    }
    Ok(draft)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{environment::EnvironmentVariables, http::PreparedRequest};
    #[test]
    fn imports_headers_bearer_cookie_json_and_continuations() {
        let draft=parse("curl 'https://example.test/a?q=a%20b' \\\n -XPOST -H 'Authorization: Bearer fake-token' --header='X-Test: one:two' \\\n -b 'a=1; b=two' --json '{\"text\":\"你好\"}' -k --compressed").unwrap();
        assert_eq!(draft.method, "POST");
        assert_eq!(draft.url, "https://example.test/a?q=a%20b");
        assert!(draft.auth && draft.skip_tls_verification);
        assert_eq!(draft.token, "fake-token");
        assert_eq!(draft.body, "{\"text\":\"你好\"}");
        assert!(
            draft
                .headers
                .iter()
                .any(|p| p.key == "X-Test" && p.value == "one:two")
        );
        assert!(
            draft
                .headers
                .iter()
                .any(|p| p.key == "Cookie" && p.value == "a=1; b=two")
        );
        assert!(
            draft
                .headers
                .iter()
                .any(|p| p.key == "Accept" && p.value == "application/json")
        );
    }
    #[test]
    fn imports_form_query_and_basic_auth_without_double_encoding() {
        let draft = parse("curl https://example.test/token --data-urlencode 'name=张 三' --data-urlencode 'name=a+b&=' --user 'u:p:q'").unwrap();
        assert_eq!(draft.body_format, BodyFormat::UrlEncoded);
        assert_eq!(draft.form.len(), 2);
        let request =
            PreparedRequest::new(&draft, &EnvironmentVariables::default().values()).unwrap();
        let payload = request.plugin_payload().unwrap();
        assert_eq!(payload["body"], "name=%E5%BC%A0+%E4%B8%89&name=a%2Bb%26%3D");
        assert_eq!(payload["headers"]["authorization"], "Basic dTpwOnE=");
        let query=parse("curl --get --url 'https://example.test/path?x=1#fragment' --data-urlencode 'q=你好 世界' --data 'raw=a%2Bb'").unwrap();
        assert_eq!(query.method, "GET");
        assert!(query.body.is_empty());
        assert_eq!(
            query.url,
            "https://example.test/path?x=1&q=%E4%BD%A0%E5%A5%BD+%E4%B8%96%E7%95%8C&raw=a%2Bb#fragment"
        );
        assert_eq!(
            parse("curl https://example.test -I").unwrap().method,
            "HEAD"
        );
        assert_eq!(
            parse("curl https://example.test -d a=1 -d b=2")
                .unwrap()
                .body,
            "a=1&b=2"
        );
        assert_eq!(
            parse("curl https://example.test --data-raw '@literal'")
                .unwrap()
                .body,
            "@literal"
        );
    }
    #[test]
    fn export_round_trip_preserves_resolved_signed_request_and_shell_literals() {
        let mut environment = EnvironmentVariables::default();
        environment
            .save(None, "baseurl", "https://example.test", false)
            .unwrap();
        environment
            .save(None, "token", "synthetic-token", true)
            .unwrap();
        let draft = Draft {
            method: "POST".into(),
            url: "{{baseurl}}/path?q=a%2Bb".into(),
            auth: true,
            token: "{{token}}".into(),
            headers: vec![Pair::new("X-Test", "quote's $HOME `id` $(id)", "")],
            body: "{\"text\":\"你好\\nquote's $HOME $(id)\"}".into(),
            skip_tls_verification: true,
            ..Draft::default()
        };
        let mut request = PreparedRequest::new(&draft, &environment.values()).unwrap();
        request
            .apply_plugin_headers(
                &serde_json::json!({"setHeaders":{"X-Signature":"synthetic-signature"}}),
                &std::collections::BTreeSet::from(["request.header.x-signature".into()]),
            )
            .unwrap();
        let command = request.to_curl().unwrap();
        assert!(command.contains(" \\\n"));
        let imported = parse(&command).unwrap();
        assert!(imported.skip_tls_verification);
        let roundtrip =
            PreparedRequest::new(&imported, &EnvironmentVariables::default().values()).unwrap();
        assert_eq!(
            request.plugin_payload().unwrap(),
            roundtrip.plugin_payload().unwrap()
        );
        assert!(!command.contains("{{"));
        let literal = "quotes '\" $HOME `id` $(id) \\ tab\t newline\n中文";
        assert_eq!(
            shlex::split(&quote(literal).unwrap()).unwrap(),
            vec![literal]
        );
        assert!(quote("nul\0").is_err());
    }
    #[test]
    fn malformed_or_unsupported_commands_fail_without_execution_or_file_reads() {
        for source in [
            "",
            "echo hi",
            "curl",
            "curl 'unclosed",
            "curl https://a.test https://b.test",
            "curl https://a.test -X",
            "curl https://a.test -F file=@/secret",
            "curl https://a.test --data @/secret",
            "curl https://a.test --data-urlencode name@/secret",
            "curl https://a.test -b /secret",
            "curl https://a.test -L",
            "curl https://a.test -X GET -d value",
            "curl https://a.test -中文",
            "curl file:///secret",
            "curl https://a.test ; touch /file",
        ] {
            assert!(parse(source).is_err(), "unexpected success for {source}");
        }
    }
}
