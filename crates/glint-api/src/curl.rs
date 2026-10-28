//! Parse cURL as data. Never invoke a shell or read referenced files.
use super::model::{BodyFormat, Draft, Pair};
use base64::Engine;
use std::collections::VecDeque;

// cURL options with an argument, including options the workbench does not import.
// Keep their values out of URL detection (a proxy or DoH server can itself be a URL).
fn option_takes_value(flag: &str) -> bool {
    const OPTIONS: &str = "
        -E -K -C -b -c -d -D -F -P -H -h -m -o -x -U -Q -r -e -X -Y -y -t -z -T -u -A -w
        --abstract-unix-socket --alt-svc --aws-sigv4 --cacert --capath --cert --cert-type
        --ciphers --config --connect-timeout --connect-to --continue-at --cookie --cookie-jar
        --create-file-mode --crlfile --curves --data --data-ascii --data-binary --data-raw
        --data-urlencode --delegation --dns-interface --dns-ipv4-addr --dns-ipv6-addr
        --dns-servers --doh-url --dump-header --ech --egd-file --engine --etag-compare
        --etag-save --expect100-timeout --form --form-string --ftp-account
        --ftp-alternative-to-user --ftp-method --ftp-port --ftp-ssl-ccc-mode
        --happy-eyeballs-timeout-ms --haproxy-clientip --header --help --hostpubmd5
        --hostpubsha256 --hsts --interface --ip-tos --ipfs-gateway --json --keepalive-cnt
        --keepalive-time --key --key-type --krb --libcurl --limit-rate --local-port
        --login-options --mail-auth --mail-from --mail-rcpt --max-filesize --max-redirs
        --max-time --netrc-file --noproxy --oauth2-bearer --output --output-dir
        --parallel-max --pass --pinnedpubkey --preproxy --proto --proto-default --proto-redir
        --proxy --proxy-cacert --proxy-capath --proxy-cert --proxy-cert-type --proxy-ciphers
        --proxy-crlfile --proxy-header --proxy-key --proxy-key-type --proxy-pass
        --proxy-pinnedpubkey --proxy-service-name --proxy-tls13-ciphers --proxy-tlsauthtype
        --proxy-tlspassword --proxy-tlsuser --proxy-user --proxy1.0 --pubkey --quote
        --random-file --range --rate --referer --request --request-target --resolve --retry
        --retry-delay --retry-max-time --sasl-authzid --service-name --socks4 --socks4a
        --socks5 --socks5-gssapi-service --socks5-hostname --speed-limit --speed-time
        --ssl-sessions --stderr --telnet-option --tftp-blksize --time-cond --tls-max
        --tls13-ciphers --tlsauthtype --tlspassword --tlsuser --trace --trace-ascii
        --trace-config --unix-socket --upload-file --upload-flags --url --url-query
        --user --user-agent --variable --vlan-priority --write-out
    ";
    OPTIONS
        .split_ascii_whitespace()
        .any(|option| option == flag)
}

fn is_request_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://") || value.starts_with("{{")
}

