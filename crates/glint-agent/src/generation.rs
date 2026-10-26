use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GenerationStats {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Occupancy of the final request, separate from cumulative turn billing.
    #[serde(default)]
    pub context_tokens: Option<u64>,
    #[serde(default)]
    pub context_window: Option<u64>,
    pub elapsed_ms: u64,
    pub first_token_ms: Option<u64>,
    pub generation_ms: Option<u64>,
}

impl GenerationStats {
    pub fn output_tps(&self) -> Option<f64> {
        let duration = self.generation_ms.unwrap_or(self.elapsed_ms);
        match (self.output_tokens, duration) {
            (Some(tokens), duration) if duration > 0 => {
                Some(tokens as f64 * 1000. / duration as f64)
            }
            _ => None,
        }
    }

    pub fn caption(&self) -> String {
        let mut parts = Vec::new();
        if let Some(tokens) = self.output_tokens {
            parts.push(format!("输出 {tokens} tokens"));
        }
        if let Some(rate) = self.output_tps() {
            parts.push(format!(
                "{} {rate:.1} tokens/s",
                if self.generation_ms.is_some() {
                    "生成"
                } else {
                    "均速"
                }
            ));
        }
        if let Some(first) = self.first_token_ms {
            parts.push(format!("首个输出 {:.2}s", first as f64 / 1000.));
        }
        parts.push(format!("总耗时 {:.2}s", self.elapsed_ms as f64 / 1000.));
        parts.join(" · ")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ContentBlock {
    Text(String),
    Thinking(String),
    ToolCall(crate::tools::ToolCall),
    ToolResult(crate::tools::ToolRecord),
}

#[derive(Clone)]
enum StreamPart {
    Text(String),
    Thinking(String),
    Tool(usize),
}

pub fn anthropic_blocks(content: &Value) -> Vec<ContentBlock> {
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| match block["type"].as_str()? {
            "text" => Some(ContentBlock::Text(block["text"].as_str()?.into())),
            "thinking" => Some(ContentBlock::Thinking(block["thinking"].as_str()?.into())),
            "tool_use" => Some(ContentBlock::ToolCall(crate::tools::ToolCall {
                id: block["id"].as_str()?.into(),
                name: block["name"].as_str()?.into(),
                arguments: block["input"].clone(),
            })),
            _ => None,
        })
        .collect()
}

#[derive(Debug, Default)]
pub struct Completion {
    pub blocks: Vec<ContentBlock>,
    pub text: String,
    pub stats: GenerationStats,
    pub tool_calls: Vec<crate::tools::ToolCall>,
    pub tool_records: Vec<crate::tools::ToolRecord>,
    pub raw_content: Value,
    pub reasoning: String,
}

#[derive(Default)]
pub struct Metrics {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    context_input_tokens: Option<u64>,
    context_window: Option<u64>,
    first_output: Option<u64>,
    last_output: Option<u64>,
    output_updates: usize,
}

impl Metrics {
    pub fn output(&mut self, elapsed_ms: u64) {
        self.first_output.get_or_insert(elapsed_ms);
        self.last_output = Some(elapsed_ms);
        self.output_updates += 1;
    }

    pub fn usage(&mut self, value: &Value) {
        if let Some(input) = value["prompt_tokens"]
            .as_u64()
            .or(value["input_tokens"].as_u64())
            .or(value["inputTokens"].as_u64())
        {
            self.input_tokens = Some(input);
            self.context_input_tokens = Some(
                input
                    .saturating_add(value["cache_read_input_tokens"].as_u64().unwrap_or(0))
                    .saturating_add(value["cache_creation_input_tokens"].as_u64().unwrap_or(0)),
            );
        }
        if let Some(output) = value["completion_tokens"]
            .as_u64()
            .or(value["output_tokens"].as_u64())
            .or(value["outputTokens"].as_u64())
        {
            self.output_tokens = Some(output);
        }
    }

    pub fn context_tokens(&self) -> Option<u64> {
        self.context_input_tokens
            .zip(self.output_tokens)
            .map(|(input, output)| input.saturating_add(output))
    }

    pub fn set_context_window(&mut self, capacity: Option<u64>) {
        if let Some(capacity) = capacity.filter(|capacity| *capacity > 0) {
            self.context_window = Some(capacity);
        }
    }

