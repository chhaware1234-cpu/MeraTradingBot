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

// 1. Crypto Data (Kraken API - 100% Unblocked on GitHub US Servers)
async fn fetch_crypto_klines(client: &Client, symbol: &str, interval: &str, _limit: u32) -> Vec<Candle> {
    let pair = match symbol {
        "BTCUSDT" => "XBTUSD",
        "ETHUSDT" => "ETHUSD",
        "SOLUSDT" => "SOLUSD",
        _ => "XBTUSD",
    };
    let kraken_interval = if interval == "1h" { "60" } else { "5" }; // 5-Minute Entry
    let url = format!("https://api.kraken.com/0/public/OHLC?pair={}&interval={}", pair, kraken_interval);

    if let Ok(resp) = client.get(&url).send().await {
        if let Ok(json) = resp.json::<Value>().await {
            if let Some(result_obj) = json["result"].as_object() {
                for (key, value) in result_obj {
                    if key != "last" {
                        if let Some(arr) = value.as_array() {
                            let mut candles = vec![];
                            for k in arr {
                                if let Some(c) = k.as_array() {
                                    candles.push(Candle {
                                        open: c[1].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                                        high: c[2].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                                        low: c[3].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                                        close: c[4].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                                        volume: c[6].as_str().unwrap_or("0").parse().unwrap_or(0.0),
                                    });
                                }
                            }
                            if candles.len() > 200 { return candles[candles.len() - 200..].to_vec(); }
                            return candles;
                        }
                    }
                }
            }
        }
    }
    vec![]
}

// 2. Stocks & Gold Data (Yahoo Finance)
async fn fetch_yahoo_klines(client: &Client, symbol: &str, interval: &str) -> Vec<Candle> {
    let range = "5d";
    let url = format!("https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&range={}", symbol, interval, range);
    let req = client.get(&url).header("User-Agent", "Mozilla/5.0").send().await;
    
    if let Ok(resp) = req {
        if let Ok(json) = resp.json::<Value>().await {
            if let Some(result) = json["chart"]["result"].as_array() {
                if let Some(data) = result.get(0) {
                    if let Some(quote) = data["indicators"]["quote"].as_array().and_then(|a| a.get(0)) {
                        let mut candles = vec![];
                        if let (Some(o), Some(h), Some(l), Some(c), Some(v)) = (
                            quote["open"].as_array(), quote["high"].as_array(), quote["low"].as_array(), quote["close"].as_array(), quote["volume"].as_array()
                        ) {
                            for i in 0..c.len() {
                                if let (Some(open), Some(high), Some(low), Some(close)) = (o[i].as_f64(), h[i].as_f64(), l[i].as_f64(), c[i].as_f64()) {
                                    let volume = v[i].as_f64().unwrap_or(0.0);
                                    candles.push(Candle { open, high, low, close, volume });
                                }
                            }
                        }
                        return candles;
                    }
                }
            }
        }
    }
    vec![]
}

// 3. Indicators & Patterns
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

