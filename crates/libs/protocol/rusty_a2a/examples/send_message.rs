//! A minimal A2A client: discovers an agent via its Agent Card, then sends
//! it a message and prints the result (streaming, if the agent supports
//! it).
//!
//! ```sh
//! cargo run --example echo_server --features server &
//! cargo run --example send_message --features client -- "hello there"
//! ```

use futures_util::StreamExt;
use rusty_a2a::client::A2aClient;
use rusty_a2a::types::Message;

#[tokio::main]
async fn main() {
    let text = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "hello there".to_string());
    let base_url = std::env::var("A2A_AGENT_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());

    let (client, card) = A2aClient::discover(&base_url)
        .await
        .expect("failed to discover agent");
    println!("Connected to \"{}\": {}", card.name, card.description);

    if card.capabilities.streaming == Some(true) {
        println!("Streaming response:");
        let mut stream = client
            .send_streaming_message(Message::user_text(&text), None)
            .await
            .expect("failed to start streaming message");
        while let Some(event) = stream.next().await {
            match event {
                Ok(event) => println!("  {event:?}"),
                Err(e) => {
                    eprintln!("stream error: {e}");
                    break;
                }
            }
        }
    } else {
        let result = client
            .send_message(Message::user_text(&text), None)
            .await
            .expect("failed to send message");
        println!("{result:?}");
    }
}
