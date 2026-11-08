//! Source highlighting shared by file previews, fenced blocks and diffs.
use super::{TokenKind, tokens};
use std::{collections::BTreeMap, ops::Range};

pub(super) fn markdown(source: &str) -> Vec<(Range<usize>, TokenKind)> {
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
    let mut spans = Vec::new();
    let mut fence = None;
    for (event, range) in Parser::new_ext(source, Options::all()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_owned()
                    }
                    _ => String::new(),
                };
                fence = Some(language);
                spans.push((range, TokenKind::Comment));
            }
            Event::End(TagEnd::CodeBlock) => fence = None,
            Event::Text(_) if fence.is_some() => {
                let language = fence.as_deref().unwrap();
                // Do not recursively parse a Markdown example as another document.
                let language = if ["md", "markdown"].contains(&language) {
                    "text"
                } else {
                    language
                };
                spans.extend(tokens(&source[range.clone()], language).into_iter().map(
                    |(part, kind)| ((range.start + part.start)..(range.start + part.end), kind),
                ));
            }
            Event::Start(Tag::Heading { .. } | Tag::TableHead) => {
                spans.push((range, TokenKind::Key))
            }
            Event::Start(Tag::Strong | Tag::Emphasis | Tag::Strikethrough) => {
                spans.push((range, TokenKind::Type))
            }
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) => {
                spans.push((range, TokenKind::String))
            }
            Event::Code(_) => spans.push((range, TokenKind::String)),
            Event::Html(_) | Event::InlineHtml(_) => spans.push((range, TokenKind::Comment)),
            Event::Rule | Event::TaskListMarker(_) => spans.push((range, TokenKind::Key)),
            _ => {}
        }
    }
    overlay(source.len(), spans)
}

/// Later (more specific) spans override earlier containers. Sweep boundaries
/// rather than walking the whole source for every nested Markdown span.
fn overlay(len: usize, spans: Vec<(Range<usize>, TokenKind)>) -> Vec<(Range<usize>, TokenKind)> {
    let mut events = vec![(0, false, usize::MAX), (len, false, usize::MAX)];
    for (id, (range, _)) in spans.iter().enumerate() {
        if !range.is_empty() {
            events.push((range.start, true, id));
            events.push((range.end, false, id));
        }
    }
    events.sort_unstable();
    let mut active = BTreeMap::new();
    let mut previous = 0;
    let mut result: Vec<(Range<usize>, TokenKind)> = Vec::new();
    for (offset, start, id) in events {
        if offset > previous {
            let kind = active
                .last_key_value()
                .map_or(TokenKind::Plain, |(_, kind)| *kind);
            if let Some((range, _)) = result.last_mut().filter(|(_, last)| *last == kind) {
                range.end = offset;
            } else {
                result.push((previous..offset, kind));
            }
        }
        if id != usize::MAX {
            if start {
                active.insert(id, spans[id].1);
            } else {
                active.remove(&id);
            }
        }
        previous = offset;
    }
    result
}

fn string_end(source: &str, start: usize) -> usize {
    let quote = source.as_bytes()[start];
    let triple = source.as_bytes()[start..].starts_with(&[quote; 3]);
    let size = if triple { 3 } else { 1 };
    let delimiter = &source[start..start + size];
    let mut end = start + size;
    while end < source.len() {
        if source[end..].starts_with(delimiter) {
            return end + size;
        }
        let ch = source[end..].chars().next().unwrap();
        end += ch.len_utf8();
        if ch == '\\' && quote == b'"' && end < source.len() {
            end += source[end..].chars().next().unwrap().len_utf8();
        }
    }
    end
}

pub(super) fn toml(source: &str) -> Vec<(Range<usize>, TokenKind)> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < source.len() {
        let start = i;
        let ch = source[i..].chars().next().unwrap();
        let kind;
        if ch == '#' {
            i += source[i..].find('\n').unwrap_or(source.len() - i);
            kind = TokenKind::Comment;
        } else if ch == '"' || ch == '\'' {
            i = string_end(source, i);
            kind = if source[i..].trim_start().starts_with(['=', '.']) {
                TokenKind::Key
            } else {
                TokenKind::String
            };
        } else if ch == '['
            && source[..i]
                .rsplit('\n')
                .next()
                .unwrap_or("")
                .trim()
                .is_empty()
        {
            i += 1;
            while i < source.len() && !source[i..].starts_with(['\n', '#', ']']) {
                if source[i..].starts_with(['"', '\'']) {
                    i = string_end(source, i);
                } else {
                    i += source[i..].chars().next().unwrap().len_utf8();
                }
            }
            while source[i..].starts_with(']') {
                i += 1;
            }
            kind = TokenKind::Type;
        } else if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '+') {
            i += ch.len_utf8();
            while i < source.len() {
                let next = source[i..].chars().next().unwrap();
                if !next.is_ascii_alphanumeric() && !matches!(next, '_' | '-' | '+' | '.' | ':') {
                    break;
                }
                i += next.len_utf8();
            }
            let word = &source[start..i];
            kind = if source[i..].trim_start().starts_with('=') {
                TokenKind::Key
            } else if matches!(
                word,
                "true" | "false" | "inf" | "nan" | "+inf" | "-inf" | "+nan" | "-nan"
            ) {
                TokenKind::Key
            } else if ch.is_ascii_digit() || matches!(ch, '-' | '+') && word.len() > 1 {
                TokenKind::Number
            } else {
                TokenKind::Plain
            };
        } else {
            i += ch.len_utf8();
            kind = TokenKind::Plain;
        }
        result.push((start..i, kind));
    }
    result
}

