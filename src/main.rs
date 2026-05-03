use openrouter_rs::{
    OpenRouterClient,
    api::chat::{ChatCompletionRequest, Message},
    types::{
        ResponseFormat, Role, ToolCall,
        typed_tool::{TypedTool, TypedToolParams},
    },
};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use std::{env, error::Error};

//  each turn should tell us the state of the loop and the output in a clean type
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub struct OrchestratorOutput {
    status: AgentStatus,
    output: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentStatus {
    Stopped,
    Active,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenvy::dotenv().ok();

    let api_key = env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY must be set");
    let modelname =
        env::var("OPENROUTER_MAIN_MODEL").unwrap_or_else(|_| "~moonshotai/kimi-latest".to_string());
    let client = OpenRouterClient::builder().api_key(api_key).build()?;
    let schema = serde_json::to_value(schema_for!(OrchestratorOutput))?;

    let messages = vec![
        Message::new(
            Role::System,
            "You are an AI designed to be as shakesparean in your problem solving as possible;
            you will be given problem statements that need brief solution submit them in a shakesparean monologue",
        ),
        Message::new(
            Role::User,
            "To understand object oriented programming what would the role of understanding polymorphism be",
        ),
    ];

    let request = openrouter_rs::api::chat::ChatCompletionRequest::builder()
        .model(modelname)
        .messages(messages.clone())
        .response_format(ResponseFormat::json_schema(
            "orchestrator_output",
            true,
            schema,
        ))
        .temperature(0.01f64)
        .build()?;

    //    loop {
    let response = client.chat().create(&request).await?;
    let content = response.choices[0].content().unwrap_or("");
    let parsed_orchestrator_output: OrchestratorOutput = serde_json::from_str(content)?;
    println!("{}", parsed_orchestrator_output.output);
    //    }

    Ok(())
}
