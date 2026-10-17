use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub(super) struct JsonFold {
    pub end_line: usize,
    pub close_offset: usize,
}

/// Multiline containers only; delimiters inside JSON strings are never structural.
pub(super) fn folds(text: &str) -> HashMap<usize, JsonFold> {
    if serde_json::from_str::<serde_json::Value>(text).is_err() {
        return HashMap::new();
    }
    let mut folds = HashMap::<usize, JsonFold>::new();
    let mut stack = vec![];
    let mut line = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (offset, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            line += 1;
        }
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' | b'[' => stack.push(line),
            b'}' | b']' => {
                if let Some(start) = stack.pop()
                    && line > start
                {
                    let fold = JsonFold {
                        end_line: line,
                        close_offset: offset,
                    };
                    if folds
                        .get(&start)
                        .is_none_or(|existing| existing.end_line < line)
                    {
                        folds.insert(start, fold);
                    }
                }
            }
            _ => {}
        }
    }
    folds
}

pub(super) fn visible_lines(
    count: usize,
    folds: &HashMap<usize, JsonFold>,
    collapsed: &HashSet<usize>,
) -> Vec<usize> {
    let mut visible = vec![];
    let mut line = 0;
    while line < count {
        visible.push(line);
        line = if collapsed.contains(&line) {
            folds.get(&line).map_or(line + 1, |fold| fold.end_line + 1)
        } else {
            line + 1
        };
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_arrays_objects_and_string_delimiters_keep_source_line_numbers() {
        let source = "{\n  \"items\": [\n    {\n      \"name\": \"中 [ } \\\"\"\n    },\n    {\n      \"id\": 2\n    }\n  ],\n  \"count\": 2\n}";
        let folds = folds(source);
        assert_eq!(folds[&1].end_line, 8);
        assert_eq!(folds[&2].end_line, 4);
        assert_eq!(folds[&5].end_line, 7);
        assert_eq!(
            visible_lines(11, &folds, &HashSet::from([2])),
            vec![0, 1, 2, 5, 6, 7, 8, 9, 10]
        );
        assert_eq!(
            visible_lines(11, &folds, &HashSet::from([1, 2])),
            vec![0, 1, 9, 10]
        );
        assert_eq!(visible_lines(11, &folds, &HashSet::from([0])), vec![0]);
        assert!(super::folds("{\"single\": []}").is_empty());
        assert!(super::folds("{\ninvalid\n}").is_empty());
    }
}