fn get_scores(candles_1h: &[Candle], candles_5m: &[Candle], news_pos: i32, news_neg: i32) -> (i32, i32, String, String, String, f64, f64) {
    if candles_1h.len() < 30 || candles_5m.len() < 20 { return (0, 0, "".to_string(), "".to_string(), "".to_string(), 50.0, 0.0); }

    let mut ema_50_1h = ExponentialMovingAverage::new(50).unwrap();
    let mut trend_ema = 0.0;
    for c in candles_1h { trend_ema = ema_50_1h.next(c.close); }
    let is_uptrend = candles_1h.last().unwrap().close > trend_ema;
    let is_downtrend = candles_1h.last().unwrap().close < trend_ema;

    let mut rsi_5m = RelativeStrengthIndex::new(14).unwrap();
    let mut macd_5m = MovingAverageConvergenceDivergence::new(12, 26, 9).unwrap();
    
    let mut rsi_values = Vec::new();
    let mut latest_macd_hist = 0.0;
    let mut vol_sma = 0.0;

    for c in candles_5m {
        rsi_values.push(rsi_5m.next(c.close));
        latest_macd_hist = macd_5m.next(c.close).histogram;
        vol_sma += c.volume;
    }
    
    vol_sma /= candles_5m.len() as f64;
    let high_volume = candles_5m.last().unwrap().volume > vol_sma * 1.5; 
    let pattern = detect_pattern(candles_5m);
    let divergence = detect_divergence(candles_5m, &rsi_values);
    let breakout = detect_sr_breakout(candles_5m);
    let latest_rsi = *rsi_values.last().unwrap();

    let current_candle = candles_5m.last().unwrap();
    let candle_change_percent = ((current_candle.close - current_candle.open) / current_candle.open) * 100.0;

    let mut long_score = 0;
    let mut short_score = 0;

    if is_uptrend { long_score += 2; }
    if latest_rsi < 50.0 { long_score += 1; }
    if latest_macd_hist > 0.0 { long_score += 1; }
    if high_volume { long_score += 1; }
    if pattern == "Bullish Engulfing" || pattern == "Bullish Pinbar" { long_score += 1; }
    if divergence == "Bullish Divergence" { long_score += 2; }
    if breakout == "Bullish Breakout" { long_score += 2; }
    long_score += news_pos;

    if is_downtrend { short_score += 2; }
    if latest_rsi > 50.0 { short_score += 1; }
    if latest_macd_hist < 0.0 { short_score += 1; }
    if high_volume { short_score += 1; }
    if pattern == "Bearish Engulfing" || pattern == "Bearish Pinbar" { short_score += 1; }
    if divergence == "Bearish Divergence" { short_score += 2; }
    if breakout == "Bearish Breakdown" { short_score += 2; }
    short_score += news_neg;

    (long_score, short_score, pattern.to_string(), divergence.to_string(), breakout.to_string(), latest_rsi, candle_change_percent)
}

// 4. News Sentiment
async fn fetch_news_sentiment(client: &Client, symbol: &str, finnhub_key: &str) -> (i32, i32) {
    let mut positive_news = 0;
    let mut negative_news = 0;

    if symbol.ends_with("USDT") {
        let coin = symbol.replace("USDT", "");
        let url = format!("https://cryptocurrency.cv/api/ai/sentiment?asset={}", coin);
        if let Ok(resp) = client.get(&url).send().await {
            if let Ok(json) = resp.json::<Value>().await {
                if let Some(label) = json["label"].as_str() {
                    match label {
                        "bullish" => positive_news = 3,
                        "bearish" => negative_news = 3,
                        _ => {}
                    }
                }
            }
        }
    } else {
        if finnhub_key.is_empty() { return (0, 0); }
        let url = format!("https://finnhub.io/api/v1/news?category=general&token={}", finnhub_key);
        if let Ok(resp) = client.get(&url).send().await {
            if let Ok(json) = resp.json::<Value>().await {
                if let Some(articles) = json.as_array() {
                    let base_symbol = symbol.split('.').next().unwrap_or(symbol).to_lowercase();
                    let search_term = match base_symbol.as_str() {
                        "gc=f" => "gold".to_string(), "si=f" => "silver".to_string(),
                        "aapl" => "apple".to_string(), "msft" => "microsoft".to_string(),
                        "googl" => "google".to_string(), "amzn" => "amazon".to_string(),
                        _ => base_symbol,
                    };

                    for article in articles {
                        let headline = article["headline"].as_str().unwrap_or("").to_lowercase();
                        if headline.contains(&search_term) {
                            if headline.contains("surge") || headline.contains("buy") || headline.contains("up") || headline.contains("beat") { positive_news += 1; }
                            else if headline.contains("drop") || headline.contains("sell") || headline.contains("down") || headline.contains("miss") { negative_news += 1; }
                        }
                    }
                }
            }
        }
    }
    (positive_news, negative_news)
}