    pub fn finish(self, elapsed_ms: u64) -> GenerationStats {
        let generation_ms = match (self.first_output, self.last_output) {
            (Some(first), Some(last)) if self.output_updates > 1 && last > first => {
                Some(last - first)
            }
            _ => None,
        };
        GenerationStats {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            context_tokens: self.context_tokens(),
            context_window: self.context_window,
            elapsed_ms,
            first_token_ms: self.first_output,
            generation_ms,
        }
    }
}

/// Newline/event framing is decoded after assembling complete byte lines, so
/// arbitrary network chunks cannot split UTF-8 characters in JSON payloads.
#[derive(Default)]
pub struct StreamCompletion {
    order: Vec<StreamPart>,
    blocks: std::collections::BTreeMap<usize, Value>,
    inputs: std::collections::HashMap<usize, String>,
    calls: std::collections::BTreeMap<usize, Value>,
    reasoning: String,
    pending: Vec<u8>,
    data: Vec<String>,
    text: String,
    metrics: Metrics,
    finished: bool,
    pub ended: bool,
}

impl StreamCompletion {
    pub fn context_tokens(&self) -> Option<u64> {
        self.metrics.context_tokens()
    }

    pub fn push(&mut self, bytes: &[u8], elapsed_ms: u64) -> Result<(), String> {
        self.pending.extend_from_slice(bytes);
        while let Some(index) = self.pending.iter().position(|byte| *byte == b'\n') {
            let bytes: Vec<_> = self.pending.drain(..=index).collect();
            let line = std::str::from_utf8(&bytes)
                .map_err(|_| "Provider 返回了无效 UTF-8 流")?
                .trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                self.flush(elapsed_ms)?;
            } else if let Some(data) = line.strip_prefix("data:") {
                self.data
                    .push(data.strip_prefix(' ').unwrap_or(data).to_owned());
            }
        }
        Ok(())
    }

    fn append_part(&mut self, text: &str, thinking: bool) {
        match (self.order.last_mut(), thinking) {
            (Some(StreamPart::Text(previous)), false)
            | (Some(StreamPart::Thinking(previous)), true) => previous.push_str(text),
            _ => self.order.push(if thinking {
                StreamPart::Thinking(text.into())
            } else {
                StreamPart::Text(text.into())
            }),
        }
    }

    pub fn snapshot(&self) -> Vec<ContentBlock> {
        if !self.blocks.is_empty() {
            return anthropic_blocks(&Value::Array(self.blocks.values().cloned().collect()));
        }
        self.order
            .iter()
            .filter_map(|part| match part {
                StreamPart::Text(text) => Some(ContentBlock::Text(text.clone())),
                StreamPart::Thinking(text) => Some(ContentBlock::Thinking(text.clone())),
                StreamPart::Tool(index) => self.calls.get(index).map(|call| {
                    ContentBlock::ToolCall(crate::tools::ToolCall {
                        id: call["id"].as_str().unwrap_or("").into(),
                        name: call["function"]["name"].as_str().unwrap_or("").into(),
                        arguments: serde_json::from_str(
                            call["function"]["arguments"].as_str().unwrap_or("{}"),
                        )
                        .unwrap_or(Value::Null),
                    })
                }),
            })
            .collect()
    }

