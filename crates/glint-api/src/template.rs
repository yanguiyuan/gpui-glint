use super::{environment::EnvironmentValues, model::Draft};
use chrono::{DateTime, Utc};
use rand::Rng;
use std::{cell::RefCell, collections::BTreeMap};

/// One evaluation context per send; repeated expressions share generated values.
pub struct TemplateResolver {
    values: EnvironmentValues,
    cookies: BTreeMap<String, String>,
    cache: RefCell<BTreeMap<String, String>>,
    now: DateTime<Utc>,
}
impl TemplateResolver {
    pub fn new(values: &EnvironmentValues) -> Self {
        Self {
            values: values.clone(),
            cookies: BTreeMap::new(),
            cache: RefCell::default(),
            now: Utc::now(),
        }
    }
    pub fn for_request(values: &EnvironmentValues, draft: &Draft) -> Result<Self, String> {
        let mut resolver = Self::new(values);
        for header in draft
            .headers
            .iter()
            .filter(|p| p.enabled && !p.key.trim().is_empty())
        {
            if resolver
                .resolve(&header.key)?
                .trim()
                .eq_ignore_ascii_case("cookie")
            {
                let value = resolver.resolve(&header.value)?;
                for part in value.split(';') {
                    if let Some((key, value)) = part.split_once('=') {
                        if !key.trim().is_empty() {
                            resolver
                                .cookies
                                .insert(key.trim().into(), value.trim().into());
                        }
                    }
                }
            }
        }
        for cookie in draft
            .cookies
            .iter()
            .filter(|p| p.enabled && !p.key.trim().is_empty())
        {
            let key = resolver.resolve(&cookie.key)?;
            resolver
                .cookies
                .insert(key.trim().into(), cookie.value.trim().into());
        }
        // Cookie-dependent expressions must reflect the final override map.
        resolver
            .cache
            .borrow_mut()
            .retain(|key, _| !key.starts_with("cookie("));
        Ok(resolver)
    }
    pub fn resolve(&self, input: &str) -> Result<String, String> {
        self.resolve_inner(input, &mut Vec::new())
    }
    fn resolve_inner(&self, input: &str, stack: &mut Vec<String>) -> Result<String, String> {
        let mut output = String::new();
        let mut rest = input;
        while let Some(start) = rest.find("{{") {
            output.push_str(&rest[..start]);
            let variable = &rest[start + 2..];
            let end = variable.find("}}").ok_or("模板缺少结束符 }}")?;
            let expression = variable[..end].trim();
            if stack.iter().any(|active| active == expression) {
                return Err(format!("模板存在循环引用：{expression}"));
            }
            if stack.len() >= 32 {
                return Err("模板嵌套超过 32 层".into());
            }
            stack.push(expression.into());
            let value = if expression.contains('(') {
                let cached = self.cache.borrow().get(expression).cloned();
                if let Some(value) = cached {
                    value
                } else {
                    let value = self.call(expression, stack)?;
                    self.cache
                        .borrow_mut()
                        .insert(expression.into(), value.clone());
                    value
                }
            } else {
                let value = self
                    .values
                    .0
                    .get(expression)
                    .ok_or_else(|| format!("未定义环境变量：{expression}"))?;
                if value.is_empty() {
                    return Err(format!("请在环境变量中填写 {expression}"));
                }
                self.resolve_inner(value, stack)?
            };
            stack.pop();
            output.push_str(&value);
            rest = &variable[end + 2..];
        }
        output.push_str(rest);
        Ok(output)
    }
    fn call(&self, expression: &str, stack: &mut Vec<String>) -> Result<String, String> {
        let (name, tail) = expression.split_once('(').ok_or("模板方法格式无效")?;
        let body = tail.strip_suffix(')').ok_or("模板方法缺少结束括号")?;
        let args = arguments(body)?;
        let name = name.trim();
        let invalid = || format!("{name} 的参数无效，请查看模板方法说明");
        let length = |default: usize| -> Result<usize, String> {
            let value = match args.as_slice() {
                [] => default,
                [value] => value.parse().map_err(|_| invalid())?,
                _ => return Err(invalid()),
            };
            if !(1..=4096).contains(&value) {
                return Err("生成长度须为 1–4096".into());
            }
            Ok(value)
        };
        match name {
            "random_int" => {
                let (min, max) = match args.as_slice() {
                    [] => (0_i64, 100_i64),
                    [min, max] => (
                        min.parse::<i64>().map_err(|_| invalid())?,
                        max.parse::<i64>().map_err(|_| invalid())?,
                    ),
                    _ => return Err(invalid()),
                };
                if min > max {
                    return Err("随机数字的最小值不能大于最大值".into());
                }
                Ok(rand::rng().random_range(min..=max).to_string())
            }
            "uuid" | "random_id" if args.is_empty() => Ok(uuid::Uuid::new_v4().to_string()),
            "random_string" => {
                const ALPHABET: &[u8] =
                    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
                let mut rng = rand::rng();
                Ok((0..length(16)?)
                    .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
                    .collect())
            }
            "random_name" if args.is_empty() => {
                const FIRST: &[&str] = &[
                    "Alex", "Sam", "Jordan", "Taylor", "Morgan", "Casey", "Jamie", "Robin",
                ];
                const LAST: &[&str] = &[
                    "Chen", "Lin", "Wang", "Lee", "Smith", "Brown", "Wilson", "Zhang",
                ];
                let mut rng = rand::rng();
                Ok(format!(
                    "{} {}",
                    FIRST[rng.random_range(0..FIRST.len())],
                    LAST[rng.random_range(0..LAST.len())]
                ))
            }
            "now" => match args.as_slice() {
                [] => Ok(self
                    .now
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
                [format] => {
                    use std::fmt::Write;
                    let mut result = String::new();
                    write!(&mut result, "{}", self.now.format(format))
                        .map_err(|_| "时间格式无效".to_string())?;
                    Ok(result)
                }
                _ => Err(invalid()),
            },
            "timestamp" if args.is_empty() => Ok(self.now.timestamp().to_string()),
            "timestamp_ms" if args.is_empty() => Ok(self.now.timestamp_millis().to_string()),
            "xss" => match args.first().map(String::as_str).unwrap_or("script") {
                _ if args.len() > 1 => Err(invalid()),
                "script" => Ok("<script>alert('glint-xss')</script>".into()),
                "img" => Ok("<img src=x onerror=alert('glint-xss')>".into()),
                "svg" => Ok("<svg onload=alert('glint-xss')>".into()),
                _ => Err("XSS 测试类型支持 script、img、svg".into()),
            },
            "cookie" => match args.as_slice() {
                [name] if !name.is_empty() => {
                    let value = self
                        .cookies
                        .get(name)
                        .ok_or_else(|| format!("请求 Cookie 中不存在：{name}"))?;
                    self.resolve_inner(value, stack)
                }
                _ => Err(invalid()),
            },
            _ => Err(format!("未知模板方法或参数无效：{name}")),
        }
    }
}
/// Quoted arguments support commas and escaped quotes; unquoted numeric args are allowed.
fn arguments(input: &str) -> Result<Vec<String>, String> {
    if input.trim().is_empty() {
        return Ok(vec![]);
    }
    let mut args = Vec::new();
    let mut chars = input.chars().peekable();
    loop {
        while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
            chars.next();
        }
        let mut value = String::new();
        if chars.peek().is_some_and(|ch| *ch == '\'' || *ch == '"') {
            let quote = chars.next().unwrap();
            let mut closed = false;
            while let Some(ch) = chars.next() {
                if ch == quote {
                    closed = true;
                    break;
                }
                if ch == '\\' {
                    value.push(chars.next().ok_or("模板参数转义不完整")?);
                } else {
                    value.push(ch);
                }
            }
            if !closed {
                return Err("模板参数引号未闭合".into());
            }
            while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
                chars.next();
            }
        } else {
            while chars.peek().is_some_and(|ch| *ch != ',') {
                value.push(chars.next().unwrap());
            }
            value = value.trim().into();
        }
        if value.is_empty() {
            return Err("模板参数不能为空".into());
        }
        args.push(value);
        match chars.next() {
            None => break,
            Some(',') if chars.peek().is_some() => {}
            _ => return Err("模板参数格式无效".into()),
        }
    }
    Ok(args)
}

