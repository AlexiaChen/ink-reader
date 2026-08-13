use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender, TryRecvError},
};

use futures_util::StreamExt;
use rig_core::{
    agent::MultiTurnStreamItem,
    client::CompletionClient,
    providers::ollama,
    streaming::{StreamedAssistantContent, StreamingPrompt},
};

const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:11434";
const DEFAULT_MODEL: &str = "qwen3.5:4b";

#[derive(Debug, Clone)]
pub struct CopilotConfig {
    pub endpoint: String,
    pub model: String,
    pub reasoning_model: String,
    api_key: Option<String>,
}

impl Default for CopilotConfig {
    fn default() -> Self {
        Self::from_overrides(None, None, None)
    }
}

impl CopilotConfig {
    pub fn from_overrides(
        endpoint: Option<String>,
        model: Option<String>,
        reasoning_model: Option<String>,
    ) -> Self {
        let endpoint = nonempty_override(endpoint)
            .or_else(|| nonempty_env("INK_READER_OLLAMA_URL"))
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
        let model = nonempty_override(model)
            .or_else(|| nonempty_env("INK_READER_COPILOT_MODEL"))
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let reasoning_model = nonempty_override(reasoning_model)
            .or_else(|| nonempty_env("INK_READER_COPILOT_REASONING_MODEL"))
            .unwrap_or_else(|| model.clone());
        let api_key =
            nonempty_env("INK_READER_OLLAMA_API_KEY").or_else(|| nonempty_env("OLLAMA_API_KEY"));

        Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            model,
            reasoning_model,
            api_key,
        }
    }

    pub fn endpoint_label(&self) -> String {
        self.endpoint
            .split_once("://")
            .map(|(scheme, rest)| {
                let authority = rest.split('/').next().unwrap_or(rest);
                let authority = authority
                    .rsplit_once('@')
                    .map_or(authority, |(_, host)| host);
                format!("{scheme}://{authority}")
            })
            .unwrap_or_else(|| self.endpoint.clone())
    }

    pub fn is_local(&self) -> bool {
        endpoint_host(&self.endpoint)
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"))
    }

    fn model_for(&self, task: &CopilotTask) -> &str {
        if matches!(task, CopilotTask::Analyze) {
            &self.reasoning_model
        } else {
            &self.model
        }
    }
}

fn endpoint_host(endpoint: &str) -> Option<&str> {
    let (_, rest) = endpoint.split_once("://")?;
    let authority = rest.split('/').next().unwrap_or(rest);
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if let Some(bracketed) = authority.strip_prefix('[') {
        return bracketed.split_once(']').map(|(host, _)| host);
    }
    Some(authority.split(':').next().unwrap_or(authority))
}

fn nonempty_override(value: Option<String>) -> Option<String> {
    value.and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_string()))
}

fn nonempty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopilotTask {
    Explain,
    Translate,
    Summarize,
    Analyze,
    Ask(String),
}

