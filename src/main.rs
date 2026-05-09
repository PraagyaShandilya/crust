mod session;
use openrouter_rs::{
    Content, OpenRouterClient,
    api::chat::Message,
    types::{Role, ToolCall, typed_tool::TypedTool},
};
use rsbash::rashf;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use session::SessionManager;
use std::{
    env,
    error::Error,
    io::{self, Read, Seek, SeekFrom, Write},
    time::Duration,
};
use tavily::Tavily;

fn generate_landing_page() -> String {
    r#"
        ▒▒          ▒▒                                ▒▒          ▒▒
        ▒▒          ▒▒                                ▒▒          ▒▒
      ▒▒▒▒▒▒      ▒▒▒▒▒▒                            ▒▒▒▒▒▒      ▒▒▒▒▒▒
      ▒▒▒▒▒▒▒▒  ▒▒▒▒▒▒▒▒                            ▒▒▒▒▒▒▒▒  ▒▒▒▒▒▒▒▒
      ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒                            ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒
        ▒▒▒▒▒▒▒▒▒▒▒▒▒▒        ████        ████        ▒▒▒▒▒▒▒▒▒▒▒▒▒▒
        ▒▒▒▒▒▒▒▒▒▒▒▒▒▒        ████        ████        ▒▒▒▒▒▒▒▒▒▒▒▒▒▒
          ▒▒▒▒▒▒▒▒▒▒                                    ▒▒▒▒▒▒▒▒▒▒
            ▒▒▒▒▒▒            ▒▒▒▒        ▒▒▒▒            ▒▒▒▒▒▒
            ▒▒▒▒▒▒            ▒▒▒▒        ▒▒▒▒            ▒▒▒▒▒▒
            ▒▒▒▒▒▒        ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒        ▒▒▒▒▒▒
            ▒▒▒▒▒▒▒▒    ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒    ▒▒▒▒▒▒▒▒
              ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒
            ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒
          ▒▒▒▒    ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒    ▒▒▒▒
        ▒▒▒▒        ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒        ▒▒▒▒
        ▒▒▒▒        ▒▒▒▒    ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒    ▒▒▒▒        ▒▒▒▒
        ▒▒▒▒      ▒▒▒▒                                ▒▒▒▒      ▒▒▒▒
          ▒▒      ▒▒▒▒                                ▒▒▒▒      ▒▒
          ▒▒      ▒▒▒▒                                ▒▒▒▒      ▒▒
          ░░        ▒▒                                ▒▒░░
                    ▒▒                                ▒▒
                    WELCOME TO THE CRUST CODING ASSISTANT

    "#
    .to_string()
}

// tool setup for web search
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
struct WebSearchParams {
    query: String,
    max_results: usize,
}

impl TypedTool for WebSearchParams {
    fn name() -> &'static str {
        "web_search_tool"
    }

    fn description() -> &'static str {
        "Look up the web for a query using search"
    }
}

// tool setup for bash commands
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
struct BashParams {
    timeout: u32,
    command: String,
}

impl TypedTool for BashParams {
    fn name() -> &'static str {
        "bash_calling_tool"
    }

    fn description() -> &'static str {
        "Run a predefined bash command in the shell in of the workspace"
    }
}

// tool setup for reading files
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
struct ReadFileParams {
    filename: String,
    offset: u64,
    limit: usize,
}

impl TypedTool for ReadFileParams {
    fn name() -> &'static str {
        "read_file_tool"
    }
    fn description() -> &'static str {
        "Read a file using its file name, starting from the offest value and ending at the limit value, and get a string containing its contents"
    }
}

// tool setup for writing files
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
struct WriteFileParams {
    filename: String,
    content: String,
}

impl TypedTool for WriteFileParams {
    fn name() -> &'static str {
        "write_file_tool"
    }
    fn description() -> &'static str {
        "Write a file using its file name and a content string"
    }
}

// tool setup for editing files
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
struct EditFileParams {
    filename: String,
    oldcontent: String,
    newcontent: String,
}