pub const METHOD_EXAMPLES: &[(&str, &str)] = &[
    ("{{random_int(1, 100)}}", "随机数字"),
    ("{{uuid()}}", "UUID"),
    ("{{random_string(16)}}", "随机字符串"),
    ("{{random_name()}}", "随机名字"),
    ("{{xss()}}", "XSS · script"),
    ("{{xss('img')}}", "XSS · img"),
    ("{{xss('svg')}}", "XSS · svg"),
    ("{{now()}}", "当前时间"),
    ("{{now('%Y-%m-%d %H:%M:%S')}}", "格式化时间"),
    ("{{timestamp()}}", "时间戳 · 秒"),
    ("{{timestamp_ms()}}", "时间戳 · 毫秒"),
    ("{{cookie('csrftoken')}}", "Cookie · csrftoken"),
];

#[cfg(test)]
mod tests {
    use super::super::model::Pair;
    use super::*;
    fn empty() -> EnvironmentValues {
        EnvironmentValues(BTreeMap::new())
    }
    #[test]
    fn generators_share_values_per_send_and_refresh_between_sends() {
        let env = EnvironmentValues(BTreeMap::from([("id".into(), "{{uuid()}}".into())]));
        let resolver = TemplateResolver::new(&env);
        let id = resolver.resolve("{{id}}").unwrap();
        let uuid = uuid::Uuid::parse_str(&id).unwrap();
        assert_eq!(uuid.get_version_num(), 4);
        assert_eq!(resolver.resolve("{{uuid()}}").unwrap(), id);
        assert_ne!(TemplateResolver::new(&env).resolve("{{id}}").unwrap(), id);
        assert_eq!(resolver.resolve("{{random_int(-8,-8)}}").unwrap(), "-8");
        let number: i64 = resolver
            .resolve("{{random_int(-5,5)}}")
            .unwrap()
            .parse()
            .unwrap();
        assert!((-5..=5).contains(&number));
        let value = resolver.resolve("{{random_string(64)}}").unwrap();
        assert_eq!(value.len(), 64);
        assert!(value.chars().all(|ch| ch.is_ascii_alphanumeric()));
        assert_eq!(
            resolver
                .resolve("{{random_name()}}")
                .unwrap()
                .split_whitespace()
                .count(),
            2
        );
        assert!(
            resolver
                .resolve("{{xss('img')}}")
                .unwrap()
                .contains("onerror")
        );
        assert!(
            resolver
                .resolve("{{xss('svg')}}")
                .unwrap()
                .contains("onload")
        );
        assert!(resolver.resolve("{{xss()}}").unwrap().contains("<script>"));
        for (expression, _) in METHOD_EXAMPLES
            .iter()
            .filter(|(expression, _)| !expression.contains("cookie("))
        {
            resolver.resolve(expression).unwrap();
        }
    }
    #[test]
    fn time_formats_share_one_clock_and_invalid_inputs_return_errors() {
        let resolver = TemplateResolver::new(&empty());
        let time = DateTime::parse_from_rfc3339(&resolver.resolve("{{now()}}").unwrap()).unwrap();
        assert_eq!(
            resolver.resolve("{{timestamp_ms()}}").unwrap(),
            time.timestamp_millis().to_string()
        );
        assert_eq!(
            resolver.resolve("{{timestamp()}}").unwrap(),
            time.timestamp().to_string()
        );
        assert_eq!(
            resolver.resolve("{{now('%Y,%m')}}").unwrap(),
            resolver.now.format("%Y,%m").to_string()
        );
        for expression in [
            "{{random_int(2,1)}}",
            "{{random_int(a,2)}}",
            "{{random_string(0)}}",
            "{{random_string(4097)}}",
            "{{uuid(1)}}",
            "{{xss('bad')}}",
            "{{now('%Q')}}",
            "{{now('oops)}}",
            "{{random_int(1,)}}",
            "{{cookie('missing')}}",
            "{{unknown()}}",
        ] {
            assert!(resolver.resolve(expression).is_err(), "{expression}");
        }
    }
    #[test]
    fn cookie_extraction_merges_headers_overrides_and_detects_cycles() {
        let env = EnvironmentValues(BTreeMap::from([
            ("token".into(), "a=b".into()),
            (
                "cookie_header".into(),
                "csrftoken=old; session={{token}}".into(),
            ),
        ]));
        let mut draft = Draft {
            headers: vec![Pair::new("cookie", "{{cookie_header}}", "")],
            cookies: vec![
                Pair::new("csrftoken", "{{token}}", ""),
                Pair::new("derived", "{{cookie('csrftoken')}}", ""),
            ],
            ..Draft::default()
        };
        let mut disabled = Pair::new("csrftoken", "ignored", "");
        disabled.enabled = false;
        draft.cookies.push(disabled);
        let resolver = TemplateResolver::for_request(&env, &draft).unwrap();
        assert_eq!(resolver.resolve("{{cookie('csrftoken')}}").unwrap(), "a=b");
        assert_eq!(resolver.resolve("{{cookie('session')}}").unwrap(), "a=b");
        assert_eq!(resolver.resolve("{{cookie('derived')}}").unwrap(), "a=b");
        assert!(resolver.resolve("{{cookie('CSRFToken')}}").is_err());
        draft.cookies = vec![
            Pair::new("a", "{{cookie('b')}}", ""),
            Pair::new("b", "{{cookie('a')}}", ""),
        ];
        let resolver = TemplateResolver::for_request(&env, &draft).unwrap();
        assert!(
            resolver
                .resolve("{{cookie('a')}}")
                .unwrap_err()
                .contains("循环")
        );
    }
}
