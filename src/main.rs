use openrouter_rs::{
    Content, OpenRouterClient,
    api::chat::{ChatCompletionRequest, Message},
    types::{
        ResponseFormat, Role, ToolCall,
        typed_tool::{TypedTool, TypedToolParams},
    },
};
use rsbash::rashf;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    env,
    error::Error,
    fmt::format,
    io::{self, Read, Seek, SeekFrom, Write},
    process::exit,
    result,
    time::Duration,
};
use tavily::Tavily;
use tokio::fs::read_link;

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
            ▒▒▒▒▒▒▒▒    ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒    ▒▒▒▒▒▒▒▒
              ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒
            ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒
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
        "read_file_tool"
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
    dotenvy::dotenv().ok();

    let max_agent_steps = env::var("MAX_AGENT_STEPS")
        .unwrap_or("5".to_string())
        .parse::<u32>()?;

    let tavily_api_key = env::var("TAVILY_API_KEY").expect("TAVILY_API_KEY must be set");
    let api_key = env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY must be set");
    let client = OpenRouterClient::builder().api_key(api_key).build()?;
    let modelname = env::var("OPENROUTER_MAIN_MODEL")
        .unwrap_or_else(|_| "~moonshotai/kimi-latest".to_string())
        .to_string();
    let mut messages = vec![Message::new(
        Role::System,
        "You are an AI agent given tools to be able to help people",
    )];

    println!("{}", generate_landing_page());

    loop {
        println!("\nEnter your prompt: \n");
        let mut prompt = String::new();

        io::stdin()
            .read_line(&mut prompt)
            .expect("Failed to capture prompt: \n");

        if prompt.clone().trim() == "/exit" {
            println!("Crust agent quitting.....\n");
            break;
        }

        messages.push(Message::new(Role::User, prompt));

        for _ in 0..max_agent_steps {
            let request = openrouter_rs::api::chat::ChatCompletionRequest::builder()
                .model(&modelname)
                .messages(messages.clone())
                .typed_tool::<WebSearchParams>()
                .typed_tool::<BashParams>()
                .typed_tool::<ReadFileParams>()
                .typed_tool::<WriteFileParams>()
                .typed_tool::<EditFileParams>()
                .temperature(0.2f64)
                .build()?;

            let response = client.chat().create(&request).await?;

            let Some(choice) = response.choices.first() else {
                println!("\nCrust Agent: No Response quitting.........");
                break;
            };

            if let Some(tool_calls) = choice.tool_calls() {
                messages.push(Message::assistant_with_tool_calls(
                    choice.content().unwrap_or(""),
                    tool_calls.to_vec(),
                ));

                for tool_call in tool_calls {
                    let tool_result =
                        execute_tool_call(tool_call, tavily_api_key.to_string()).await?;
                    println!("tool call:  {} -> {}", tool_call.name(), tool_result);
                    messages.push(Message::tool_response_named(
                        tool_call.id(),
                        tool_call.name(),
                        tool_result,
                    ));
                }

                continue;
            } else {
                println!(
                    "{}\nCrust Agent:{}\n{}",
                    ("=").repeat(50),
                    ("=").repeat(50),
                    choice.content().unwrap_or("")
                );
                messages.push(Message::new(
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
        let cursor_start = params.offset.clone();
        let filename = params.filename.clone();
        let end = params.limit.clone();
        let file = std::fs::File::open(params.filename)?;
        let mut reader = std::io::BufReader::new(file);
        reader.seek(SeekFrom::Start(params.offset))?;

        let mut buffer = [0; 1_048_576];

        reader.read(&mut buffer)?;

        let readfileresults = json!(
            {
                "filename" : filename,
                "offset" : cursor_start,
                "end"    : end,
                "content": buffer[..end],
            }
        );

        return Ok(serde_json::to_string_pretty(&readfileresults)?);
    }

    if tc.is_tool::<WriteFileParams>() {
        let params = tc.parse_params::<WriteFileParams>()?;
        let filename = params.filename;
        let content = params.content;
        let file = std::fs::File::open(&filename)?;
        let mut writer = std::io::BufWriter::new(file);
        writer.write(content.as_bytes())?;

        let writerresults = json!({
            "filename":filename
        });
        return Ok(serde_json::to_string_pretty(&writerresults)?);
    }

    if tc.is_tool::<EditFileParams>() {
        let params = tc.parse_params::<EditFileParams>()?;
        return Ok("edit file activated".to_string());
    }

    Ok("unhandled tool:{tc.name()}".to_string())
}
// for step in 1..=MAX_AGENT_STEPS {
//         let request = ChatCompletionRequest::builder()
//             .model(model.clone())
//             .messages(messages.clone())
//             .typed_tool::<DeploymentStatusParams>()
//             .typed_tool::<RunbookLookupParams>()
//             .tool_choice_auto()
//             .parallel_tool_calls(false)
//             .max_tokens(700)
//             .build()?;

//         let response = client.chat().create(&request).await?;
//         let Some(choice) = response.choices.first() else {
//             println!("OpenRouter returned no choices");
//             return Ok(());
//         };

//         if let Some(tool_calls) = choice.tool_calls() {
//             println!("step {step}: executing {} tool call(s)", tool_calls.len());
//             messages.push(Message::assistant_with_tool_calls(
//                 choice.content().unwrap_or(""),
//                 tool_calls.to_vec(),
//             ));

//             for tool_call in tool_calls {
//                 let tool_result = execute_tool_call(tool_call)?;
//                 println!("  {} -> {}", tool_call.name(), tool_result);
//                 messages.push(Message::tool_response_named(
//                     tool_call.id(),
//                     tool_call.name(),
//                     tool_result,
//                 ));
//             }

//             continue;
//         }

//         println!("final answer:\n");
//         println!("{}", choice.content().unwrap_or("(empty response)"));
//         return Ok(());
//     }

//     println!("agent stopped after reaching the configured step limit");
//     Ok(())
// }
