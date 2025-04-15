use nym_sdk::tcp_proxy;
use nym_sdk::mixnet::Recipient;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Duration, Instant};
use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Stats {
    requests_sent: AtomicUsize,
    requests_succeeded: AtomicUsize,
    requests_failed: AtomicUsize,
    total_time_ns: AtomicUsize,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let server_address = env::args().nth(1).expect("Please provide server NYM address");
    let env_path = env::args().nth(2);
    let concurrency = env::args().nth(3).unwrap_or_else(|| "10".to_string()).parse::<usize>()?;
    let total_requests = env::args().nth(4).unwrap_or_else(|| "1000".to_string()).parse::<usize>()?;
    
    // Parse the server address
    let server_recipient = Recipient::try_from_base58_string(&server_address)?;
    
    // Create the proxy client
    let proxy_client = tcp_proxy::NymProxyClient::new(
        server_recipient,
        "127.0.0.1",
        "9050",
        30,
        env_path,
        concurrency.min(10) // Use concurrency level for client pool, up to 10
    ).await?;
    
    // Start the proxy client
    let proxy_client_clone = proxy_client.clone();
    tokio::spawn(async move {
        proxy_client_clone.run().await
    });
    
    // Give the client time to connect
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Statistics
    let stats = Arc::new(Stats {
        requests_sent: AtomicUsize::new(0),
        requests_succeeded: AtomicUsize::new(0),
        requests_failed: AtomicUsize::new(0),
        total_time_ns: AtomicUsize::new(0),
    });
    
    println!("Starting stress test with {} concurrent connections, {} total requests", 
             concurrency, total_requests);
    
    let start_time = Instant::now();
    
    // Spawn worker tasks
    let mut handles = Vec::new();
    
    for _ in 0..concurrency {
        let requests_per_worker = total_requests / concurrency;
        let stats = Arc::clone(&stats);
        
        let handle = tokio::spawn(async move {
            for _ in 0..requests_per_worker {
                stats.requests_sent.fetch_add(1, Ordering::SeqCst);
                let request_start = Instant::now();
                
                match TcpStream::connect("127.0.0.1:9050").await {
                    Ok(mut stream) => {
                        // Send a simple HEAD request for maximum throughput
                        if stream.write_all(b"HEAD").await.is_ok() {
                            let mut buffer = [0u8; 1024];
                            match stream.read(&mut buffer).await {
                                Ok(n) if n > 0 => {
                                    let response = String::from_utf8_lossy(&buffer[..n]);
                                    if response.trim() == "OK" {
                                        stats.requests_succeeded.fetch_add(1, Ordering::SeqCst);
                                        let elapsed = request_start.elapsed().as_nanos() as usize;
                                        stats.total_time_ns.fetch_add(elapsed, Ordering::SeqCst);
                                    } else {
                                        stats.requests_failed.fetch_add(1, Ordering::SeqCst);
                                    }
                                },
                                _ => {
                                    stats.requests_failed.fetch_add(1, Ordering::SeqCst);
                                }
                            }
                        } else {
                            stats.requests_failed.fetch_add(1, Ordering::SeqCst);
                        }
                    },
                    Err(_) => {
                        stats.requests_failed.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }
        });
        
        handles.push(handle);
    }
    
    // Wait for all workers to complete
    for handle in handles {
        let _ = handle.await;
    }
    
    let total_time = start_time.elapsed();
    
    // Print results
    println!("Stress test completed in {:?}", total_time);
    println!("Total requests: {}", stats.requests_sent.load(Ordering::SeqCst));
    println!("Successful: {}", stats.requests_succeeded.load(Ordering::SeqCst));
    println!("Failed: {}", stats.requests_failed.load(Ordering::SeqCst));
    
    let success_rate = (stats.requests_succeeded.load(Ordering::SeqCst) as f64 / 
                        stats.requests_sent.load(Ordering::SeqCst) as f64) * 100.0;
    println!("Success rate: {:.2}%", success_rate);
    
    if stats.requests_succeeded.load(Ordering::SeqCst) > 0 {
        let avg_time_ns = stats.total_time_ns.load(Ordering::SeqCst) / 
                          stats.requests_succeeded.load(Ordering::SeqCst);
        println!("Average response time: {:.2} ms", avg_time_ns as f64 / 1_000_000.0);
    }
    
    println!("Requests per second: {:.2}", 
             stats.requests_sent.load(Ordering::SeqCst) as f64 / total_time.as_secs_f64());
    
    // Clean shutdown
    proxy_client.disconnect().await;
    
    Ok(())
}
