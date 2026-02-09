//! Server-Sent Events (SSE) parser.

/// A parsed SSE event.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
    pub id: Option<String>,
    pub retry: Option<u64>,
}

impl SseEvent {
    fn is_empty(&self) -> bool {
        self.event.is_none() && self.data.is_empty() && self.id.is_none() && self.retry.is_none()
    }
}

/// Incremental SSE parser that accepts chunks and yields events.
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: String,
    current: SseEvent,
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            current: SseEvent::default(),
        }
    }

    /// Feed a text chunk and return any completed events.
    pub fn push(&mut self, chunk: &str) -> Vec<SseEvent> {
        self.buffer.push_str(chunk);
        let mut events = Vec::new();

        while let Some(pos) = self.find_line_end() {
            let line = self.buffer[..pos].to_string();
            let next = if self.buffer[pos..].starts_with("\r\n") { 2 } else { 1 };
            self.buffer.drain(..pos + next);
            let line = line.trim_end_matches(['\r', '\n']);

            if line.is_empty() {
                if !self.current.is_empty() {
                    events.push(std::mem::replace(&mut self.current, SseEvent::default()));
                }
                continue;
            }

            if line.starts_with(':') {
                continue;
            }

            let mut parts = line.splitn(2, ':');
            let field = parts.next().unwrap_or("");
            let value = parts.next().unwrap_or("");
            let value = value.strip_prefix(' ').unwrap_or(value);

            match field {
                "event" => self.current.event = Some(value.to_string()),
                "data" => {
                    if !self.current.data.is_empty() {
                        self.current.data.push('\n');
                    }
                    self.current.data.push_str(value);
                }
                "id" => self.current.id = Some(value.to_string()),
                "retry" => {
                    if let Ok(parsed) = value.parse::<u64>() {
                        self.current.retry = Some(parsed);
                    }
                }
                _ => {}
            }
        }

        events
    }

    fn find_line_end(&self) -> Option<usize> {
        self.buffer.find('\n')
    }

    /// Flush any remaining buffered event when the stream ends.
    pub fn finish(mut self) -> Option<SseEvent> {
        if self.buffer.ends_with('\r') || self.buffer.ends_with('\n') {
            self.buffer = self.buffer.trim_end_matches(['\r', '\n']).to_string();
        }
        if !self.buffer.is_empty() {
            self.push("");
        }
        if self.current.is_empty() {
            None
        } else {
            Some(self.current)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiline_data() {
        let mut parser = SseParser::new();
        let events = parser.push("data: hello\ndata: world\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "hello\nworld");
    }

    #[test]
    fn ignores_comments_and_handles_event() {
        let mut parser = SseParser::new();
        let events = parser.push(": ping\nevent: message\ndata: hi\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.as_deref(), Some("message"));
        assert_eq!(events[0].data, "hi");
    }

    #[test]
    fn handles_retry_and_id() {
        let mut parser = SseParser::new();
        let events = parser.push("id: 42\nretry: 1500\ndata: ok\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id.as_deref(), Some("42"));
        assert_eq!(events[0].retry, Some(1500));
    }
}
