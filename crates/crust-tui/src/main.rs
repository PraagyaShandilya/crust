use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    crust_types::load_env();
    crust_tui::run_tui().await
}
