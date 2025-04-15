use anyhow::{Result, Context};
use clap::Parser;
use nym_sdk::{mixnet::Recipient, tcp_proxy::NymProxyClient};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Parser)]
#[clap(name = "nymbazaar-client", about = "NymBazaar client for shopping vintage collectibles")]
struct Args {
    /// NYM mixnet address of the NymBazaar server
    #[clap(long, required = true)]
    bazaar_id: String,
    
    /// Enable verbose logging
    #[clap(long)]
    verbose: bool,
    
    /// Log file path
    #[clap(long)]
    log: Option<PathBuf>,
}

struct Client {
    verbose: bool,
    log_file: Option<PathBuf>,
    server_address: Recipient,
}

impl Client {
    fn new(args: Args) -> Result<Self> {
        let server_address = Recipient::try_from_base58_string(&args.bazaar_id)
            .context("Invalid bazaar server address")?;
        
        Ok(Self {
            verbose: args.verbose,
            log_file: args.log,
            server_address,
        })
    }
    
    fn log(&self, message: &str) {
        if self.verbose {
            println!("[LOG] {}", message);
        }
        
        if let Some(log_path) = &self.log_file {
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path) 
            {
                let _ = writeln!(file, "{}", message);
            }
        }
    }
    
    async fn connect_to_mixnet(&self, temp_dir: &str) -> Result<NymProxyClient> {
        self.log("Connecting to NYM mixnet...");
        
        let proxy_client = NymProxyClient::new(
            self.server_address,
            "127.0.0.1",
            "9050",  // Local port for SOCKS proxy
            60,      // Timeout in seconds
            None,    // Env path (None for default network)
            1,       // Client pool reserve
        ).await?;
        
        self.log("Connected to NYM mixnet");
        
        Ok(proxy_client)
    }
    
    async fn send_command(&self, stream: &mut TcpStream, command: &str) -> Result<String> {
        self.log(&format!("Sending command: {}", command.trim()));
        
        stream.write_all(command.as_bytes()).await?;
        
        let mut buffer = vec![0u8; 4096];
        let n = stream.read(&mut buffer).await?;
        
        let response = String::from_utf8_lossy(&buffer[..n]).to_string();
        self.log(&format!("Received response: {} bytes", response.len()));
        
        Ok(response)
    }
    
    async fn run_ui(&self, mut stream: TcpStream) -> Result<()> {
        // Initial connection check
        let response = self.send_command(&mut stream, "HEAD\n").await?;
        if response.trim() != "OK" {
            println!("Failed to connect to bazaar server: {}", response);
            return Ok(());
        }
        
        println!("\n🏪 Welcome to NymBazaar - Vintage Collectibles Marketplace 🏪");
        println!("Connected to server via NYM mixnet");
        
        // Main UI loop
        loop {
            println!("\n📋 Menu:");
            println!("1. List all items");
            println!("2. List by category");
            println!("3. Search items");
            println!("4. View item details");
            println!("5. Show categories");
            println!("6. Exit");
            
            print!("\nSelect an option: ");
            io::stdout().flush()?;
            
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            
            match input.trim() {
                "1" => {
                    println!("\n📦 All Items:");
                    let response = self.send_command(&mut stream, "LIST\n").await?;
                    println!("{}", response);
                },
                "2" => {
                    println!("\nFirst, let's get available categories:");
                    let cats = self.send_command(&mut stream, "CATEGORIES\n").await?;
                    println!("{}", cats);
                    
                    print!("Enter category: ");
                    io::stdout().flush()?;
                    let mut cat = String::new();
                    io::stdin().read_line(&mut cat)?;
                    
                    println!("\n📦 Items in category '{}':", cat.trim());
                    let response = self.send_command(&mut stream, &format!("LIST {}\n", cat.trim())).await?;
                    println!("{}", response);
                },
                "3" => {
                    print!("Enter search term: ");
                    io::stdout().flush()?;
                    let mut term = String::new();
                    io::stdin().read_line(&mut term)?;
                    
                    println!("\n🔍 Search results for '{}':", term.trim());
                    let response = self.send_command(&mut stream, &format!("SEARCH {}\n", term.trim())).await?;
                    println!("{}", response);
                },
                "4" => {
                    print!("Enter item ID: ");
                    io::stdout().flush()?;
                    let mut id = String::new();
                    io::stdin().read_line(&mut id)?;
                    
                    println!("\n📋 Item details:");
                    let response = self.send_command(&mut stream, &format!("GET {}\n", id.trim())).await?;
                    println!("{}", response);
                },
                "5" => {
                    println!("\n🏷️ Categories:");
                    let response = self.send_command(&mut stream, "CATEGORIES\n").await?;
                    println!("{}", response);
                },
                "6" => {
                    println!("Thank you for using NymBazaar! Goodbye.");
                    break;
                },
                _ => println!("Invalid option. Please try again."),
            }
        }
        
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    let client = Client::new(args)?;
    
    // Use a temporary directory for the client
    let temp_dir = format!("/tmp/nymbazaar-client-{}", uuid::Uuid::new_v4());
    std::fs::create_dir_all(&temp_dir)?;
    
    // Start the proxy client
    let proxy_client = client.connect_to_mixnet(&temp_dir).await?;
    
    // Run proxy client in background
    let _proxy_handle = tokio::spawn(async move {
        if let Err(e) = proxy_client.run().await {
            eprintln!("Proxy client error: {}", e);
        }
    });
    
    // Wait for proxy to start
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
    
    // Connect to local proxy socket
    let stream = match TcpStream::connect("127.0.0.1:9050").await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to connect to proxy: {}", e);
            return Ok(());
        }
    };
    
    // Run the UI
    if let Err(e) = client.run_ui(stream).await {
        eprintln!("UI error: {}", e);
    }
    
    // Clean up
    std::fs::remove_dir_all(temp_dir).ok();
    
    Ok(())
}