// 5. Analysis & Anti-Late Entry Filter
async fn analyze_asset(client: &Client, symbol: &str, is_crypto: bool, finnhub_key: &str) -> Option<String> {
    let (candles_1h, candles_5m) = if is_crypto {
        tokio::join!(
            fetch_crypto_klines(client, symbol, "1h", 100), 
            fetch_crypto_klines(client, symbol, "5m", 200) 
        )
    } else {
        tokio::join!(
            fetch_yahoo_klines(client, symbol, "1h"), 
            fetch_yahoo_klines(client, symbol, "5m")
        )
    };

    if candles_1h.len() < 30 || candles_5m.len() < 20 { 
        println!("[{}] Skipped: Not enough data.", symbol);
        return None; 
    }

    let (pos_news, neg_news) = fetch_news_sentiment(client, symbol, finnhub_key).await;
    let (long_score, short_score, pattern, divergence, breakout, latest_rsi, candle_change) = 
        get_scores(&candles_1h, &candles_5m, pos_news * 2, neg_news * 2);
    
    let current_price = candles_5m.last().unwrap().close;

    let news_info = if pos_news > 0 { format!("Positive ({})", pos_news) } 
                    else if neg_news > 0 { format!("Negative ({})", neg_news) } 
                    else { "Neutral".to_string() };

    // Anti-Late Entry Logic: Agar 5m candle already 1.2% dump/pump ho chuki hai, toh entry mat do
    if short_score >= 6 && short_score > long_score {
        if latest_rsi < 25.0 || candle_change < -1.2 {
            println!("[{}] Skipped SHORT: Late entry blocked (RSI: {:.1}, Candle: {:.2}%)", symbol, latest_rsi, candle_change);
            return None; 
        }
        return Some(format!(
            "🔴 <b>NEW SHORT ENTRY: {}</b>\n\nEntry Price: <code>{:.2}</code>\n🔥 Score: {}/10\n📊 RSI: {:.1}\n📰 News: {}\n🕯️ Pattern: {}\n⚡ Divergence: {}\n🚀 S/R: {}",
            symbol, current_price, short_score, latest_rsi, news_info, pattern, divergence, breakout
        ));
    } else if long_score >= 6 && long_score > short_score {
        if latest_rsi > 75.0 || candle_change > 1.2 {
            println!("[{}] Skipped LONG: Late entry blocked (RSI: {:.1}, Candle: {:.2}%)", symbol, latest_rsi, candle_change);
            return None;
        }
        return Some(format!(
            "🟢 <b>NEW LONG ENTRY: {}</b>\n\nEntry Price: <code>{:.2}</code>\n🔥 Score: {}/10\n📊 RSI: {:.1}\n📰 News: {}\n🕯️ Pattern: {}\n⚡ Divergence: {}\n🚀 S/R: {}",
            symbol, current_price, long_score, latest_rsi, news_info, pattern, divergence, breakout
        ));
    }
    None
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let bot = Bot::from_env();
    let chat_id = teloxide::types::ChatId(std::env::var("TELEGRAM_CHAT_ID").unwrap().parse().unwrap());
    let finnhub_key = std::env::var("FINNHUB_API_KEY").unwrap_or_default();
    let client = Client::new();
    
    // Yahan aap apne symbols add/remove kar sakte hain
    let crypto_symbols = vec!["BTCUSDT", "ETHUSDT", "SOLUSDT"];
    let stock_symbols = vec![
        "RELIANCE.NS", "TCS.NS", "AAPL", "TSLA", "MSFT", "GC=F", "SI=F" 
    ];

    println!("🔍 Final 5-Minute Scanner Running (With News & Anti-Late Entry)...");

    let mut crypto_tasks = vec![];
    for symbol in &crypto_symbols {
        crypto_tasks.push(analyze_asset(&client, symbol, true, &finnhub_key));
    }
    for (i, result) in join_all(crypto_tasks).await.into_iter().enumerate() {
        if let Some(msg) = result {
            let _ = bot.send_message(chat_id.clone(), msg).parse_mode(ParseMode::Html).await;
            println!("✅ Crypto Signal Sent: {}", crypto_symbols[i]);
        }
    }

    let mut stock_tasks = vec![];
    for symbol in &stock_symbols {
        stock_tasks.push(analyze_asset(&client, symbol, false, &finnhub_key));
    }
    for (i, result) in join_all(stock_tasks).await.into_iter().enumerate() {
        if let Some(msg) = result {
            let _ = bot.send_message(chat_id.clone(), msg).parse_mode(ParseMode::Html).await;
            println!("✅ Stock Signal Sent: {}", stock_symbols[i]);
        }
    }
    
    println!("✅ Scan complete.");
}