    fn flush(&mut self, elapsed_ms: u64) -> Result<(), String> {
        if self.data.is_empty() {
            return Ok(());
        }
        let data = std::mem::take(&mut self.data).join("\n");
        if data == "[DONE]" {
            self.finished = true;
            self.ended = true;
            return Ok(());
        }
        let event: Value =
            serde_json::from_str(&data).map_err(|_| "Provider 返回了无效的流事件")?;
        if event.get("error").is_some() || event["type"] == "error" {
            return Err("Provider 返回流错误，回复尚未完成".into());
        }
        self.metrics.usage(&event["usage"]);
        let index = event["index"].as_u64().unwrap_or(0) as usize;
        match event["type"].as_str() {
            Some("content_block_start") => {
                self.blocks.insert(index, event["content_block"].clone());
            }
            Some("message_start") => self.metrics.usage(&event["message"]["usage"]),
            Some("message_delta") => self.metrics.usage(&event["usage"]),
            Some("message_stop") => {
                self.finished = true;
                self.ended = true;
            }
            Some("content_block_delta") => {
                if let Some(partial) = event["delta"]["partial_json"].as_str() {
                    self.inputs.entry(index).or_default().push_str(partial);
                }
                if let Some(text) = event["delta"]["text"]
                    .as_str()
                    .filter(|text| !text.is_empty())
                {
                    self.metrics.output(elapsed_ms);
                    self.text.push_str(text);
                    let block = self
                        .blocks
                        .entry(index)
                        .or_insert_with(|| serde_json::json!({"type":"text","text":""}));
                    let previous = block["text"].as_str().unwrap_or("").to_owned();
                    block["text"] = (previous + text).into();
                } else if event["delta"]["thinking"]
                    .as_str()
                    .is_some_and(|text| !text.is_empty())
                {
                    self.metrics.output(elapsed_ms);
                    let text = event["delta"]["thinking"].as_str().unwrap();
                    let block = self
                        .blocks
                        .entry(index)
                        .or_insert_with(|| serde_json::json!({"type":"thinking","thinking":""}));
                    let previous = block["thinking"].as_str().unwrap_or("").to_owned();
                    block["thinking"] = (previous + text).into();
                }
                if let Some(signature) = event["delta"]["signature"].as_str() {
                    let block = self
                        .blocks
                        .entry(index)
                        .or_insert_with(|| serde_json::json!({"type":"thinking","thinking":""}));
                    let previous = block["signature"].as_str().unwrap_or("").to_owned();
                    block["signature"] = (previous + signature).into();
                }
            }
            _ => {}
        }
        if let Some(choice) = event["choices"]
            .as_array()
            .and_then(|choices| choices.first())
        {
            if let Some(calls) = choice["delta"]["tool_calls"].as_array() {
                for call in calls {
                    let index = call["index"].as_u64().unwrap_or(0) as usize;
                    if !self.calls.contains_key(&index) {
                        self.order.push(StreamPart::Tool(index));
                    }
                    let target = self.calls.entry(index).or_insert_with(|| serde_json::json!({"id":"","type":"function","function":{"name":"","arguments":""}}));
                    if let Some(id) = call["id"].as_str() {
                        target["id"] = id.into();
                    }
                    for field in ["name", "arguments"] {
                        if let Some(part) = call["function"][field].as_str() {
                            let previous =
                                target["function"][field].as_str().unwrap_or("").to_owned();
                            target["function"][field] = (previous + part).into();
                        }
                    }
                }
            }
            if let Some(reasoning) = choice["delta"]["reasoning_content"].as_str() {
                self.reasoning.push_str(reasoning);
                self.append_part(reasoning, true);
            }
            if let Some(text) = choice["delta"]["content"]
                .as_str()
                .filter(|text| !text.is_empty())
            {
                self.metrics.output(elapsed_ms);
                self.text.push_str(text);
                self.append_part(text, false);
            } else if choice["delta"]["reasoning_content"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
            {
                self.metrics.output(elapsed_ms);
            }
            if choice["finish_reason"].is_string() {
                self.finished = true;
            }
        }
        Ok(())
    }

    pub fn finish(mut self, elapsed_ms: u64) -> Result<Completion, String> {
        if !self.pending.is_empty() {
            self.push(b"\n", elapsed_ms)?;
        }
        self.flush(elapsed_ms)?;
        if !self.finished {
            return Err("Provider 响应流中断，回复尚未完成".into());
        }
        for (index, input) in std::mem::take(&mut self.inputs) {
            self.blocks.entry(index).or_insert(Value::Null)["input"] =
                serde_json::from_str(&input).map_err(|_| "工具参数流不是有效 JSON")?;
        }
        let blocks = self.snapshot();
        let raw_content = Value::Array(self.blocks.into_values().collect());
        let calls = if self.calls.is_empty() {
            crate::tools::parse_calls(&raw_content, true)?
        } else {
            crate::tools::parse_calls(
                &serde_json::json!({"tool_calls":self.calls.into_values().collect::<Vec<_>>()}),
                false,
            )?
        };
        if self.text.trim().is_empty() && calls.is_empty() {
            return Err("Provider 未返回文本回复".into());
        }
        Ok(Completion {
            blocks,
            text: self.text,
            stats: self.metrics.finish(elapsed_ms),
            tool_calls: calls,
            raw_content,
            reasoning: self.reasoning,
            tool_records: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_occupancy_includes_anthropic_cache_and_waits_for_measured_counts() {
        let mut metrics = Metrics::default();
        metrics.usage(&serde_json::json!({"input_tokens":100,"cache_read_input_tokens":500,"cache_creation_input_tokens":50,"output_tokens":0}));
        assert_eq!(metrics.context_tokens(), Some(650));
        metrics.usage(&serde_json::json!({"output_tokens":40}));
        metrics.set_context_window(Some(1000));
        let stats = metrics.finish(10);
        assert_eq!(stats.context_tokens, Some(690));
        assert_eq!(stats.context_window, Some(1000));
        let mut unknown = Metrics::default();
        unknown.usage(&serde_json::json!({"output_tokens":10}));
        assert_eq!(unknown.context_tokens(), None);
        let mut openai = Metrics::default();
        openai.usage(&serde_json::json!({"prompt_tokens":900,"completion_tokens":50,"prompt_tokens_details":{"cached_tokens":800}}));
        assert_eq!(openai.context_tokens(), Some(950));
    }

    #[test]
    fn anthropic_split_utf8_and_usage_produce_real_token_speed() {
        let mut stream = StreamCompletion::default();
        let first = "event: message_start\r\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":0}}}\r\n\r\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"你好\"}}\n\n";
        for byte in first.bytes() {
            stream.push(&[byte], 100).unwrap();
        }
        stream.push(b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"!\"}}\n\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":20}}\n\ndata: {\"type\":\"message_stop\"}\n\n", 600).unwrap();
        let completion = stream.finish(650).unwrap();
        assert_eq!(completion.text, "你好!");
        assert_eq!(completion.stats.output_tokens, Some(20));
        assert_eq!(completion.stats.input_tokens, Some(12));
        assert_eq!(completion.stats.first_token_ms, Some(100));
        assert_eq!(completion.stats.generation_ms, Some(500));
        assert_eq!(completion.stats.output_tps(), Some(40.));
    }