/// Filter previously cached exports as well as newly generated response commands.
pub fn filter_response_resolves(command: &str) -> String {
    if !command.contains("--resolve") {
        return command.into();
    }
    let Ok(normalized) = normalize(command) else {
        return command.into();
    };
    let Some(tokens) = shlex::split(&normalized) else {
        return command.into();
    };
    let Ok(draft) = parse(command) else {
        return command.into();
    };
    let Ok(url) = reqwest::Url::parse(&draft.url) else {
        return command.into();
    };
    let Some(host) = url.host_str() else {
        return command.into();
    };
    let prefix = format!("{}:{}:", host, url.port_or_known_default().unwrap_or(80));
    let mut parts = vec![];
    let mut removed = false;
    let mut tokens = tokens.into_iter();
    while let Some(token) = tokens.next() {
        if option_takes_value(&token) {
            let Some(value) = tokens.next() else {
                return command.into();
            };
            if token == "--resolve" && !value.starts_with(&prefix) {
                removed = true;
                continue;
            }
            let Ok(value) = quote(&value) else {
                return command.into();
            };
            parts.push(format!("{token} {value}"));
        } else {
            let Ok(token) = quote(&token) else {
                return command.into();
            };
            parts.push(token);
        }
    }
    if removed {
        parts.join(" \\\n  ")
    } else {
        command.into()
    }
}

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
    let mut args: VecDeque<_> = args.into_iter().skip(1).collect();
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
    while let Some(arg) = args.pop_front() {
        if !end && arg == "--" {
            end = true;
            continue;
        }
        if end || !arg.starts_with('-') {
            if url.replace(arg).is_some() {
                return Err("一次只能导入一个请求 URL，不能包含其他命令".into());
            }
            continue;
        }
        let (flag, attached) = if arg.starts_with("--") {
            arg.split_once('=')
                .map_or((arg.as_str(), None), |(a, b)| (a, Some(b.to_owned())))
        } else if arg.len() > 2 && arg.as_bytes()[1].is_ascii() {
            let flag = &arg[..2];
            if option_takes_value(flag) {
                (flag, Some(arg[2..].to_owned()))
            } else {
                // Expand grouped switches without changing an attached option value.
                let mut switches = Vec::new();
                for (offset, ch) in arg[2..].char_indices() {
                    let next = format!("-{ch}");
                    if option_takes_value(&next) {
                        switches.push(format!("-{}", &arg[2 + offset..]));
                        break;
                    }
                    switches.push(next);
                }
                for switch in switches.into_iter().rev() {
                    args.push_front(switch);
                }
                (flag, None)
            }
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
        );
        let supported_switch = matches!(
            flag,
            "-G" | "--get"
                | "-I"
                | "--head"
                | "-k"
                | "--insecure"
                | "--compressed"
                | "-s"
                | "--silent"
                | "-S"
                | "--show-error"
                | "-v"
                | "--verbose"
                | "-i"
                | "--include"
                | "--globoff"
                | "-g"
        );
        if !needs_value && !supported_switch {
            if attached.is_none()
                && args.front().is_some_and(|value| {
                    // Unknown future options may also have a separate argument.
                    // Preserve a bare request URL when the option's arity is unknown.
                    let looks_like_value = !value.starts_with('-')
                        && !is_request_url(value)
                        && !matches!(value.as_str(), ";" | "|" | "||" | "&" | "&&");
                    (option_takes_value(flag) || looks_like_value)
                        && (!value.starts_with('-') || value.parse::<f64>().is_ok())
                })
            {
                args.pop_front();
            }
            continue;
        }
        let value = if needs_value {
            match attached {
                Some(value) => value,
                None => args.pop_front().ok_or_else(|| format!("{flag} 缺少参数"))?,
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
            _ => unreachable!("unsupported options are filtered above"),
        }
    }
    let mut url = url.ok_or("命令中缺少请求 URL")?;
    if !is_request_url(&url) {
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

    #[test]
    fn response_export_filters_cached_mappings_without_changing_request_data() {
        let command = "curl --globoff --compressed --url 'https://api.test/path?q=1' --request 'POST' --resolve 'other.test:443:192.0.2.1' --resolve 'api.test:80:192.0.2.2' --resolve 'api.test:443:192.0.2.3' --header 'X-Example: --resolve other.test:443:192.0.2.1' --data-raw '{\"value\":\"--resolve other.test\"}'";
        let filtered = filter_response_resolves(command);
        assert_eq!(parse(&filtered).unwrap(), parse(command).unwrap());
        let tokens = shlex::split(&filtered).unwrap();
        let mappings: Vec<_> = tokens
            .windows(2)
            .filter(|pair| pair[0] == "--resolve")
            .map(|pair| pair[1].as_str())
            .collect();
        assert_eq!(mappings, ["api.test:443:192.0.2.3"]);
        assert_eq!(filter_response_resolves(&filtered), filtered);
        assert_eq!(
            filter_response_resolves("curl https://api.test"),
            "curl https://api.test"
        );
    }

    #[test]
    fn response_export_normalizes_lf_and_crlf_continuations_without_empty_arguments() {
        let body = "first\\\nsecond";
        let parts = [
            "curl --globoff --compressed".to_owned(),
            "--url 'https://api.test/path'".into(),
            "--request 'POST'".into(),
            "--noproxy ''".into(),
            "--resolve 'other.test:443:192.0.2.1'".into(),
            "--resolve 'api.test:80:192.0.2.2'".into(),
            "--resolve 'api.test:443:192.0.2.3'".into(),
            format!("--data-raw {}", quote(body).unwrap()),
        ];
        for newline in ["\n", "\r\n"] {
            let command = parts.join(&format!(" \\{newline}  "));
            let filtered = filter_response_resolves(&command);
            assert_eq!(parse(&filtered).unwrap(), parse(&command).unwrap());
            let tokens = shlex::split(&normalize(&filtered).unwrap()).unwrap();
            assert_eq!(tokens.iter().filter(|token| token.is_empty()).count(), 1);
            assert!(tokens.windows(2).any(|pair| pair == ["--noproxy", ""]));
            assert!(tokens.windows(2).any(|pair| pair == ["--data-raw", body]));
            assert_eq!(
                tokens
                    .iter()
                    .filter(|token| token.as_str() == "--resolve")
                    .count(),
                1
            );
            assert!(!filtered.contains("other.test"));
            assert!(!filtered.contains("api.test:80:"));
            assert_eq!(filter_response_resolves(&filtered), filtered);
        }
    }

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
    fn filters_unsupported_options_without_changing_request_fields() {
        let source = "curl https://example.test/api -X POST -H 'X-Test: keep' -H 'Authorization: Bearer token' --json '{\"text\":\"你好\"}'";
        let expected = parse(source).unwrap();
        for options in [
            "--proxy http://127.0.0.1:7890",
            "--proxy=http://127.0.0.1:7890",
            "-x http://127.0.0.1:7890",
            "-xhttp://127.0.0.1:7890",
            "--proxy-user 'user:pass' --noproxy '*'",
            "--doh-url https://dns.example.test/query",
            "--location --http2 --ipv4 --no-progress-meter",
            "-sSL --retry 3 --max-time 10 --max-redirs -1",
            "-o result.json --output-dir 'some folder' -w '%{http_code}'",
            "--cacert cert.pem --cert client.pem --key key.pem",
            "--config @config --upload-file @body --form file=@/secret",
            "--future-option=value --future-option 'separate value'",
        ] {
            let suffix = parse(&format!("{source} {options}")).unwrap();
            let prefix = parse(&format!("curl {options} {}", &source[5..])).unwrap();
            assert_eq!(suffix, expected, "{options}");
            assert_eq!(prefix, expected, "{options}");
        }
    }
    #[test]
    fn ignored_switches_preserve_url_and_supported_grouped_options() {
        let expected = parse("curl https://example.test -k -X POST -d 'a=1'").unwrap();
        assert_eq!(
            parse("curl -sSLkXPOST https://example.test -d 'a=1'").unwrap(),
            expected
        );
        assert_eq!(
            parse("curl --location --proxy --header 'X-Test: keep' https://example.test").unwrap(),
            parse("curl --header 'X-Test: keep' https://example.test").unwrap()
        );
        for source in [
            "curl --location https://example.test",
            "curl --http2 https://example.test --proxy",
            "curl --unknown --url https://example.test",
            "curl --unknown https://example.test",
            "curl --unknown=value -- https://example.test",
            "curl -中文 https://example.test",
        ] {
            assert_eq!(
                parse(source).unwrap().url,
                "https://example.test",
                "{source}"
            );
        }
        assert!(parse("curl --proxy http://127.0.0.1:7890").is_err());
        assert!(parse("curl --location https://a.test https://b.test").is_err());
        assert!(parse("curl https://a.test -- --proxy ignored").is_err());
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
    fn malformed_commands_fail_without_execution_or_file_reads() {
        for source in [
            "",
            "echo hi",
            "curl",
            "curl 'unclosed",
            "curl https://a.test https://b.test",
            "curl https://a.test -X",
            "curl https://a.test --data @/secret",
            "curl https://a.test --data-urlencode name@/secret",
            "curl https://a.test -b /secret",
            "curl https://a.test -X GET -d value",
            "curl file:///secret",
            "curl https://a.test ; touch /file",
        ] {
            assert!(parse(source).is_err(), "unexpected success for {source}");
        }
    }
}
