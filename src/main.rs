use std::{env, error::Error};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    crust_types::load_env();

    let args: Vec<String> = env::args().skip(1).collect();
    if !args.is_empty() {
        return crust_cli::run_cli(args).await;
    }

    crust_tui::run_tui().await
}
