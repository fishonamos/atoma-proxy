use anyhow::Result;
use atoma_client::{AtomaProxy, AtomaProxyConfig};
use clap::{command, Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmds>,
}

#[derive(Subcommand)]
enum Cmds {
    AcquireNewStackEntry {
        #[arg(short, long)]
        task_small_id: u64,
        #[arg(short, long)]
        num_compute_units: u64,
        #[arg(short, long)]
        price: u64,
    },
    SendOpenAiApiRequest {
        #[arg(short = 's', long)]
        system_prompt: Option<String>,
        #[arg(short = 'u', long)]
        user_prompt: Option<String>,
        #[arg(short = 't', long)]
        stream: Option<bool>,
        #[arg(short = 'm', long)]
        max_tokens: u64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();
    let config = AtomaProxyConfig::from_file_path("../config.toml");
    let mut proxy = AtomaProxy::new(config).await?;
    match args.command {
        Some(Cmds::AcquireNewStackEntry {
            task_small_id,
            num_compute_units,
            price,
        }) => {
            let stack_entry = proxy
                .acquire_new_stack_entry(task_small_id, num_compute_units, price)
                .await?;

            println!(
                "Stack acquired within transaction with digest: {}",
                stack_entry
            );
        }
        Some(Cmds::SendOpenAiApiRequest {
            system_prompt,
            user_prompt,
            stream,
            max_tokens,
        }) => {
            let response = proxy
                .send_openai_api_request(system_prompt, user_prompt, stream, max_tokens)
                .await?;
            println!("Response: {}", response);
        }
        _ => unreachable!(),
    }

    Ok(())
}