    #[test]
    fn streamed_openai_content_keeps_reasoning_text_and_call_order() {
        let mut stream = StreamCompletion::default();
        for delta in [
            serde_json::json!({"reasoning_content":"plan"}),
            serde_json::json!({"content":"before"}),
            serde_json::json!({"tool_calls":[{"index":0,"id":"call-1","function":{"name":"read_file","arguments":"{\"path\":\"README.md\"}"}}]}),
            serde_json::json!({"content":"after"}),
        ] {
            stream
                .push(
                    format!(
                        "data: {}\n\n",
                        serde_json::json!({"choices":[{"delta":delta}]})
                    )
                    .as_bytes(),
                    100,
                )
                .unwrap();
        }
        stream.push(b"data: [DONE]\n\n", 200).unwrap();
        let reply = stream.finish(200).unwrap();
        assert!(
            matches!(&reply.blocks[..], [ContentBlock::Thinking(plan), ContentBlock::Text(before), ContentBlock::ToolCall(call), ContentBlock::Text(after)] if plan == "plan" && before == "before" && call.arguments["path"] == "README.md" && after == "after")
        );
    }

    #[test]
    fn openai_usage_and_missing_usage_are_never_estimated_from_text() {
        let mut stream = StreamCompletion::default();
        stream.push(b"data: {\"choices\":[{\"delta\":{\"content\":\"OK\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"completion_tokens\":2}}\n\ndata: [DONE]\n\n", 500).unwrap();
        let completion = stream.finish(500).unwrap();
        assert_eq!(completion.stats.output_tps(), Some(4.));
        assert_eq!(completion.stats.generation_ms, None);
        assert!(
            GenerationStats {
                elapsed_ms: 500,
                ..Default::default()
            }
            .output_tps()
            .is_none()
        );
        assert!(StreamCompletion::default().finish(500).is_err());
    }

    #[test]
    fn streamed_anthropic_tool_arguments_and_thinking_are_preserved() {
        let mut stream = StreamCompletion::default();
        for event in [
            serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":"sig"}}),
            serde_json::json!({"type":"content_block_delta","index":0,"delta":{"thinking":"plan"}}),
            serde_json::json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call-1","name":"read_file","input":{}}}),
            serde_json::json!({"type":"content_block_delta","index":1,"delta":{"partial_json":"{\"path\":"}}),
            serde_json::json!({"type":"content_block_delta","index":1,"delta":{"partial_json":"\"README.md\"}"}}),
            serde_json::json!({"type":"message_stop"}),
        ] {
            stream
                .push(format!("data: {event}\n\n").as_bytes(), 100)
                .unwrap();
        }
        let reply = stream.finish(200).unwrap();
        assert_eq!(reply.tool_calls[0].name, "read_file");
        assert_eq!(reply.tool_calls[0].arguments["path"], "README.md");
        assert_eq!(reply.raw_content[0]["thinking"], "plan");
        assert_eq!(reply.raw_content[0]["signature"], "sig");
        assert!(
            matches!(&reply.blocks[..], [ContentBlock::Thinking(text), ContentBlock::ToolCall(call)] if text == "plan" && call.arguments["path"] == "README.md")
        );
    }
}