impl TypedTool for EditFileParams {
    fn name() -> &'static str {
        "edit_file_tool"
    }
    fn description() -> &'static str {
        "Edit a file using its file finding and editing an oldcontent and replacing it with newcontent"
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Load .env from the current directory, falling back to the project root.
    dotenvy::dotenv().ok();
    dotenvy::from_filename(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")).ok();

    let max_agent_steps = env::var("MAX_AGENT_STEPS")
        .unwrap_or("5".to_string())
        .parse::<u32>()?;

    let tavily_api_key = env::var("TAVILY_API_KEY").expect("TAVILY_API_KEY must be set");

    let api_key = env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY must be set");

    let client = OpenRouterClient::builder().api_key(api_key).build()?;

    let modelname = env::var("OPENROUTER_MAIN_MODEL")
        .unwrap_or_else(|_| "moonshotai/kimi-latest".to_string())
        .to_string();

    println!("Using OpenRouter model: {modelname}");

    // Initialize messages with system prompt
    let system_message = r#"You are an AI agent given tools to be able to help people you can use the
        bash tool to run unix commands in the shell, the read write and edit tools
        respectively for editing files that you know the filenames of and the web search
        tool to interface with the web."#;

    // Initialize Session Manager
    let mut session_manager = SessionManager::new();
    let default_session = session_manager.create_session("Default".to_string(), system_message);
    println!("Created session: {}", default_session.name);

    println!("{}", generate_landing_page());

    loop {
        println!("\nEnter your prompt: \n");
        let mut prompt = String::new();

        io::stdin()
            .read_line(&mut prompt)
            .expect("Failed to capture prompt: \n");

        let prompt_trimmed = prompt.trim();

        // Session Management Commands
        if prompt_trimmed.starts_with("/new ") {
            let session_name = prompt_trimmed[5..].trim().to_string();
            if session_name.is_empty() {
                println!("Usage: /new <session_name>");
                continue;
            }
            let new_session = session_manager.create_session(session_name, system_message);
            println!("Created new session: {}", new_session.name);
            continue;
        }

        if prompt_trimmed == "/list" {
            println!("\n--- Sessions ---");
            for session in session_manager.list_sessions() {
                let current_marker = if session_manager
                    .get_current_session()
                    .map_or(false, |s| s.id == session.id)
                {
                    " (current)"
                } else {
                    ""
                };
                println!(
                    "ID: {} | Name: {} | Messages: {}{}",
                    session.id,
                    session.name,
                    session.messages.len(),
                    current_marker
                );
            }
            println!("----------------");
            continue;
        }

        if prompt_trimmed.starts_with("/switch ") {
            let session_id = prompt_trimmed[8..].trim();
            match session_manager.switch_session(session_id) {
                Ok(session) => println!("Switched to session: {}", session.name),
                Err(e) => println!("Error: {}", e),
            }
            continue;
        }

        if prompt_trimmed == "/exit" {
            println!("Crust agent quitting.....\n");
            break;
        }

        // Add user message to session
        let user_message = Message {
            role: Role::User,
            content: Content::Text(prompt),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        };
        session_manager.add_message_to_current(user_message);

        for _ in 0..max_agent_steps {
            // Get current messages for API request
            let current_messages = session_manager
                .get_current_session()
                .unwrap()
                .messages
                .clone();

            let request = openrouter_rs::api::chat::ChatCompletionRequest::builder()
                .model(&modelname)
                .messages(current_messages.clone()) // Use cloned messages
                .typed_tool::<WebSearchParams>()
                .typed_tool::<BashParams>()
                .typed_tool::<ReadFileParams>()
                .typed_tool::<WriteFileParams>()
                .typed_tool::<EditFileParams>()
                .temperature(0.2f64)
                .build()?;

            let response = match client.chat().create(&request).await {
                Ok(response) => response,
                Err(err) => {
                    eprintln!("OpenRouter request failed for model `{modelname}`: {err:?}");
                    return Err(Box::new(err) as Box<dyn Error>);
                }
            };

            let Some(choice) = response.choices.first() else {
                println!("\nCrust Agent: No Response quitting.........");
                break;
            };

            if let Some(details) = choice.reasoning_details() {
                for block in details {
                    if let Some(text) = block.content() {
                        println!("Thinking block [{}]:\n{}", block.reasoning_type(), text);
                    }
                }
            }

            if let Some(tool_calls) = choice.tool_calls() {
                // Add assistant message with tool calls to session
                session_manager.add_message_to_current(Message::assistant_with_tool_calls(
                    choice.content().unwrap_or(""),
                    tool_calls.to_vec(),
                ));

                for tool_call in tool_calls {
                    let tool_result =
                        execute_tool_call(tool_call, tavily_api_key.to_string()).await?;
                    println!("tool call:  {} -> {}", tool_call.name(), tool_result);

                    // Add tool response to session
                    session_manager.add_message_to_current(Message::tool_response_named(
                        tool_call.id(),
                        tool_call.name(),
                        tool_result,
                    ));
                }

                continue;
            } else {
                println!(
                    "{}Crust Agent:{}\n{}",
                    ("=").repeat(50),
                    ("=").repeat(50),
                    choice.content().unwrap_or("")
                );

                // Add final assistant message to session
                session_manager.add_message_to_current(Message::new(
                    Role::Assistant,
                    choice.content().unwrap_or(""),
                ));

                break;
            }
        }
    }

    Ok(())
}

