use anyhow::Result;
use nym_sdk::tcp_proxy;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio::signal;

struct Item {
    id: String,
    name: String,
    category: String,
    description: String,
    price: String,
    seller: String,
}

struct BazaarServer {
    items: Arc<RwLock<HashMap<String, Item>>>,
}

impl BazaarServer {
    fn new() -> Self {
        let mut items = HashMap::new();
        
        // Sample items
        items.insert("1".to_string(), Item {
            id: "1".to_string(),
            name: "Nintendo NES".to_string(),
            category: "gaming".to_string(),
            description: "Original Nintendo Entertainment System from 1985. Good condition with controllers.".to_string(),
            price: "$150".to_string(),
            seller: "RetroGamer".to_string(),
        });
        
        items.insert("2".to_string(), Item {
            id: "2".to_string(),
            name: "Yamaha DX7".to_string(),
            category: "synthesizer".to_string(),
            description: "Classic FM synthesizer from 1983. The quintessential 80s synth sound.".to_string(),
            price: "$800".to_string(),
            seller: "SynthWave".to_string(),
        });
        
        // Add more items here...
        
        BazaarServer {
            items: Arc::new(RwLock::new(items)),
        }
    }
    
    async fn handle_command(&self, command: &str) -> String {
        let parts: Vec<&str> = command.trim().split_whitespace().collect();
        
        match parts.get(0).map(|s| s.to_uppercase()).as_deref() {
            Some("HEAD") => "OK\n".to_string(),
            
            Some("LIST") => {
                let category_filter = parts.get(1).map(|s| s.to_lowercase());
                
                let items = self.items.read().await;
                let filtered_items: Vec<&Item> = items
                    .values()
                    .filter(|item| {
                        if let Some(ref cat) = category_filter {
                            item.category.to_lowercase() == *cat
                        } else {
                            true
                        }
                    })
                    .collect();
                
                if filtered_items.is_empty() {
                    return "No items found\n".to_string();
                }
                
                let mut response = String::new();
                for item in filtered_items {
                    response.push_str(&format!("{}. {} - {}\n", item.id, item.name, item.price));
                }
                
                response
            },
            
            Some("GET") if parts.len() > 1 => {
                let id = parts[1];
                let items = self.items.read().await;
                
                if let Some(item) = items.get(id) {
                    format!(
                        "ID: {}\nName: {}\nCategory: {}\nPrice: {}\nSeller: {}\n\n{}\n",
                        item.id, item.name, item.category, item.price, item.seller, item.description
                    )
                } else {
                    format!("Item with ID {} not found\n", id)
                }
            },
            
            Some("SEARCH") if parts.len() > 1 => {
                let term = parts[1].to_lowercase();
                let items = self.items.read().await;
                
                let results: Vec<&Item> = items
                    .values()
                    .filter(|item| {
                        item.name.to_lowercase().contains(&term) ||
                        item.description.to_lowercase().contains(&term) ||
                        item.category.to_lowercase().contains(&term)
                    })
                    .collect();
                
                if results.is_empty() {
                    return "No items found matching your search\n".to_string();
                }
                
                let mut response = String::new();
                for item in results {
                    response.push_str(&format!("{}. {} - {}\n", item.id, item.name, item.price));
                }
                
                response
            },
            
            Some("CATEGORIES") => {
                let items = self.items.read().await;
                let mut categories = HashSet::new();
                
                for item in items.values() {
                    categories.insert(&item.category);
                }
                
                let mut response = String::from("Available categories:\n");
                for category in categories {
                    response.push_str(&format!("- {}\n", category));
                }
                
                response
            },
            
            _ => "Invalid command. Available commands:\nHEAD\nLIST [category]\nGET <id>\nSEARCH <term>\nCATEGORIES\n".to_string(),
        }
    }
}

async fn handle_connection(mut socket: tokio::net::TcpStream, server: Arc<BazaarServer>) {
    let mut buffer = vec![0u8; 4096];
    
    loop {
        match socket.read(&mut buffer).await {
            Ok(0) => {
                println!("Connection closed by client");
                break;
            },
            Ok(n) => {
                let request = String::from_utf8_lossy(&buffer[..n]);
                println!("Command: {}", request.trim());
                
                let response = server.handle_command(&request).await;
                
                if let Err(e) = socket.write_all(response.as_bytes()).await {
                    eprintln!("Write error: {}", e);
                    break;
                }
            },
            Err(e) => {
                eprintln!("Read error: {}", e);
                break;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let config_dir = std::env::args().nth(1).expect("Config directory not provided");
    let env_path = std::env::args().nth(2);
    
    let tcp_addr = "127.0.0.1:8000";
    
    // Create NymProxyServer
    let mut proxy_server = tcp_proxy::NymProxyServer::new(tcp_addr, &config_dir, env_path).await?;
    let server_address = proxy_server.nym_address();
    
    println!("NymBazaar server starting on NYM mixnet");
    println!("Server address: {}", server_address);
    
    // Run proxy server
    let proxy_task = tokio::spawn(async move {
        if let Err(e) = proxy_server.run_with_shutdown().await {
            eprintln!("Proxy error: {}", e);
        }
    });
    
    // Create bazaar server
    let bazaar_server = Arc::new(BazaarServer::new());
    println!("Marketplace initialized with sample items");
    
    // Create TCP server
    let listener = TcpListener::bind(tcp_addr).await?;
    
    // Handle shutdown
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let shutdown_clone = shutdown.clone();
    
    tokio::spawn(async move {
        signal::ctrl_c().await.expect("Failed to listen for ctrl+c");
        shutdown_clone.notify_one();
    });
    
    // Accept connections
    loop {
        tokio::select! {
            Ok((socket, _)) = listener.accept() => {
                let server_ref = bazaar_server.clone();
                tokio::spawn(async move {
                    handle_connection(socket, server_ref).await;
                });
            },
            _ = shutdown.notified() => {
                println!("Server shutting down...");
                break;
            }
        }
    }
    
    proxy_task.abort();
    println!("Server shutdown complete");
    Ok(())
}
