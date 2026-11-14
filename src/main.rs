use reqwest::Client;
use ta::indicators::{RelativeStrengthIndex, MovingAverageConvergenceDivergence, ExponentialMovingAverage};
use ta::Next;
use teloxide::{prelude::*, types::ParseMode};
use futures::future::join_all;
use serde_json::Value;

#[derive(Debug, Clone)]
struct Candle {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

#[derive(Debug, Clone)]
struct ActiveTrade {
    symbol: String,
    action: String,       
    entry_price: f64,
    highest_lowest: f64,  
    stop_loss: f64,       
}

async fn fetch_klines(client: &Client, symbol: &str, interval: &str) -> Vec<Candle> {
    let url = format!("https://api.binance.com/api/v3/klines?symbol={}&interval={}&limit=100", symbol, interval);
    if let Ok(resp) = client.get(&url).send().await {
        if let Ok(data) = resp.json::<Vec<Vec<Value>>>().await {
            return data.iter().map(|k| Candle {
                open: k[1].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                high: k[2].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                low: k[3].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                close: k[4].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                volume: k[5].as_str().unwrap_or("0").parse().unwrap_or(0.0),
            }).collect();
        }
    }
    vec![]
}

fn detect_pattern(candles: &[Candle]) -> &'static str {
    if candles.len() < 2 { return "None"; }
    let curr = &candles[candles.len() - 1];
    let prev = &candles[candles.len() - 2];

    if prev.close < prev.open && curr.close > curr.open && curr.close > prev.open && curr.open < prev.close { return "Bullish Engulfing"; }
    if prev.close > prev.open && curr.close < curr.open && curr.close < prev.open && curr.open > prev.close { return "Bearish Engulfing"; }
    
    let body = (curr.close - curr.open).abs();
    let lower_wick = curr.open.min(curr.close) - curr.low;
    let upper_wick = curr.high - curr.open.max(curr.close);
    if lower_wick > body * 2.0 && upper_wick < body * 0.5 { return "Bullish Pinbar"; }
    if upper_wick > body * 2.0 && lower_wick < body * 0.5 { return "Bearish Pinbar"; }
    
    "None"
}

fn detect_divergence(candles: &[Candle], rsi_values: &[f64]) -> &'static str {
    if candles.len() < 30 || rsi_values.len() < 30 { return "None"; }
    let len = candles.len();
    let current_price = candles[len - 1].close;
    let current_rsi = rsi_values[len - 1];
    
    let past_price_low = candles[len - 15..len - 2].iter().map(|c| c.low).fold(f64::MAX, f64::min);
    let past_rsi_low = rsi_values[len - 15..len - 2].iter().fold(f64::MAX, |a, &b| a.min(b));
    let past_price_high = candles[len - 15..len - 2].iter().map(|c| c.high).fold(f64::MIN, f64::max);
    let past_rsi_high = rsi_values[len - 15..len - 2].iter().fold(f64::MIN, |a, &b| a.max(b));

    if current_price < past_price_low && current_rsi > past_rsi_low && current_rsi < 45.0 { return "Bullish Divergence"; }
    if current_price > past_price_high && current_rsi < past_rsi_high && current_rsi > 55.0 { return "Bearish Divergence"; }
    "None"
}

fn detect_sr_breakout(candles: &[Candle]) -> &'static str {
    if candles.len() < 25 { return "None"; }
    let len = candles.len();
    let current_close = candles[len - 1].close;
    let resistance = candles[len - 21..len - 1].iter().map(|c| c.high).fold(f64::MIN, f64::max);
    let support = candles[len - 21..len - 1].iter().map(|c| c.low).fold(f64::MAX, f64::min);

    if current_close > resistance { return "Bullish Breakout"; }
    else if current_close < support { return "Bearish Breakdown"; }
    "None"
}

