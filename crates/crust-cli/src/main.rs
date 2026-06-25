use std::{env, error::Error};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    crust_types::load_env();
    crust_cli::run_cli(env::args().skip(1).collect()).await
}
