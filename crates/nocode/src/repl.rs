use nocode_core::message::{ContentBlock, Message, SystemBlock};
use nocode_core::provider::Provider;
use nocode_core::provider::types::{StreamDelta, StreamEvent};
use nocode_core::query::r#loop::{self, LoopConfig, LoopObserver};
use nocode_core::tool::ToolRegistry;
use nocode_core::tool::executor::ToolExecutor;
use std::io::{self, BufRead, Write};

/// Run the interactive REPL.
pub fn run_repl(
    provider: &dyn Provider,
    registry: &ToolRegistry,
    system: &[SystemBlock],
    model: &str,
    max_tokens: u32,
    max_turns: u32,
) {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    println!("nocode v{} — type /quit to exit", env!("CARGO_PKG_VERSION"));
    println!();

    let executor = ToolExecutor::new(registry);
    let mut messages: Vec<Message> = Vec::new();

    loop {
        print!("> ");
        let _ = stdout.flush();

        let mut input = String::new();
        if stdin.lock().read_line(&mut input).is_err() || input.is_empty() {
            break;
        }
        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        // Slash commands
        match input {
            "/quit" | "/exit" | "/q" => break,
            "/clear" => {
                messages.clear();
                println!("(conversation cleared)");
                continue;
            }
            "/help" => {
                println!("/quit   — exit");
                println!("/clear  — clear conversation");
                println!("/help   — show this help");
                continue;
            }
            _ => {}
        }

        messages.push(Message::user_text(input));

        let config = LoopConfig {
            model: model.to_string(),
            max_tokens,
            max_turns,
            system: system.to_vec(),
            tools: registry.definitions(),
            parallel_tool_execution: true,
        };

        let mut observer = ReplObserver::new();

        match r#loop::run_agentic_loop(
            provider,
            &executor,
            &config,
            messages.clone(),
            &mut observer,
        ) {
            Ok(result) => {
                // Print final newline if streaming didn't end with one
                if observer.needs_newline {
                    println!();
                }
                messages = result.messages;
            }
            Err(e) => {
                eprintln!("\nerror: {e}");
            }
        }

        println!();
    }
}

struct ReplObserver {
    needs_newline: bool,
}

impl ReplObserver {
    fn new() -> Self {
        Self {
            needs_newline: false,
        }
    }
}

impl LoopObserver for ReplObserver {
    fn on_stream_event(&mut self, event: &StreamEvent) {
        if let StreamEvent::ContentBlockDelta { delta, .. } = event {
            match delta {
                StreamDelta::TextDelta { text } => {
                    print!("{text}");
                    let _ = io::stdout().flush();
                    self.needs_newline = !text.ends_with('\n');
                }
                StreamDelta::ThinkingDelta { thinking } => {
                    // Show thinking in dim
                    print!("\x1b[2m{thinking}\x1b[0m");
                    let _ = io::stdout().flush();
                }
                _ => {}
            }
        }
    }

    fn on_tool_start(&mut self, name: &str, _id: &str) {
        if self.needs_newline {
            println!();
            self.needs_newline = false;
        }
        println!("\x1b[36m● {name}\x1b[0m");
    }

    fn on_tool_done(&mut self, name: &str, _id: &str, result: &ContentBlock) {
        if let ContentBlock::ToolResult {
            content, is_error, ..
        } = result
        {
            let prefix = if *is_error {
                "\x1b[31m✗"
            } else {
                "\x1b[32m✓"
            };
            // Show truncated result
            let display = if content.len() > 200 {
                format!("{}...", &content[..200])
            } else {
                content.clone()
            };
            println!("{prefix} {name}\x1b[0m {display}");
        }
    }
}