async fn execute_tool_call(
    tc: &ToolCall,
    tavily_api_key: String,
) -> Result<String, Box<dyn Error>> {
    //Web Search tool exec

    if tc.is_tool::<WebSearchParams>() {
        let params = tc.parse_params::<WebSearchParams>()?;
        let tavily = Tavily::builder(tavily_api_key)
            .timeout(Duration::from_secs(45))
            .max_retries(5)
            .build()?;

        let response = tavily.search(params.query.clone()).await?;

        let mut title: Vec<String> = Vec::new();
        let mut content: Vec<String> = Vec::new();
        let mut url: Vec<String> = Vec::new();

        for (i, result) in response.results.into_iter().enumerate() {
            if i >= params.max_results {
                break;
            }

            title.push(result.title);
            content.push(result.content);
            url.push(result.url);
        }

        let answer = match response.answer {
            Some(answer) => answer,
            _ => "no answers".to_string(),
        };

        let follow_up_questions: Vec<String> = match response.follow_up_questions {
            Some(questions) => questions,
            _ => vec!["no follow ups".to_string()],
        };

        let websearch = json!({
            "answer":answer,
            "follow_up_questions":follow_up_questions,
            "results_titles":title,
            "results_urls":url,
            "results_contents":content,

        });

        return Ok(serde_json::to_string_pretty(&websearch)?);
    }
    // Bash Tool exec

    if tc.is_tool::<BashParams>() {
        let params = tc.parse_params::<BashParams>()?;
        let (exitcode, output, error) = rashf!("timeout {} {}", params.timeout, params.command)?;
        let bashresults = json!({
            "exitcode":exitcode,
            "output":output,
            "error":error,
        });
        return Ok(serde_json::to_string_pretty(&bashresults)?);
    }

    if tc.is_tool::<ReadFileParams>() {
        let params = tc.parse_params::<ReadFileParams>()?;
        let cursor_start = params.offset;
        let filename = params.filename.clone();
        let limit = params.limit.min(1_048_576);
        let file = std::fs::File::open(&filename)?;
        let mut reader = std::io::BufReader::new(file);
        reader.seek(SeekFrom::Start(params.offset))?;

        let mut buffer = vec![0; limit];
        let bytes_read = reader.read(&mut buffer)?;
        buffer.truncate(bytes_read);
        let content = String::from_utf8_lossy(&buffer).to_string();

        let readfileresults = json!(
            {
                "filename" : filename,
                "offset" : cursor_start,
                "bytes_read" : bytes_read,
                "content": content,
            }
        );

        return Ok(serde_json::to_string_pretty(&readfileresults)?);
    }

    if tc.is_tool::<WriteFileParams>() {
        let params = tc.parse_params::<WriteFileParams>()?;
        let filename = params.filename;
        let content = params.content;
        let file = std::fs::File::create(&filename)?;
        let mut writer = std::io::BufWriter::new(file);
        writer.write_all(content.as_bytes())?;

        let writerresults = json!({
            "filename":filename
        });
        return Ok(serde_json::to_string_pretty(&writerresults)?);
    }

    if tc.is_tool::<EditFileParams>() {
        let params = tc.parse_params::<EditFileParams>()?;

        let mut buf = std::fs::read_to_string(&params.filename)?;

        if let Some(offset) = buf.find(&params.oldcontent) {
            let end = offset + params.oldcontent.len();

            buf.replace_range(offset..end, &params.newcontent);

            std::fs::write(&params.filename, buf)?;

            let editfileresults = json!({
                "err" :"false",
                "content" : "edit file activated"
            });

            return Ok(serde_json::to_string_pretty(&editfileresults)?);
        } else {
            let editfileresults = json!({
                "err" :"true",
                "content" : "Oldcontent not found"
            });

            return Ok(serde_json::to_string_pretty(&editfileresults)?);
        }
    }

    Ok("unhandled tool:{tc.name()}".to_string())
}