impl CopilotTask {
    fn instruction(&self) -> String {
        match self {
            Self::Explain => "Explain this excerpt in clear Chinese. Identify the thesis, difficult terms, assumptions, and how the argument progresses. Preserve formulas and technical precision. Use compact sections.".to_string(),
            Self::Translate => "Translate the excerpt into natural, accurate Simplified Chinese. Preserve equations, citations, names, and paragraph structure. Briefly annotate only terms whose literal translation would be misleading.".to_string(),
            Self::Summarize => "Summarize this excerpt in Chinese for efficient study: one-sentence thesis, key points, evidence or derivation, and one likely comprehension trap. Do not add facts absent from the excerpt.".to_string(),
            Self::Analyze => "Analyze the excerpt's mathematical or logical reasoning in Chinese. Reconstruct the derivation step by step, define symbols, test assumptions, and clearly mark any gap that cannot be resolved from this excerpt. Prefer correctness over brevity.".to_string(),
            Self::Ask(question) => format!("Answer this reader question in Chinese: {question}\nUse the excerpt as primary evidence. If outside knowledge is necessary, label it explicitly; if the excerpt is insufficient, say what is missing."),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CopilotContext {
    pub book_title: String,
    pub location: String,
    pub excerpt: String,
    pub prior_exchange: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopilotPhase {
    Menu,
    Input,
    Working,
    Answer,
    Error,
}

#[derive(Debug)]
enum StreamEvent {
    Thinking,
    Delta(String),
    Done(GenerationStats),
    Error(String),
}

#[derive(Debug, Default)]
struct GenerationStats {
    output_tokens: Option<u64>,
}

struct ActiveRequest {
    receiver: Receiver<StreamEvent>,
    cancelled: Arc<AtomicBool>,
}

pub struct CopilotState {
    pub config: CopilotConfig,
    pub phase: CopilotPhase,
    pub input: String,
    pub answer: String,
    pub status: String,
    pub error: String,
    pub scroll: u16,
    pub active_model: String,
    last_task: Option<CopilotTask>,
    last_context: Option<CopilotContext>,
    request: Option<ActiveRequest>,
}

impl CopilotState {
    pub fn new(config: CopilotConfig) -> Self {
        let active_model = config.model.clone();
        Self {
            config,
            phase: CopilotPhase::Menu,
            input: String::new(),
            answer: String::new(),
            status: String::new(),
            error: String::new(),
            scroll: 0,
            active_model,
            last_task: None,
            last_context: None,
            request: None,
        }
    }

    pub fn open(&mut self) {
        self.cancel();
        self.phase = CopilotPhase::Menu;
        self.input.clear();
        self.answer.clear();
        self.status.clear();
        self.error.clear();
        self.scroll = 0;
        self.last_task = None;
        self.last_context = None;
    }

    pub fn begin_input(&mut self) {
        self.phase = CopilotPhase::Input;
        self.input.clear();
        self.error.clear();
    }

    pub fn start(&mut self, task: CopilotTask, mut context: CopilotContext) {
        self.cancel();
        if matches!(task, CopilotTask::Ask(_))
            && !self.answer.trim().is_empty()
            && let Some(previous_task) = &self.last_task
        {
            context.prior_exchange = Some(format!(
                "Previous reader task:\n{}\n\nPrevious agent answer:\n{}",
                previous_task.instruction(),
                self.answer
            ));
        }
        self.answer.clear();
        self.error.clear();
        self.scroll = 0;
        self.status = "Connecting to Ollama…".to_string();
        self.active_model = self.config.model_for(&task).to_string();
        self.phase = CopilotPhase::Working;
        self.last_task = Some(task.clone());
        self.last_context = Some(context.clone());
        self.request = Some(spawn_request(self.config.clone(), task, context));
    }

    pub fn retry(&mut self) {
        if let (Some(task), Some(context)) = (self.last_task.clone(), self.last_context.clone()) {
            self.start(task, context);
        }
    }

    pub fn cancel(&mut self) {
        if let Some(request) = self.request.take() {
            request.cancelled.store(true, Ordering::Relaxed);
        }
    }

    pub fn is_working(&self) -> bool {
        self.phase == CopilotPhase::Working
    }

    pub fn poll(&mut self) {
        let mut finished = false;
        let Some(request) = &self.request else {
            return;
        };

        loop {
            match request.receiver.try_recv() {
                Ok(StreamEvent::Thinking) => {
                    self.status = "Reasoning…".to_string();
                }
                Ok(StreamEvent::Delta(delta)) => {
                    self.status = "Answering…".to_string();
                    self.answer.push_str(&delta);
                }
                Ok(StreamEvent::Done(stats)) => {
                    if self.answer.trim().is_empty() {
                        self.phase = CopilotPhase::Error;
                        self.error = "The model finished without a final answer.".to_string();
                    } else {
                        self.phase = CopilotPhase::Answer;
                        self.status = format_stats(&stats);
                    }
                    finished = true;
                    break;
                }
                Ok(StreamEvent::Error(error)) => {
                    self.phase = CopilotPhase::Error;
                    self.error = actionable_error(&self.config, &error);
                    finished = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.phase == CopilotPhase::Working {
                        self.phase = CopilotPhase::Error;
                        self.error = "The Copilot worker stopped unexpectedly.".to_string();
                    }
                    finished = true;
                    break;
                }
            }
        }

        if finished {
            self.request = None;
        }
    }
}

impl Drop for CopilotState {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn format_stats(stats: &GenerationStats) -> String {
    match stats.output_tokens {
        Some(tokens) if tokens > 0 => format!("Done · {tokens} output tokens"),
        _ => "Done".to_string(),
    }
}

fn actionable_error(config: &CopilotConfig, error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if config.is_local()
        && (lower.contains("connection refused")
            || lower.contains("tcp connect")
            || lower.contains("failed to connect"))
    {
        return format!(
            "Cannot reach local Ollama at {}.\n\nInstall/start Ollama, then pull the model:\n  ollama pull {}\n\nThe reader itself works without Ollama.",
            config.endpoint_label(),
            config.model
        );
    }
    if lower.contains("model") && (lower.contains("not found") || lower.contains("does not exist"))
    {
        return format!(
            "Model '{}' is not available. Run:\n  ollama pull {}\n\nOr choose another model with --copilot-model.",
            config.model, config.model
        );
    }
    if lower.contains("401") || lower.contains("unauthorized") {
        return "Ollama authentication failed. Set OLLAMA_API_KEY (or INK_READER_OLLAMA_API_KEY) for direct cloud access, or sign in through a local Ollama server.".to_string();
    }
    error.to_string()
}

fn spawn_request(
    config: CopilotConfig,
    task: CopilotTask,
    context: CopilotContext,
) -> ActiveRequest {
    let (sender, receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);

    std::thread::spawn(move || {
        if let Err(error) = run_agent(&config, &task, &context, &worker_cancelled, &sender) {
            let _ = sender.send(StreamEvent::Error(error));
        }
    });

    ActiveRequest {
        receiver,
        cancelled,
    }
}

fn run_agent(
    config: &CopilotConfig,
    task: &CopilotTask,
    context: &CopilotContext,
    cancelled: &AtomicBool,
    sender: &Sender<StreamEvent>,
) -> Result<(), String> {
    validate_endpoint(&config.endpoint)?;
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| format!("Cannot start the Copilot runtime: {error}"))?;
    runtime.block_on(stream_agent(config, task, context, cancelled, sender))
}

async fn stream_agent(
    config: &CopilotConfig,
    task: &CopilotTask,
    context: &CopilotContext,
    cancelled: &AtomicBool,
    sender: &Sender<StreamEvent>,
) -> Result<(), String> {
    let client = ollama::Client::builder()
        .api_key(config.api_key.clone().unwrap_or_default())
        .base_url(&config.endpoint)
        .build()
        .map_err(|error| format!("Cannot configure the Ollama provider: {error}"))?;
    let page_context = format!(
        "Book: {}\nLocation: {}\n\nCurrent visible excerpt:\n---\n{}\n---{}",
        context.book_title,
        context.location,
        context.excerpt,
        context
            .prior_exchange
            .as_deref()
            .map(|exchange| format!("\n\nConversation context:\n---\n{exchange}\n---"))
            .unwrap_or_default()
    );
    let system = "You are Ink Reader's reading agent. Help the reader understand the supplied excerpt. Never pretend to have seen the rest of the book. Distinguish claims supported by the excerpt from outside knowledge. Preserve mathematical notation and citation meaning. Return useful Markdown without a preamble.";
    let agent = client
        .agent(config.model_for(task))
        .name("ink-reading-agent")
        .description("A page-scoped agent for close reading, translation, and reasoning")
        .preamble(system)
        .context(&page_context)
        .temperature(if matches!(task, CopilotTask::Translate) {
            0.1
        } else {
            0.3
        })
        .max_tokens(2048)
        .default_max_turns(1)
        .additional_params(agent_params(task))
        .build();

    let mut stream = agent.stream_prompt(task.instruction()).max_turns(1).await;
    let mut saw_final = false;
    let mut saw_thinking = false;
    let mut saw_text = false;
    while let Some(item) = stream.next().await {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(());
        }
        match item.map_err(|error| format!("Reading agent failed: {error}"))? {
            MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text)) => {
                saw_text |= !text.text.is_empty();
                if !text.text.is_empty() && sender.send(StreamEvent::Delta(text.text)).is_err() {
                    return Ok(());
                }
            }
            MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::Reasoning(_)
                | StreamedAssistantContent::ReasoningDelta { .. },
            ) => {
                if !saw_thinking {
                    saw_thinking = true;
                    if sender.send(StreamEvent::Thinking).is_err() {
                        return Ok(());
                    }
                }
            }
            MultiTurnStreamItem::FinalResponse(response) => {
                saw_final = true;
                if !saw_text
                    && !response.output.is_empty()
                    && sender
                        .send(StreamEvent::Delta(response.output.clone()))
                        .is_err()
                {
                    return Ok(());
                }
                let output_tokens = response.usage.output_tokens;
                let _ = sender.send(StreamEvent::Done(GenerationStats {
                    output_tokens: (output_tokens > 0).then_some(output_tokens),
                }));
            }
            _ => {}
        }
    }

