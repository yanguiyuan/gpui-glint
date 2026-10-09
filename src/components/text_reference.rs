use std::{ops::Range, rc::Rc};

/// Resolves a complete `{{name}}` expression without changing the stored input.
pub type TextReferenceResolver = Rc<dyn Fn(&str) -> Result<String, String>>;

pub(super) fn reference_colors(theme: crate::theme::ThemePreset, warning: bool) -> (u32, u32) {
    if warning {
        if theme.is_dark() {
            (0xffc56d, 0x49331b)
        } else {
            (0xa56308, 0xffedcd)
        }
    } else {
        (theme.palette().accent, theme.palette().accent_tint)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct TextReference {
    pub range: Range<usize>,
    pub value: Result<String, String>,
}

pub(super) fn references(text: &str, resolver: &TextReferenceResolver) -> Vec<TextReference> {
    let mut result = Vec::new();
    let mut offset = 0;
    while let Some(start) = text[offset..].find("{{") {
        let start = offset + start;
        let end = text[start + 2..].find("}}").map(|end| start + 2 + end + 2);
        let range = start..end.unwrap_or(text.len());
        let value = if end.is_some() {
            resolver(&text[range.clone()])
        } else {
            Err("变量引用缺少结束符 }}".into())
        };
        offset = range.end;
        result.push(TextReference { range, value });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_preserve_utf8_ranges_and_independent_errors() {
        let resolver: TextReferenceResolver = Rc::new(|expression| match expression {
            "{{host}}" => Ok("服务.example".into()),
            "{{empty}}" => Err("空值".into()),
            _ => Err("未定义".into()),
        });
        let raw = "https://{{host}}/中文?x={{missing}}&y={{host}}{{empty}}";
        let refs = references(raw, &resolver);
        assert_eq!(refs.len(), 4);
        assert_eq!(&raw[refs[0].range.clone()], "{{host}}");
        assert_eq!(refs[0].value, Ok("服务.example".into()));
        assert!(refs[1].value.is_err());
        assert_eq!(&raw[refs[2].range.clone()], "{{host}}");
        assert!(refs[3].value.is_err());
        assert_eq!(&raw[refs[3].range.clone()], "{{empty}}");
    }

    #[test]
    fn incomplete_expression_is_a_warning_without_calling_the_resolver() {
        let resolver: TextReferenceResolver = Rc::new(|_| panic!("incomplete expression"));
        let raw = "前缀{{host";
        let refs = references(raw, &resolver);
        assert_eq!(refs.len(), 1);
        assert_eq!(&raw[refs[0].range.clone()], "{{host");
        assert!(refs[0].value.is_err());
        assert!(references("plain text", &resolver).is_empty());
    }
}