async fn analyze_coin(client: &Client, symbol: &str) -> Option<(String, f64)> {
    let (candles_1h, candles_15m) = tokio::join!(
        fetch_klines(client, symbol, "1h"),
        fetch_klines(client, symbol, "15m")
    );

    if candles_1h.len() < 50 || candles_15m.len() < 30 { return None; }

    let mut ema_50_1h = ExponentialMovingAverage::new(50).unwrap();
    let mut trend_ema = 0.0;
    for c in &candles_1h { trend_ema = ema_50_1h.next(c.close); }
    let current_price = candles_15m.last().unwrap().close;
    let is_uptrend = candles_1h.last().unwrap().close > trend_ema;
    let is_downtrend = candles_1h.last().unwrap().close < trend_ema;

    let mut rsi_15m = RelativeStrengthIndex::new(14).unwrap();
    let mut macd_15m = MovingAverageConvergenceDivergence::new(12, 26, 9).unwrap();
    
    let mut rsi_values = Vec::new();
    let mut latest_macd_hist = 0.0;
    let mut vol_sma = 0.0;

    for c in &candles_15m {
        rsi_values.push(rsi_15m.next(c.close));
        latest_macd_hist = macd_15m.next(c.close).histogram;
        vol_sma += c.volume;
    }
    
    vol_sma /= candles_15m.len() as f64;
    let high_volume = candles_15m.last().unwrap().volume > vol_sma * 1.5; 
    let pattern = detect_pattern(&candles_15m);
    let divergence = detect_divergence(&candles_15m, &rsi_values);
    let breakout = detect_sr_breakout(&candles_15m);
    let latest_rsi = *rsi_values.last().unwrap();

    let mut long_score = 0;
    let mut short_score = 0;

    if is_uptrend { long_score += 2; }
    if latest_rsi < 50.0 { long_score += 1; }
    if latest_macd_hist > 0.0 { long_score += 1; }
    if high_volume { long_score += 1; }
    if pattern == "Bullish Engulfing" || pattern == "Bullish Pinbar" { long_score += 1; }
    if divergence == "Bullish Divergence" { long_score += 2; }
    if breakout == "Bullish Breakout" { long_score += 2; }

    if is_downtrend { short_score += 2; }
    if latest_rsi > 50.0 { short_score += 1; }
    if latest_macd_hist < 0.0 { short_score += 1; }
    if high_volume { short_score += 1; }
    if pattern == "Bearish Engulfing" || pattern == "Bearish Pinbar" { short_score += 1; }
    if divergence == "Bearish Divergence" { short_score += 2; }
    if breakout == "Bearish Breakdown" { short_score += 2; }

    // Score threshold yahan 4 kar diya gaya hai testing ke liye
    if long_score >= 4 && long_score > short_score {
        let msg = format!(
            "🟢 **NEW LONG ENTRY: {}**\n\nEntry Price: `{}`\n🔥 Score: {}/10\n🕯️ Pattern: {}\n⚡ Divergence: {}\n🚀 S/R: {}",
            symbol, current_price, long_score, pattern, divergence, breakout
        );
        return Some((msg, current_price));
    } else if short_score >= 4 && short_score > long_score {
        let msg = format!(
            "🔴 **NEW SHORT ENTRY: {}**\n\nEntry Price: `{}`\n🔥 Score: {}/10\n🕯️ Pattern: {}\n⚡ Divergence: {}\n🚀 S/R: {}",
            symbol, current_price, short_score, pattern, divergence, breakout
        );
        return Some((msg, current_price));
    }
    None
}

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

    let active_trades: Vec<ActiveTrade> = Vec::new();

    println!("🔍 Scanning & Managing Trades...");

    // Ye line Telegram par test message bhejegi
    let _ = bot.send_message(chat_id.clone(), "🤖 Bot successfully chalu ho gaya hai!").await;

    for trade in &active_trades {
        if let Ok(resp) = client.get(format!("https://api.binance.com/api/v3/ticker/price?symbol={}", trade.symbol)).send().await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                let current_price = json["price"].as_str().unwrap_or("0").parse::<f64>().unwrap_or(0.0);
                
                if trade.action == "LONG" && current_price <= trade.stop_loss {
                    let msg = format!("🚨 **EXIT LONG: {}**\n\nStop Loss Hit!\nExit Price: `{:.4}`", trade.symbol, current_price);
                    let _ = bot.send_message(chat_id.clone(), msg).parse_mode(ParseMode::MarkdownV2).await;
                } else if trade.action == "SHORT" && current_price >= trade.stop_loss {
                    let msg = format!("🚨 **EXIT SHORT: {}**\n\nStop Loss Hit!\nExit Price: `{:.4}`", trade.symbol, current_price);
                    let _ = bot.send_message(chat_id.clone(), msg).parse_mode(ParseMode::MarkdownV2).await;
                }
            }
        }
    }
    
    let mut tasks = vec![];
    for symbol in &coins {
        tasks.push(analyze_coin(&client, symbol));
    }
    let results = join_all(tasks).await;

    for (i, result) in results.into_iter().enumerate() {
        if let Some((signal_msg, _entry_price)) = result {
            let symbol = coins[i].to_string();
            
            if !active_trades.iter().any(|t| t.symbol == symbol) {
                let _ = bot.send_message(chat_id.clone(), signal_msg).parse_mode(ParseMode::MarkdownV2).await;
                println!("New Trade Added: {}", symbol);
            }
        }
    }

    println!("✅ Scan complete.");
}