    if saw_final {
        Ok(())
    } else {
        Err("The reading agent stream ended before producing a final response.".to_string())
    }
}

fn validate_endpoint(endpoint: &str) -> Result<(), String> {
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        Ok(())
    } else {
        Err("Ollama URL must use http:// or https://".to_string())
    }
}

fn agent_params(task: &CopilotTask) -> serde_json::Value {
    serde_json::json!({
        "think": matches!(task, CopilotTask::Analyze),
        "keep_alive": "5m",
        // Bound KV-cache memory for constrained WSL2/Linux hosts.
        "num_ctx": 8192
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn defaults_to_local_private_endpoint_and_single_model() {
        let config = CopilotConfig {
            endpoint: DEFAULT_ENDPOINT.to_string(),
            model: DEFAULT_MODEL.to_string(),
            reasoning_model: DEFAULT_MODEL.to_string(),
            api_key: None,
        };

        assert!(config.is_local());
        assert_eq!(config.model_for(&CopilotTask::Explain), DEFAULT_MODEL);
        assert_eq!(config.model_for(&CopilotTask::Analyze), DEFAULT_MODEL);
    }

    #[test]
    fn routes_analysis_to_optional_reasoning_model() {
        let config = CopilotConfig {
            endpoint: DEFAULT_ENDPOINT.to_string(),
            model: "qwen3.5:4b".to_string(),
            reasoning_model: "phi4-mini-reasoning".to_string(),
            api_key: None,
        };

        assert_eq!(config.model_for(&CopilotTask::Explain), "qwen3.5:4b");
        assert_eq!(
            config.model_for(&CopilotTask::Analyze),
            "phi4-mini-reasoning"
        );
    }

    #[test]
    fn request_enables_thinking_only_for_deep_analysis() {
        assert_eq!(agent_params(&CopilotTask::Summarize)["think"], false);
        assert_eq!(agent_params(&CopilotTask::Analyze)["think"], true);
        assert_eq!(agent_params(&CopilotTask::Analyze)["num_ctx"], 8192);
    }

    #[test]
    fn malformed_endpoint_is_reported_without_panicking() {
        let error = validate_endpoint("not a URL").unwrap_err();
        assert!(error.contains("http:// or https://"));
    }

    #[test]
    fn local_detection_handles_ports_ipv6_and_does_not_show_credentials() {
        for endpoint in [
            "http://localhost:11434",
            "http://127.0.0.1:11434/api",
            "http://[::1]:11434",
        ] {
            let config = CopilotConfig {
                endpoint: endpoint.to_string(),
                model: DEFAULT_MODEL.to_string(),
                reasoning_model: DEFAULT_MODEL.to_string(),
                api_key: None,
            };
            assert!(config.is_local(), "{endpoint}");
        }

        let config = CopilotConfig {
            endpoint: "https://user:secret@example.com:443/path".to_string(),
            model: DEFAULT_MODEL.to_string(),
            reasoning_model: DEFAULT_MODEL.to_string(),
            api_key: None,
        };
        assert_eq!(config.endpoint_label(), "https://example.com:443");
        assert!(!config.is_local());
    }

    #[test]
    #[ignore = "requires permission to bind a loopback socket"]
    fn rig_agent_streams_against_an_ollama_protocol_mock() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_sender, request_receiver) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            let header_end = loop {
                let read = socket.read(&mut buffer).unwrap();
                assert!(read > 0);
                request.extend_from_slice(&buffer[..read]);
                if let Some(index) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            while request.len() < header_end + content_length {
                let read = socket.read(&mut buffer).unwrap();
                assert!(read > 0);
                request.extend_from_slice(&buffer[..read]);
            }
            request_sender
                .send(String::from_utf8(request).unwrap())
                .unwrap();

            let body = concat!(
                "{\"model\":\"mock\",\"created_at\":\"2026-08-13T00:00:00Z\",",
                "\"message\":{\"role\":\"assistant\",\"content\":\"你好\"},\"done\":false}\n",
                "{\"model\":\"mock\",\"created_at\":\"2026-08-13T00:00:01Z\",",
                "\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true,",
                "\"prompt_eval_count\":10,\"eval_count\":2}\n"
            );
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });

        let config = CopilotConfig {
            endpoint: format!("http://{address}"),
            model: "mock-reader".to_string(),
            reasoning_model: "mock-reasoner".to_string(),
            api_key: None,
        };
        let context = CopilotContext {
            book_title: "Paper".to_string(),
            location: "Section 1".to_string(),
            excerpt: "A short excerpt.".to_string(),
            prior_exchange: None,
        };
        let (event_sender, event_receiver) = mpsc::channel();

        run_agent(
            &config,
            &CopilotTask::Explain,
            &context,
            &AtomicBool::new(false),
            &event_sender,
        )
        .unwrap();
        server.join().unwrap();

        let request = request_receiver.recv().unwrap();
        assert!(request.starts_with("POST /api/chat HTTP/1.1"));
        assert!(request.contains("\"model\":\"mock-reader\""));
        assert!(request.contains("\"stream\":true"));
        assert!(request.contains("\"think\":false"));
        assert!(request.contains("\"num_ctx\":8192"));

        let events: Vec<StreamEvent> = event_receiver.try_iter().collect();
        assert!(matches!(events.first(), Some(StreamEvent::Delta(text)) if text == "你好"));
        assert!(matches!(
            events.get(1),
            Some(StreamEvent::Done(GenerationStats {
                output_tokens: Some(2)
            }))
        ));
    }
}
