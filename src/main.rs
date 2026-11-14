#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let bot = Bot::from_env();
    let chat_id = teloxide::types::ChatId(std::env::var("TELEGRAM_CHAT_ID").unwrap().parse().unwrap());
    let client = Client::new();
    let coins = vec![
        "BTCUSDT", "ETHUSDT", "SOLUSDT", "XRPUSDT", "DOGEUSDT",
        "ADAUSDT", "AVAXUSDT", "LINKUSDT", "DOTUSDT", "MATICUSDT",
    ];

    let mut active_trades: Vec<ActiveTrade> = Vec::new();
    let trail_percentage = 0.02; 

    println!("🔍 Scanning & Managing Trades...");

    // Purane trades check karo (TSL logic)
    for trade in &mut active_trades {
        if let Ok(resp) = client.get(format!("https://api.binance.com/api/v3/ticker/price?symbol={}", trade.symbol)).send().await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                let current_price = json["price"].as_str().unwrap_or("0").parse::<f64>().unwrap_or(0.0);
                
                if trade.action == "LONG" {
                    if current_price > trade.highest_lowest {
                        trade.highest_lowest = current_price;
                        let new_sl = trade.highest_lowest * (1.0 - trail_percentage);
                        if new_sl > trade.stop_loss {
                            trade.stop_loss = new_sl;
                            let msg = format!("🔄 **TSL UPDATED (LONG): {}**\n\nPrice badh gaya hai! Naya Stop Loss: `{:.4}`", trade.symbol, trade.stop_loss);
                            let _ = bot.send_message(chat_id.clone(), msg).parse_mode(ParseMode::MarkdownV2).await;
                        }
                    }
                    if current_price <= trade.stop_loss {
                        let msg = format!("🚨 **EXIT LONG: {}**\n\nStop Loss Hit!\nExit Price: `{:.4}`\n\nApna Bybit trade band kar dein.", trade.symbol, current_price);
                        let _ = bot.send_message(chat_id.clone(), msg).parse_mode(ParseMode::MarkdownV2).await;
                    }
                } else if trade.action == "SHORT" {
                    if current_price < trade.highest_lowest {
                        trade.highest_lowest = current_price;
                        let new_sl = trade.highest_lowest * (1.0 + trail_percentage);
                        if new_sl < trade.stop_loss {
                            trade.stop_loss = new_sl;
                            let msg = format!("🔄 **TSL UPDATED (SHORT): {}**\n\nPrice niche gaya hai! Naya Stop Loss: `{:.4}`", trade.symbol, trade.stop_loss);
                            let _ = bot.send_message(chat_id.clone(), msg).parse_mode(ParseMode::MarkdownV2).await;
                        }
                    }
                    if current_price >= trade.stop_loss {
                        let msg = format!("🚨 **EXIT SHORT: {}**\n\nStop Loss Hit!\nExit Price: `{:.4}`\n\nApna Bybit trade band kar dein.", trade.symbol, current_price);
                        let _ = bot.send_message(chat_id.clone(), msg).parse_mode(ParseMode::MarkdownV2).await;
                    }
                }
            }
        }
    }
    
    // Naye signals check karo
    let mut tasks = vec![];
    for symbol in &coins {
        tasks.push(analyze_coin(&client, symbol));
    }
    let results = join_all(tasks).await;

    for (i, result) in results.into_iter().enumerate() {
        if let Some((signal_msg, entry_price)) = result {
            let symbol = coins[i].to_string();
            
            if !active_trades.iter().any(|t| t.symbol == symbol) {
                let is_long = signal_msg.contains("LONG");
                let action = if is_long { "LONG".to_string() } else { "SHORT".to_string() };
                let initial_sl = if is_long { entry_price * (1.0 - trail_percentage) } else { entry_price * (1.0 + trail_percentage) };

                active_trades.push(ActiveTrade {
                    symbol: symbol.clone(),
                    action: action.clone(),
                    entry_price,
                    highest_lowest: entry_price,
                    stop_loss: initial_sl,
                });

                let _ = bot.send_message(chat_id.clone(), signal_msg).parse_mode(ParseMode::MarkdownV2).await;
                println!("New Trade Added: {}", symbol);
            }
        }
    }

    println!("✅ Scan complete.");
}
