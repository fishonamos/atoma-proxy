use anyhow::Result;
use atoma_client::{AtomaSuiClient, AtomaSuiConfig};
use clap::Parser;

#[derive(Parser)]
struct Args {
    #[arg(short, long)]
    task_small_id: u64,
    #[arg(short, long)]
    num_compute_units: u64,
    #[arg(short, long)]
    price: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = AtomaSuiConfig::from_file_path("../config.toml");
    let mut client = AtomaSuiClient::new(config).await?;
    let stack_entry = client
        .acquire_new_stack_entry(args.task_small_id, args.num_compute_units, args.price)
        .await?;

    println!(
        "Stack acquired within transaction with digest: {}",
        stack_entry
    );

    Ok(())
}