pub(super) fn yaml(source: &str) -> Vec<(Range<usize>, TokenKind)> {
    let mut spans = Vec::new();
    let mut offset = 0;
    let mut block_indent = None;
    let mut quoted_until = 0;
    for line in source.split_inclusive('\n') {
        let end = offset + line.len();
        let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
        if let Some(base) = block_indent {
            if line.trim().is_empty() || indent > base {
                spans.push((offset..end, TokenKind::String));
                offset = end;
                continue;
            }
            block_indent = None;
        }
        let mut i = (offset + indent).max(quoted_until.min(end));
        let mut key_start = i;
        while i < end {
            let ch = source[i..].chars().next().unwrap();
            let previous = source[..i].chars().next_back();
            if ch == '#' && previous.is_none_or(char::is_whitespace) {
                spans.push((
                    i..(end - usize::from(line.ends_with('\n'))),
                    TokenKind::Comment,
                ));
                break;
            }
            if ch == '"' || ch == '\'' {
                let start = i;
                i += 1;
                while i < source.len() {
                    let next = source[i..].chars().next().unwrap();
                    i += next.len_utf8();
                    if next == ch {
                        if ch == '\'' && source[i..].starts_with('\'') {
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    if next == '\\' && ch == '"' && i < source.len() {
                        i += source[i..].chars().next().unwrap().len_utf8();
                    }
                }
                spans.push((start..i, TokenKind::String));
                quoted_until = i;
                continue;
            }
            let next = source[i + ch.len_utf8()..].chars().next();
            if ch == ':' && next.is_none_or(|c| c.is_whitespace() || ",[]{}".contains(c)) {
                let key = &source[key_start..i];
                let start = key_start + key.len() - key.trim_start().len();
                let finish = i - (key.len() - key.trim_end().len());
                if start < finish {
                    spans.push((start..finish, TokenKind::Key));
                }
                spans.push((i..i + 1, TokenKind::Type));
                i += 1;
                key_start = i;
                continue;
            }
            if (ch == '|' || ch == '>')
                && (i == offset + indent || previous.is_some_and(char::is_whitespace))
            {
                let start = i;
                i += 1;
                while i < end
                    && source[i..]
                        .starts_with(|c: char| c.is_ascii_digit() || matches!(c, '+' | '-'))
                {
                    i += 1;
                }
                spans.push((start..i, TokenKind::Key));
                block_indent = Some(indent);
                continue;
            }
            if matches!(ch, '&' | '*' | '!')
                && (i == offset + indent
                    || previous.is_some_and(|c| c.is_whitespace() || "[{,".contains(c)))
            {
                let start = i;
                i += 1;
                while i < end
                    && !source[i..].starts_with(|c: char| c.is_whitespace() || ",[]{}".contains(c))
                {
                    i += source[i..].chars().next().unwrap().len_utf8();
                }
                spans.push((start..i, TokenKind::Type));
                continue;
            }
            if matches!(ch, '{' | '[' | '}' | ']' | ',')
                || matches!(ch, '-' | '?') && next.is_some_and(char::is_whitespace)
            {
                spans.push((i..i + 1, TokenKind::Type));
                i += 1;
                key_start = i;
                continue;
            }
            if !ch.is_whitespace() {
                let start = i;
                i += ch.len_utf8();
                while i < end {
                    let next = source[i..].chars().next().unwrap();
                    if next.is_whitespace()
                        || ",[]{}".contains(next)
                        || next == ':'
                            && source[i + 1..]
                                .chars()
                                .next()
                                .is_none_or(|c| c.is_whitespace() || ",[]{}".contains(c))
                    {
                        break;
                    }
                    i += next.len_utf8();
                }
                let word = &source[start..i];
                let lower = word.to_ascii_lowercase();
                let kind = if matches!(
                    lower.as_str(),
                    "true" | "false" | "null" | "~" | "---" | "..."
                ) {
                    TokenKind::Key
                } else if word.parse::<f64>().is_ok()
                    || matches!(lower.as_str(), ".inf" | "-.inf" | "+.inf" | ".nan")
                    || ["0x", "0o", "0b"]
                        .iter()
                        .any(|prefix| lower.starts_with(prefix))
                {
                    TokenKind::Number
                } else {
                    TokenKind::Plain
                };
                if kind != TokenKind::Plain {
                    spans.push((start..i, kind));
                }
            } else {
                i += ch.len_utf8();
            }
        }
        offset = end;
    }
    overlay(source.len(), spans)
}

pub(super) fn script(source: &str, typescript: bool) -> Vec<(Range<usize>, TokenKind)> {
    let mut result = Vec::new();
    let mut i = 0;
    let mut regex_allowed = true;
    let mut type_name = false;
    while i < source.len() {
        let start = i;
        let ch = source[i..].chars().next().unwrap();
        let kind;
        if source[i..].starts_with("//") || i == 0 && source.starts_with("#!") {
            i += source[i..].find('\n').unwrap_or(source.len() - i);
            kind = TokenKind::Comment;
        } else if source[i..].starts_with("/*") {
            i = source[i + 2..]
                .find("*/")
                .map_or(source.len(), |end| i + 2 + end + 2);
            kind = TokenKind::Comment;
        } else if matches!(ch, '\'' | '"' | '`') {
            i += 1;
            while i < source.len() {
                let next = source[i..].chars().next().unwrap();
                i += next.len_utf8();
                if next == '\\' && i < source.len() {
                    i += source[i..].chars().next().unwrap().len_utf8();
                } else if next == ch {
                    break;
                }
            }
            kind = TokenKind::String;
            regex_allowed = false;
        } else if ch == '/' && regex_allowed {
            i += 1;
            let mut class = false;
            while i < source.len() {
                let next = source[i..].chars().next().unwrap();
                if matches!(next, '\n' | '\r') {
                    break;
                }
                i += next.len_utf8();
                if next == '\\' && i < source.len() {
                    i += source[i..].chars().next().unwrap().len_utf8();
                } else if next == '[' {
                    class = true;
                } else if next == ']' {
                    class = false;
                } else if next == '/' && !class {
                    while i < source.len() && source.as_bytes()[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    break;
                }
            }
            kind = TokenKind::String;
            regex_allowed = false;
        } else if ch.is_ascii_digit()
            || ch == '.' && source[i + 1..].starts_with(|c: char| c.is_ascii_digit())
        {
            if source[i..].starts_with("0x")
                || source[i..].starts_with("0X")
                || source[i..].starts_with("0b")
                || source[i..].starts_with("0B")
                || source[i..].starts_with("0o")
                || source[i..].starts_with("0O")
            {
                i += 2;
                while i < source.len()
                    && (source.as_bytes()[i].is_ascii_alphanumeric()
                        || source.as_bytes()[i] == b'_')
                {
                    i += 1;
                }
            } else {
                if ch == '.' {
                    i += 1;
                }
                while i < source.len()
                    && (source.as_bytes()[i].is_ascii_digit() || source.as_bytes()[i] == b'_')
                {
                    i += 1;
                }
                if ch != '.' && source[i..].starts_with('.') {
                    i += 1;
                    while i < source.len()
                        && (source.as_bytes()[i].is_ascii_digit() || source.as_bytes()[i] == b'_')
                    {
                        i += 1;
                    }
                }
                if source[i..].starts_with(['e', 'E']) {
                    i += 1;
                    if source[i..].starts_with(['+', '-']) {
                        i += 1;
                    }
                    while i < source.len()
                        && (source.as_bytes()[i].is_ascii_digit() || source.as_bytes()[i] == b'_')
                    {
                        i += 1;
                    }
                }
                if source[i..].starts_with('n') {
                    i += 1;
                }
            }
            kind = TokenKind::Number;
            regex_allowed = false;
        } else if ch.is_alphabetic() || matches!(ch, '_' | '$') {
            i += ch.len_utf8();
            while i < source.len() {
                let next = source[i..].chars().next().unwrap();
                if !next.is_alphanumeric() && !matches!(next, '_' | '$') {
                    break;
                }
                i += next.len_utf8();
            }
            let word = &source[start..i];
            let keyword = matches!(
                word,
                "async"
                    | "await"
                    | "break"
                    | "case"
                    | "catch"
                    | "class"
                    | "const"
                    | "continue"
                    | "debugger"
                    | "default"
                    | "delete"
                    | "do"
                    | "else"
                    | "export"
                    | "extends"
                    | "finally"
                    | "for"
                    | "from"
                    | "function"
                    | "get"
                    | "if"
                    | "import"
                    | "in"
                    | "instanceof"
                    | "let"
                    | "new"
                    | "of"
                    | "return"
                    | "set"
                    | "static"
                    | "super"
                    | "switch"
                    | "this"
                    | "throw"
                    | "try"
                    | "typeof"
                    | "var"
                    | "void"
                    | "while"
                    | "with"
                    | "yield"
                    | "true"
                    | "false"
                    | "null"
                    | "undefined"
            );
            let ts_keyword = typescript
                && matches!(
                    word,
                    "abstract"
                        | "accessor"
                        | "as"
                        | "asserts"
                        | "declare"
                        | "enum"
                        | "implements"
                        | "infer"
                        | "interface"
                        | "is"
                        | "keyof"
                        | "module"
                        | "namespace"
                        | "override"
                        | "private"
                        | "protected"
                        | "public"
                        | "readonly"
                        | "satisfies"
                        | "type"
                        | "unique"
                        | "using"
                );
            let primitive = typescript
                && matches!(
                    word,
                    "any"
                        | "bigint"
                        | "boolean"
                        | "intrinsic"
                        | "never"
                        | "number"
                        | "object"
                        | "string"
                        | "symbol"
                        | "unknown"
                        | "void"
                );
            kind = if primitive || type_name || ch.is_uppercase() {
                TokenKind::Type
            } else if keyword
                || ts_keyword
                || matches!(word, "require" | "module" | "exports")
                || source[i..].trim_start().starts_with(['(', ':'])
            {
                TokenKind::Key
            } else {
                TokenKind::Plain
            };
            type_name = matches!(word, "class" | "extends" | "new")
                || typescript
                    && matches!(
                        word,
                        "type" | "interface" | "enum" | "implements" | "namespace"
                    );
            regex_allowed = matches!(
                word,
                "return"
                    | "throw"
                    | "case"
                    | "yield"
                    | "await"
                    | "typeof"
                    | "void"
                    | "delete"
                    | "in"
                    | "instanceof"
                    | "new"
            );
        } else {
            i += ch.len_utf8();
            kind = TokenKind::Plain;
            if !ch.is_whitespace() {
                regex_allowed = !matches!(ch, ')' | ']' | '}' | '.');
                if !matches!(ch, ':' | '<' | ',' | '.') {
                    type_name = false;
                }
            }
        }
        super::push_web_token(&mut result, start..i, kind);
    }
    result
}

pub(super) fn python(source: &str) -> Vec<(Range<usize>, TokenKind)> {
    let mut result = Vec::new();
    let mut i = 0;
    let mut definition = None;
    while i < source.len() {
        let start = i;
        let ch = source[i..].chars().next().unwrap();
        let prefix = if ch.is_ascii_alphabetic() {
            let end = source[i..]
                .find(|c: char| !c.is_ascii_alphabetic())
                .map_or(source.len(), |end| i + end);
            let word = source[i..end].to_ascii_lowercase();
            (matches!(
                word.as_str(),
                "r" | "u" | "b" | "f" | "br" | "rb" | "fr" | "rf"
            ) && source[end..].starts_with(['\'', '"']))
            .then_some(end)
        } else {
            None
        };
        let kind;
        if ch == '#' {
            i += source[i..].find('\n').unwrap_or(source.len() - i);
            kind = TokenKind::Comment;
        } else if matches!(ch, '\'' | '"') || prefix.is_some() {
            let quote_start = prefix.unwrap_or(i);
            let quote = source.as_bytes()[quote_start];
            let triple = source.as_bytes()[quote_start..].starts_with(&[quote; 3]);
            let size = if triple { 3 } else { 1 };
            let delimiter = &source[quote_start..quote_start + size];
            i = quote_start + size;
            while i < source.len() {
                if source[i..].starts_with(delimiter) {
                    i += size;
                    break;
                }
                let next = source[i..].chars().next().unwrap();
                i += next.len_utf8();
                if next == '\\' && i < source.len() {
                    i += source[i..].chars().next().unwrap().len_utf8();
                } else if !triple && matches!(next, '\n' | '\r') {
                    break;
                }
            }
            kind = TokenKind::String;
        } else if ch == '@'
            && source[..i]
                .rsplit('\n')
                .next()
                .unwrap_or("")
                .trim()
                .is_empty()
            && source[i + 1..].starts_with(|c: char| c.is_alphabetic() || c == '_')
        {
            i += 1;
            while i < source.len()
                && source[i..].starts_with(|c: char| c.is_alphanumeric() || matches!(c, '_' | '.'))
            {
                i += source[i..].chars().next().unwrap().len_utf8();
            }
            kind = TokenKind::Type;
        } else if ch.is_ascii_digit()
            || ch == '.' && source[i + 1..].starts_with(|c: char| c.is_ascii_digit())
        {
            if ["0x", "0X", "0o", "0O", "0b", "0B"]
                .iter()
                .any(|prefix| source[i..].starts_with(prefix))
            {
                i += 2;
                while i < source.len()
                    && (source.as_bytes()[i].is_ascii_alphanumeric()
                        || source.as_bytes()[i] == b'_')
                {
                    i += 1;
                }
            } else {
                if ch == '.' {
                    i += 1;
                }
                while i < source.len()
                    && (source.as_bytes()[i].is_ascii_digit() || source.as_bytes()[i] == b'_')
                {
                    i += 1;
                }
                if ch != '.' && source[i..].starts_with('.') {
                    i += 1;
                    while i < source.len()
                        && (source.as_bytes()[i].is_ascii_digit() || source.as_bytes()[i] == b'_')
                    {
                        i += 1;
                    }
                }
                if source[i..].starts_with(['e', 'E']) {
                    i += 1;
                    if source[i..].starts_with(['+', '-']) {
                        i += 1;
                    }
                    while i < source.len()
                        && (source.as_bytes()[i].is_ascii_digit() || source.as_bytes()[i] == b'_')
                    {
                        i += 1;
                    }
                }
                if source[i..].starts_with(['j', 'J']) {
                    i += 1;
                }
            }
            kind = TokenKind::Number;
        } else if ch.is_alphabetic() || ch == '_' {
            i += ch.len_utf8();
            while i < source.len()
                && source[i..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
            {
                i += source[i..].chars().next().unwrap().len_utf8();
            }
            let word = &source[start..i];
            let keyword = matches!(
                word,
                "False"
                    | "None"
                    | "True"
                    | "and"
                    | "as"
                    | "assert"
                    | "async"
                    | "await"
                    | "break"
                    | "case"
                    | "class"
                    | "continue"
                    | "def"
                    | "del"
                    | "elif"
                    | "else"
                    | "except"
                    | "finally"
                    | "for"
                    | "from"
                    | "global"
                    | "if"
                    | "import"
                    | "in"
                    | "is"
                    | "lambda"
                    | "match"
                    | "nonlocal"
                    | "not"
                    | "or"
                    | "pass"
                    | "raise"
                    | "return"
                    | "try"
                    | "while"
                    | "with"
                    | "yield"
            );
            kind = definition.take().unwrap_or_else(|| {
                if keyword || source[i..].trim_start().starts_with('(') {
                    TokenKind::Key
                } else if ch.is_uppercase()
                    || matches!(
                        word,
                        "str"
                            | "int"
                            | "float"
                            | "complex"
                            | "bool"
                            | "list"
                            | "dict"
                            | "set"
                            | "tuple"
                            | "bytes"
                            | "object"
                            | "type"
                    )
                {
                    TokenKind::Type
                } else {
                    TokenKind::Plain
                }
            });
            definition = match word {
                "class" => Some(TokenKind::Type),
                "def" => Some(TokenKind::Key),
                _ => None,
            };
        } else {
            i += ch.len_utf8();
            kind = TokenKind::Plain;
        }
        super::push_web_token(&mut result, start..i, kind);
    }
    result
}

pub(super) fn dockerfile(source: &str) -> Vec<(Range<usize>, TokenKind)> {
    const INSTRUCTIONS: &[&str] = &[
        "ADD",
        "ARG",
        "CMD",
        "COPY",
        "ENTRYPOINT",
        "ENV",
        "EXPOSE",
        "FROM",
        "HEALTHCHECK",
        "LABEL",
        "MAINTAINER",
        "ONBUILD",
        "RUN",
        "SHELL",
        "STOPSIGNAL",
        "USER",
        "VOLUME",
        "WORKDIR",
    ];
    let mut spans = Vec::new();
    let mut offset = 0;
    let mut continued = false;
    let mut instruction = String::new();
    let mut heredoc = None::<String>;
    let mut escape = '\\';
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        if let Some(delimiter) = &heredoc {
            spans.push((offset..offset + line.len(), TokenKind::String));
            if trimmed == delimiter {
                heredoc = None;
            }
            offset += line.len();
            continue;
        }
        let end = offset + line.trim_end_matches(['\n', '\r']).len();
        let mut i = offset + line.len() - line.trim_start().len();
        if trimmed.starts_with('#') {
            spans.push((i..end, TokenKind::Comment));
            if let Some(value) = trimmed
                .trim_start_matches('#')
                .trim()
                .strip_prefix("escape=")
            {
                if let Some(ch @ ('\\' | '`')) = value.trim().chars().next() {
                    escape = ch;
                }
            }
            offset += line.len();
            continue;
        }
        let mut first = !continued;
        let mut stage = false;
        while i < end {
            let start = i;
            let ch = source[i..].chars().next().unwrap();
            if ch.is_whitespace() {
                i += ch.len_utf8();
                continue;
            }
            if ch == '\'' || ch == '"' {
                i += 1;
                while i < end {
                    let next = source[i..].chars().next().unwrap();
                    i += next.len_utf8();
                    if next == '\\' && i < end {
                        i += source[i..].chars().next().unwrap().len_utf8();
                    } else if next == ch {
                        break;
                    }
                }
                spans.push((start..i, TokenKind::String));
            } else if ch == '$' {
                i += 1;
                if source[i..].starts_with('{') {
                    i = source[i..end].find('}').map_or(end, |close| i + close + 1);
                } else {
                    while i < end
                        && source[i..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
                    {
                        i += source[i..].chars().next().unwrap().len_utf8();
                    }
                }
                spans.push((start..i, TokenKind::Type));
            } else if source[i..].starts_with("<<") {
                i += 2;
                if source[i..].starts_with('-') {
                    i += 1;
                }
                let delimiter_start = i;
                while i < end && !source[i..].starts_with(char::is_whitespace) {
                    i += source[i..].chars().next().unwrap().len_utf8();
                }
                let delimiter = source[delimiter_start..i].trim_matches(['\'', '"']);
                if !delimiter.is_empty() {
                    heredoc = Some(delimiter.to_owned());
                }
                spans.push((start..i, TokenKind::Type));
            } else if source[i..].starts_with("--") {
                i += 2;
                while i < end
                    && source[i..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '-')
                {
                    i += 1;
                }
                spans.push((start..i, TokenKind::Key));
            } else if matches!(ch, '=' | ',' | '[' | ']' | '\\') || ch == escape {
                i += ch.len_utf8();
            } else {
                i += ch.len_utf8();
                while i < end
                    && !source[i..]
                        .starts_with(|c: char| c.is_whitespace() || "=,[]\\$\"'".contains(c))
                {
                    i += source[i..].chars().next().unwrap().len_utf8();
                }
                let word = &source[start..i];
                let upper = word.to_ascii_uppercase();
                let kind = if first && INSTRUCTIONS.contains(&upper.as_str()) {
                    instruction = upper.clone();
                    TokenKind::Key
                } else if instruction == "FROM" && upper == "AS" {
                    stage = true;
                    TokenKind::Key
                } else if stage {
                    stage = false;
                    TokenKind::Type
                } else if instruction == "ONBUILD" && INSTRUCTIONS.contains(&upper.as_str())
                    || instruction == "HEALTHCHECK" && matches!(upper.as_str(), "CMD" | "NONE")
                {
                    TokenKind::Key
                } else if source[i..].starts_with('=') {
                    TokenKind::Key
                } else if word.parse::<f64>().is_ok() {
                    TokenKind::Number
                } else if instruction == "FROM" {
                    TokenKind::String
                } else {
                    TokenKind::Plain
                };
                first = false;
                if kind != TokenKind::Plain {
                    spans.push((start..i, kind));
                }
            }
        }
        if !trimmed.is_empty() {
            continued = trimmed.ends_with(escape);
        }
        offset += line.len();
    }
    overlay(source.len(), spans)
}

pub(super) fn makefile(source: &str) -> Vec<(Range<usize>, TokenKind)> {
    let mut spans = Vec::new();
    let mut offset = 0;
    let mut recipe_prefix = '\t';
    let mut continued_recipe = false;
    let mut define = false;
    for line in source.split_inclusive('\n') {
        let end = offset + line.trim_end_matches(['\n', '\r']).len();
        let trimmed = line.trim_start();
        let begin = offset + line.len() - trimmed.len();
        let recipe = line.starts_with(recipe_prefix) || continued_recipe;
        let word_end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
        let word = &trimmed[..word_end];
        let directive = !recipe
            && matches!(
                word,
                "define"
                    | "endef"
                    | "include"
                    | "-include"
                    | "sinclude"
                    | "ifdef"
                    | "ifndef"
                    | "ifeq"
                    | "ifneq"
                    | "else"
                    | "endif"
                    | "export"
                    | "unexport"
                    | "override"
                    | "private"
                    | "vpath"
                    | "undefine"
            );
        if directive {
            spans.push((begin..begin + word.len(), TokenKind::Key));
        }
        if word == "endef" && !recipe {
            define = false;
        }
        let mut i = if recipe && line.starts_with(recipe_prefix) {
            offset + recipe_prefix.len_utf8()
        } else {
            begin
        };
        let mut comment = None;
        while i < end {
            let ch = source[i..].chars().next().unwrap();
            if ch == '\\' && i + 1 < end {
                i += 1 + source[i + 1..].chars().next().unwrap().len_utf8();
            } else if ch == '#' {
                comment = Some(i);
                break;
            } else if recipe && matches!(ch, '\'' | '"') {
                let start = i;
                i += 1;
                while i < end {
                    let next = source[i..].chars().next().unwrap();
                    i += next.len_utf8();
                    if next == '\\' && i < end {
                        i += source[i..].chars().next().unwrap().len_utf8();
                    } else if next == ch {
                        break;
                    }
                }
                spans.push((start..i, TokenKind::String));
            } else {
                i += ch.len_utf8();
            }
        }
        let content_end = comment.unwrap_or(end);
        if recipe {
            let mut start = if line.starts_with(recipe_prefix) {
                offset + recipe_prefix.len_utf8()
            } else {
                begin.min(content_end)
            };
            while start < content_end
                && source[start..].starts_with(|c: char| c.is_whitespace() || "@-+".contains(c))
            {
                start += source[start..].chars().next().unwrap().len_utf8();
            }
            let finish = source[start..content_end]
                .find(char::is_whitespace)
                .map_or(content_end, |end| start + end);
            if start < finish {
                spans.push((start..finish, TokenKind::Key));
            }
        } else if define && word != "define" {
            spans.push((begin.min(content_end)..content_end, TokenKind::String));
        } else {
            let content = &source[begin.min(content_end)..content_end];
            let equal = content.find('=');
            let colon = content.find(':').filter(|colon| {
                !content[*colon..].starts_with(":=")
                    && !content[*colon..].starts_with("::=")
                    && !content[*colon..].starts_with(":::=")
                    && equal.is_none_or(|equal| *colon < equal)
            });
            if let Some(colon) = colon {
                spans.push((begin..begin + colon, TokenKind::Type));
            }
            if let Some(equal) = equal {
                let start = colon.map_or(begin, |colon| begin + colon + 1);
                let key = source[start..begin + equal]
                    .trim()
                    .trim_end_matches([':', '?', '+', '!'])
                    .trim_end();
                let start = start + source[start..begin + equal].len()
                    - source[start..begin + equal].trim_start().len();
                if !key.is_empty() {
                    spans.push((start..start + key.len(), TokenKind::Key));
                }
                if content.starts_with(".RECIPEPREFIX") {
                    if let Some(prefix) = content[equal + 1..].trim_start().chars().next() {
                        recipe_prefix = prefix;
                    }
                }
            }
        }
        // Make expansion applies even inside shell quotes; overlay references
        // after strings, but before comments. Walk nested expansions as well.
        i = offset;
        while i < content_end {
            if source[i..].starts_with('$') {
                let start = i;
                let next = source[i + 1..].chars().next();
                if let Some(open @ ('(' | '{')) = next {
                    let close = if open == '(' { ')' } else { '}' };
                    let mut nesting = 1;
                    let mut finish = i + 2;
                    while finish < content_end && nesting > 0 {
                        let ch = source[finish..].chars().next().unwrap();
                        finish += ch.len_utf8();
                        if ch == open {
                            nesting += 1;
                        }
                        if ch == close {
                            nesting -= 1;
                        }
                    }
                    spans.push((start..finish, TokenKind::Type));
                    let name_start = i + 2;
                    let name_end = source[name_start..finish]
                        .find(|c: char| c.is_whitespace() || matches!(c, ',' | ')' | '}'))
                        .map_or(finish, |end| name_start + end);
                    if matches!(
                        &source[name_start..name_end],
                        "abspath"
                            | "addprefix"
                            | "addsuffix"
                            | "and"
                            | "basename"
                            | "call"
                            | "dir"
                            | "error"
                            | "eval"
                            | "file"
                            | "filter"
                            | "filter-out"
                            | "findstring"
                            | "firstword"
                            | "flavor"
                            | "foreach"
                            | "if"
                            | "info"
                            | "join"
                            | "lastword"
                            | "notdir"
                            | "or"
                            | "origin"
                            | "patsubst"
                            | "realpath"
                            | "shell"
                            | "sort"
                            | "strip"
                            | "subst"
                            | "suffix"
                            | "value"
                            | "warning"
                            | "wildcard"
                            | "word"
                            | "wordlist"
                            | "words"
                    ) {
                        spans.push((name_start..name_end, TokenKind::Key));
                    }
                    i += 2;
                } else if let Some(next) = next {
                    spans.push((start..start + 1 + next.len_utf8(), TokenKind::Type));
                    i += 1 + next.len_utf8();
                } else {
                    i += 1;
                }
            } else {
                i += source[i..].chars().next().unwrap().len_utf8();
            }
        }
        if let Some(comment) = comment {
            spans.push((comment..end, TokenKind::Comment));
        }
        if !recipe && word == "define" {
            define = true;
        }
        continued_recipe = recipe && line.trim_end().ends_with('\\');
        offset += line.len();
    }
    overlay(source.len(), spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(source: &str, ranges: &[(Range<usize>, TokenKind)]) {
        let mut offset = 0;
        for (range, _) in ranges {
            assert_eq!(range.start, offset);
            assert!(source.is_char_boundary(range.start) && source.is_char_boundary(range.end));
            offset = range.end;
        }
        assert_eq!(offset, source.len());
    }
    #[test]
    fn makefile_highlights_rules_assignments_functions_and_recipes() {
        let source = "# 中文\nCC := clang\nSOURCES = $(wildcard src/*.c)\nOBJECTS = $(patsubst %.c,%.o,$(SOURCES))\n.PHONY: all clean\nall: $(OBJECTS)\n%.o: %.c\n\t@$(CC) -c $< -o $@ # compile\nclean:\n\t@echo \"remove $(OBJECTS) # literal\"\n\trm -f $(OBJECTS)\ninclude config.mk\nifdef DEBUG\nCFLAGS += -g\nendif\n";
        let ranges = tokens(source, "makefile");
        check(source, &ranges);
        for alias in ["Make", "mk", "gnumake"] {
            assert_eq!(ranges, tokens(source, alias));
        }
        for (part, kind) in [
            ("# 中文", TokenKind::Comment),
            ("CC", TokenKind::Key),
            ("wildcard", TokenKind::Key),
            ("patsubst", TokenKind::Key),
            ("$(SOURCES)", TokenKind::Type),
            (".PHONY", TokenKind::Type),
            ("%.o", TokenKind::Type),
            ("$<", TokenKind::Type),
            ("$@", TokenKind::Type),
            ("# compile", TokenKind::Comment),
            ("include", TokenKind::Key),
            ("ifdef", TokenKind::Key),
            ("endif", TokenKind::Key),
        ] {
            let start = source.find(part).unwrap();
            assert!(
                ranges
                    .iter()
                    .any(|(r, k)| r.start <= start && r.end >= start + part.len() && *k == kind),
                "missing {part}"
            );
        }
        let hash = source.find("# literal").unwrap();
        assert!(
            ranges
                .iter()
                .any(|(r, k)| r.contains(&hash) && *k == TokenKind::String)
        );
        for source in [
            "",
            "x := $(未完成",
            "all:\n\t@echo hello \\\n\nnext:\n\t@echo world\n",
            "define TEXT\n多行 $(VALUE)\nendef\n",
            ".RECIPEPREFIX := >\nall:\n>@echo hi\n",
            "var = hello \\#literal # comment\n",
            "all: VAR := value\n\t@echo $$HOME ${VAR}\n",
        ] {
            check(source, &makefile(source));
        }
        let markdown = "```make\nall:\n\techo $@\n```\n";
        let ranges = tokens(markdown, "md");
        let start = markdown.find("$@").unwrap();
        assert!(
            ranges
                .iter()
                .any(|(r, k)| r.contains(&start) && *k == TokenKind::Type)
        );
    }

    #[test]
    fn dockerfile_highlights_stages_flags_variables_and_heredocs() {
        let source = "# syntax=docker/dockerfile:1\nARG BASE=python:3.12\nFROM --platform=$BUILDPLATFORM ${BASE} AS build\nWORKDIR /app\nCOPY --from=build . .\nENV PORT=8080\nRUN echo \"中文 # literal\" && \\\n    echo ${PORT}\nEXPOSE 8080\nRUN <<'EOF'\n# shell body\necho hello\nEOF\nCMD [\"python\", \"app.py\"]\n";
        let ranges = tokens(source, "dockerfile");
        check(source, &ranges);
        for language in ["Docker", "containerfile"] {
            assert_eq!(ranges, tokens(source, language));
        }
        for (part, kind) in [
            ("# syntax=docker/dockerfile:1", TokenKind::Comment),
            ("ARG", TokenKind::Key),
            ("FROM", TokenKind::Key),
            ("--platform", TokenKind::Key),
            ("$BUILDPLATFORM", TokenKind::Type),
            ("${BASE}", TokenKind::Type),
            ("AS", TokenKind::Key),
            ("build", TokenKind::Type),
            ("--from", TokenKind::Key),
            ("PORT", TokenKind::Key),
            ("8080", TokenKind::Number),
            ("\"中文 # literal\"", TokenKind::String),
            ("# shell body", TokenKind::String),
            ("CMD", TokenKind::Key),
            ("\"app.py\"", TokenKind::String),
        ] {
            let start = source.find(part).unwrap();
            assert!(
                ranges
                    .iter()
                    .any(|(r, k)| r.start <= start && r.end >= start + part.len() && *k == kind),
                "missing {part}"
            );
        }
        for source in [
            "",
            "FROM ubuntu",
            "RUN echo ${UNFINISHED",
            "RUN <<EOF\n中文",
            "# escape=`\nRUN echo hello `\n && echo world\n",
        ] {
            check(source, &dockerfile(source));
        }
    }

    #[test]
    fn python_highlights_definitions_decorators_prefixed_strings_and_numbers() {
        let source = "@functools.cache\nasync def load(name: str) -> int:\n    text = fr'''多行 {name}\n# 非注释\n'''\n    path = r'C:\\tmp'\n    return 1_000 + .5e-2 + 0xFF + 2j # 注释\nclass 用户:\n    pass\n";
        let ranges = tokens(source, "python");
        check(source, &ranges);
        for language in ["py", "Python3"] {
            assert_eq!(ranges, tokens(source, language));
        }
        for (part, kind) in [
            ("@functools.cache", TokenKind::Type),
            ("async", TokenKind::Key),
            ("def", TokenKind::Key),
            ("load", TokenKind::Key),
            ("str", TokenKind::Type),
            ("int", TokenKind::Type),
            ("fr'''多行 {name}\n# 非注释\n'''", TokenKind::String),
            ("r'C:\\tmp'", TokenKind::String),
            ("1_000", TokenKind::Number),
            (".5e-2", TokenKind::Number),
            ("0xFF", TokenKind::Number),
            ("2j", TokenKind::Number),
            ("# 注释", TokenKind::Comment),
            ("用户", TokenKind::Type),
        ] {
            let start = source.find(part).unwrap();
            assert!(
                ranges
                    .iter()
                    .any(|(r, k)| r.start <= start && r.end >= start + part.len() && *k == kind),
                "missing {part}"
            );
        }
        for source in [
            "",
            "'''未完成\n中文",
            "x = '未完成\npass",
            "value = b'bytes'",
            "return '\\''",
            "数据 = [1, 2]",
        ] {
            check(source, &python(source));
        }
        let markdown = "```python\ndef load():\n    return True\n```\n";
        let ranges = tokens(markdown, "md");
        let at = markdown.find("def").unwrap();
        assert!(
            ranges
                .iter()
                .any(|(r, k)| r.contains(&at) && *k == TokenKind::Key)
        );
    }

    #[test]
    fn yaml_handles_mappings_anchors_block_scalars_and_literal_hashes() {
        let source = "---\nname: 中文\nlong key: 'it''s # literal' # comment\nenabled: true\ncount: 12.5\nbase: &defaults { value: null }\ncopy: *defaults\nurl: https://example.com/#anchor\nscript: |-\n  echo '# content'\n  # still scalar\nnext: !!str value\n";
        let ranges = tokens(source, "YAML");
        check(source, &ranges);
        assert_eq!(ranges, tokens(source, "yml"));
        for (part, kind) in [
            ("name", TokenKind::Key),
            ("long key", TokenKind::Key),
            ("'it''s # literal'", TokenKind::String),
            ("# comment", TokenKind::Comment),
            ("true", TokenKind::Key),
            ("12.5", TokenKind::Number),
            ("&defaults", TokenKind::Type),
            ("*defaults", TokenKind::Type),
            ("!!str", TokenKind::Type),
            ("# still scalar", TokenKind::String),
        ] {
            let start = source.find(part).unwrap();
            assert!(
                ranges
                    .iter()
                    .any(|(r, k)| r.start <= start && r.end >= start + part.len() && *k == kind),
                "missing {part}: {ranges:?}"
            );
        }
        let hash = source.find("#anchor").unwrap();
        assert!(
            !ranges
                .iter()
                .any(|(r, k)| r.contains(&hash) && *k == TokenKind::Comment)
        );
        for source in [
            "",
            "'未结束",
            "value: \"多行\n字符串\"\nnext: 3",
            "a: { 中文: true, x: 1 }",
            "items:\n - one\n - two\n",
            "# 注释\r\n",
        ] {
            check(source, &yaml(source));
        }
    }

    #[test]
    fn javascript_aliases_highlight_commonjs_regex_templates_and_numbers() {
        let source = "#!/usr/bin/env node\nconst fs = require('node:fs'); // 中文\nmodule.exports = async function load() {\n const pattern = /[\\/]/gu;\n const text = `中文 ${value}`;\n return total / count + 1.5e-3 + 1_000n + 0xFF;\n};\n";
        let ranges = tokens(source, "js");
        check(source, &ranges);
        for language in ["JavaScript", "mjs", "CJS"] {
            assert_eq!(ranges, tokens(source, language));
        }
        for (part, kind) in [
            ("#!/usr/bin/env node", TokenKind::Comment),
            ("const", TokenKind::Key),
            ("require", TokenKind::Key),
            ("module", TokenKind::Key),
            ("exports", TokenKind::Key),
            ("'node:fs'", TokenKind::String),
            ("/[\\/]/gu", TokenKind::String),
            ("`中文 ${value}`", TokenKind::String),
            ("1.5e-3", TokenKind::Number),
            ("1_000n", TokenKind::Number),
            ("0xFF", TokenKind::Number),
        ] {
            let start = source.find(part).unwrap();
            assert!(
                ranges
                    .iter()
                    .any(|(r, k)| r.start <= start && r.end >= start + part.len() && *k == kind),
                "missing {part}"
            );
        }
        let division = source.find("/ count").unwrap();
        assert!(
            !ranges
                .iter()
                .any(|(r, k)| r.contains(&division) && *k == TokenKind::String)
        );
        for source in [
            "",
            "/* 未完成",
            "`未结束 ${",
            "const 中文 = .5;",
            "let x = /未完成",
            "x / 2 / 3",
        ] {
            check(source, &script(source, false));
        }
    }

    #[test]
    fn typescript_highlights_interfaces_aliases_and_primitive_types() {
        let source = "export interface User<T> { readonly name: string; value: T; }\ntype userId = number | null;\nconst user: User<string> = load() satisfies User<string>;\n";
        let ranges = tokens(source, "TS");
        check(source, &ranges);
        for language in ["typescript", "mts", "cts"] {
            assert_eq!(ranges, tokens(source, language));
        }
        for (part, kind) in [
            ("interface", TokenKind::Key),
            ("User", TokenKind::Type),
            ("readonly", TokenKind::Key),
            ("name", TokenKind::Key),
            ("string", TokenKind::Type),
            ("type", TokenKind::Key),
            ("userId", TokenKind::Type),
            ("number", TokenKind::Type),
            ("satisfies", TokenKind::Key),
        ] {
            let start = source.find(part).unwrap();
            assert!(
                ranges
                    .iter()
                    .any(|(r, k)| r.start <= start && r.end >= start + part.len() && *k == kind),
                "missing {part}"
            );
        }
        let markdown = "```ts\ninterface User { name: string; }\n```\n";
        let ranges = tokens(markdown, "md");
        check(markdown, &ranges);
        let at = markdown.find("string").unwrap();
        assert!(
            ranges
                .iter()
                .any(|(r, k)| r.contains(&at) && *k == TokenKind::Type)
        );
    }

    #[test]
    fn markdown_preserves_source_and_highlights_fenced_toml() {
        let source = "# 中文标题\n\n**强调** 和 `代码` [链接](https://example.com)\n\n```toml\nname = \"中文\" # note\n```\n";
        let ranges = markdown(source);
        check(source, &ranges);
        for (part, kind) in [
            ("# 中文标题", TokenKind::Key),
            ("**强调**", TokenKind::Type),
            ("`代码`", TokenKind::String),
            ("name", TokenKind::Key),
            ("\"中文\"", TokenKind::String),
            ("# note", TokenKind::Comment),
        ] {
            let start = source.find(part).unwrap();
            assert!(
                ranges
                    .iter()
                    .any(|(r, k)| r.start <= start && r.end >= start + part.len() && *k == kind),
                "missing {part}: {ranges:?}"
            );
        }
    }
    #[test]
    fn toml_handles_quoted_keys_multiline_strings_dates_and_comments() {
        let source = "[package]\nname = \"应用#一\" # comment\n\"键\" = '''多行\n# 非注释\n'''\nenabled = true\nwhen = 2026-11-08T12:30:00Z\nvalue = -1.5e+2\n[[items]]\ninline = { nested = 0xFF }\n";
        let ranges = toml(source);
        check(source, &ranges);
        for (part, kind) in [
            ("[package]", TokenKind::Type),
            ("\"键\"", TokenKind::Key),
            ("\"应用#一\"", TokenKind::String),
            ("# comment", TokenKind::Comment),
            ("'''多行\n# 非注释\n'''", TokenKind::String),
            ("2026-11-08T12:30:00Z", TokenKind::Number),
            ("-1.5e+2", TokenKind::Number),
            ("nested", TokenKind::Key),
            ("0xFF", TokenKind::Number),
        ] {
            assert!(
                ranges
                    .iter()
                    .any(|(r, k)| &source[r.clone()] == part && *k == kind),
                "missing {part}"
            );
        }
        for source in [
            "",
            "\"未结束",
            "'''多行\n未结束",
            "# 注释",
            "[\"包含]的键\"]\nx=1",
        ] {
            check(source, &toml(source));
        }
    }
}
