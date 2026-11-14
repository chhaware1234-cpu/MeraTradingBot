use reqwest::Client;
use ta::indicators::{RelativeStrengthIndex, MovingAverageConvergenceDivergence, ExponentialMovingAverage, AverageTrueRange};
use ta::Next;
use teloxide::{prelude::*, types::ParseMode};
use futures::future::join_all;
use serde_json::Value;
use serde::{Serialize, Deserialize};
use chrono::{Utc, Timelike, Datelike, Weekday};
use chrono_tz::America::New_York;
use chrono_tz::Asia::Kolkata;
use std::fs;
use std::collections::HashMap;

#[derive(Debug, Clone)]
struct Candle {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

#[derive(Serialize, Deserialize, Default)]
struct SignalCache {
    signals: HashMap<String, u64>, // Format: "BTCUSDT_LONG" -> timestamp
}

// 1. Kraken Fetching
async fn fetch_crypto_klines(client: &Client, symbol: &str, interval: &str) -> Vec<Candle> {
    let pair = match symbol {
        "BTCUSDT" => "XBTUSD",
        "ETHUSDT" => "ETHUSD",
        "SOLUSDT" => "SOLUSD",
        _ => return vec![],
    };
    let kraken_interval = if interval == "1h" { "60" } else { "5" };
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
                                    if c.len() >= 7 {
                                        candles.push(Candle {
                                            open: c.get(1).and_then(|v| v.as_str()).unwrap_or("0").parse().unwrap_or(0.0),
                                            high: c.get(2).and_then(|v| v.as_str()).unwrap_or("0").parse().unwrap_or(0.0),
                                            low: c.get(3).and_then(|v| v.as_str()).unwrap_or("0").parse().unwrap_or(0.0),
                                            close: c.get(4).and_then(|v| v.as_str()).unwrap_or("0").parse().unwrap_or(0.0),
                                            volume: c.get(6).and_then(|v| v.as_str()).unwrap_or("0").parse().unwrap_or(0.0),
                                        });
                                    }
                                }
                            }
                            candles.pop(); // Drop incomplete
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

// 2. Yahoo Fetching (Fixed Timestamp Pop)
async fn fetch_yahoo_klines(client: &Client, symbol: &str, interval: &str) -> Vec<Candle> {
    let range = if interval == "1h" { "1mo" } else { "5d" };
    let url = format!("https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&range={}", symbol, interval, range);
    let req = client.get(&url).header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)").send().await;
    
    if let Ok(resp) = req {
        if let Ok(json) = resp.json::<Value>().await {
            if let Some(result) = json["chart"]["result"].as_array() {
                if let Some(data) = result.get(0) {
                    if let Some(quote) = data["indicators"]["quote"].as_array().and_then(|a| a.get(0)) {
                        let mut candles = vec![];
                        if let (Some(o), Some(h), Some(l), Some(c), Some(v), Some(t)) = (
                            quote["open"].as_array(), quote["high"].as_array(), quote["low"].as_array(), quote["close"].as_array(), quote["volume"].as_array(), data["timestamp"].as_array()
                        ) {
                            for i in 0..c.len() {
                                if let (Some(open), Some(high), Some(low), Some(close)) = (
                                    o.get(i).and_then(|x| x.as_f64()), h.get(i).and_then(|x| x.as_f64()),
                                    l.get(i).and_then(|x| x.as_f64()), c.get(i).and_then(|x| x.as_f64()),
                                ) {
                                    let volume = v.get(i).and_then(|x| x.as_f64()).unwrap_or(0.0);
                                    let ts = t.get(i).and_then(|x| x.as_i64()).unwrap_or(0);
                                    candles.push((ts, Candle { open, high, low, close, volume }));
                                }
                            }
                        }
                        
                        // Timestamp based pop (agar last candle incomplete hai)
                        if let Some((last_ts, _)) = candles.last() {
                            let now = Utc::now().timestamp();
                            let interval_secs = if interval == "1h" { 3600 } else { 300 };
                            if now - last_ts < interval_secs {
                                candles.pop();
                            }
                        }
                        return candles.into_iter().map(|(_, c)| c).collect();
                    }
                }
            }
        }
    }
    vec![]
}

// 3. Market Hours
fn is_market_open(symbol: &str) -> bool {
    let now_utc = Utc::now();
    if symbol.ends_with(".NS") {
        let now_ist = now_utc.with_timezone(&Kolkata);
        if now_ist.weekday() == Weekday::Sat || now_ist.weekday() == Weekday::Sun { return false; }
        let mins = now_ist.hour() * 60 + now_ist.minute();
        return mins >= (9 * 60 + 15) && mins <= (15 * 60 + 30);
    } else if ["AAPL", "TSLA", "MSFT", "AMZN", "GOOGL", "NVDA", "META", "NFLX", "AMD", "JPM"].contains(&symbol) {
        let now_est = now_utc.with_timezone(&New_York);
        if now_est.weekday() == Weekday::Sat || now_est.weekday() == Weekday::Sun { return false; }
        let mins = now_est.hour() * 60 + now_est.minute();
        return mins >= (9 * 60 + 30) && mins <= (16 * 60);
    } else if symbol == "GC=F" || symbol == "SI=F" {
        let day = now_utc.weekday();
        if day == Weekday::Sat { return false; }
        return true;
    }
    true
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

// Fixed: Swing Points based Divergence
fn detect_divergence(candles: &[Candle], rsi_values: &[f64]) -> &'static str {
    if candles.len() < 35 || rsi_values.len() < 35 { return "None"; }
    let len = candles.len();
    let current_rsi = rsi_values[len - 1];

    // Bullish Divergence
    let old_low = candles[len - 35..len - 10].iter().map(|c| c.low).fold(f64::MAX, f64::min);
    let old_rsi_low = rsi_values[len - 35..len - 10].iter().fold(f64::MAX, |a, &b| a.min(b));
    let recent_low = candles[len - 5..len].iter().map(|c| c.low).fold(f64::MAX, f64::min);
    let recent_rsi_low = rsi_values[len - 5..len].iter().fold(f64::MAX, |a, &b| a.min(b));

    if recent_low < old_low && recent_rsi_low > old_rsi_low && current_rsi < 45.0 {
        return "Bullish Divergence";
    }

    // Bearish Divergence
    let old_high = candles[len - 35..len - 10].iter().map(|c| c.high).fold(f64::MIN, f64::max);
    let old_rsi_high = rsi_values[len - 35..len - 10].iter().fold(f64::MIN, |a, &b| a.max(b));
    let recent_high = candles[len - 5..len].iter().map(|c| c.high).fold(f64::MIN, f64::max);
    let recent_rsi_high = rsi_values[len - 5..len].iter().fold(f64::MIN, |a, &b| a.max(b));

    if recent_high > old_high && recent_rsi_high < old_rsi_high && current_rsi > 55.0 {
        return "Bearish Divergence";
    }

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

fn get_scores(candles_1h: &[Candle], candles_5m: &[Candle], news_pos: i32, news_neg: i32) -> Option<(i32, i32, String, String, String, f64, f64, f64, f64)> {
    if candles_1h.len() < 50 || candles_5m.len() < 35 { return None; }

    let mut ema_50_1h = ExponentialMovingAverage::new(50).unwrap();
    let mut trend_ema = 0.0;
    for c in candles_1h { trend_ema = ema_50_1h.next(c.close); }
    let is_uptrend = candles_1h.last().unwrap().close > trend_ema;
    let is_downtrend = candles_1h.last().unwrap().close < trend_ema;

    let mut rsi_5m = RelativeStrengthIndex::new(14).unwrap();
    let mut macd_5m = MovingAverageConvergenceDivergence::new(12, 26, 9).unwrap();
    let mut atr_5m = AverageTrueRange::new(14).unwrap();
    
    let mut rsi_values = Vec::new();
    let mut latest_macd_hist = 0.0;
    let mut latest_atr = 0.0;
    let vol_period = 20;
    let mut vol_sma = 0.0;

    for c in candles_5m.iter() {
        rsi_values.push(rsi_5m.next(c.close));
        latest_macd_hist = macd_5m.next(c.close).histogram;
        if let Ok(item) = ta::DataItem::builder().open(c.open).high(c.high).low(c.low).close(c.close).volume(c.volume).build() {
            latest_atr = atr_5m.next(&item);
        }
    }

    if latest_atr <= 0.0 { return None; }

    for i in (candles_5m.len() - vol_period - 1)..(candles_5m.len() - 1) {
        vol_sma += candles_5m[i].volume;
    }
    vol_sma /= vol_period as f64;

    let current_vol = candles_5m.last().unwrap().volume;
    let vol_ratio = if vol_sma > 0.0 { current_vol / vol_sma } else { 1.0 };
    let high_volume = vol_ratio > 1.5; 
    
    let pattern = detect_pattern(candles_5m);
    let divergence = detect_divergence(candles_5m, &rsi_values);
    let breakout = detect_sr_breakout(candles_5m);
    let latest_rsi = *rsi_values.last().unwrap();

    let current_candle = candles_5m.last().unwrap();
    let candle_change_percent = ((current_candle.close - current_candle.open) / current_candle.open) * 100.0;

    let mut long_score = 0;
    let mut short_score = 0;

    if is_uptrend { long_score += 2; }
    if latest_rsi > 50.0 && (latest_rsi < 70.0 || breakout == "Bullish Breakout") { long_score += 1; } 
    if latest_macd_hist > 0.0 { long_score += 1; }
    if high_volume { long_score += 1; }
    if pattern == "Bullish Engulfing" || pattern == "Bullish Pinbar" { long_score += 1; }
    if divergence == "Bullish Divergence" { long_score += 2; }
    if breakout == "Bullish Breakout" { long_score += 2; }
    long_score += news_pos.min(2); 

    if is_downtrend { short_score += 2; }
    if latest_rsi < 50.0 && (latest_rsi > 30.0 || breakout == "Bearish Breakdown") { short_score += 1; } 
    if latest_macd_hist < 0.0 { short_score += 1; }
    if high_volume { short_score += 1; }
    if pattern == "Bearish Engulfing" || pattern == "Bearish Pinbar" { short_score += 1; }
    if divergence == "Bearish Divergence" { short_score += 2; }
    if breakout == "Bearish Breakdown" { short_score += 2; }
    short_score += news_neg.min(2);

    Some((long_score, short_score, pattern.to_string(), divergence.to_string(), breakout.to_string(), latest_rsi, candle_change_percent, vol_ratio, latest_atr))
}

// Crypto News (Printing label for verification)
async fn fetch_crypto_news(client: &Client, coin: &str) -> (i32, i32) {
    let url = format!("https://cryptocurrency.cv/api/ai/sentiment?asset={}", coin);
    let mut pos = 0; let mut neg = 0;
    if let Ok(resp) = client.get(&url).send().await {
        if let Ok(json) = resp.json::<Value>().await {
            // DEBUG: Print raw response once to verify field
            println!("[DEBUG] Crypto News Raw for {}: {:?}", coin, json);
            
            if let Some(label) = json.get("label").and_then(|v| v.as_str()) {
                match label {
                    "bullish" => pos = 3, "bearish" => neg = 3, _ => {}
                }
            }
        }
    }
    (pos, neg)
}

async fn analyze_asset(
    client: &Client, 
    symbol: &str, 
    is_crypto: bool, 
    finnhub_headlines: &[String],
    cache: &SignalCache,
) -> Option<String> {
    if !is_market_open(symbol) { return None; }

    let (candles_1h, candles_5m) = if is_crypto {
        tokio::join!(fetch_crypto_klines(client, symbol, "1h"), fetch_crypto_klines(client, symbol, "5m"))
    } else {
        tokio::join!(fetch_yahoo_klines(client, symbol, "1h"), fetch_yahoo_klines(client, symbol, "5m"))
    };

    if candles_1h.len() < 50 || candles_5m.len() < 35 { return None; }

    let (pos_news, neg_news) = if symbol.ends_with("USDT") {
        fetch_crypto_news(client, &symbol.replace("USDT", "")).await
    } else if symbol.ends_with(".NS") {
        (0, 0) // Skip news for Indian Stocks
    } else {
        let mut pos = 0; let mut neg = 0;
        let base_symbol = symbol.split('.').next().unwrap_or(symbol).to_lowercase();
        let search_term = match base_symbol.as_str() {
            "gc=f" => "gold price", "si=f" => "silver price",
            "aapl" => "apple", "msft" => "microsoft",
            "googl" => "google", "amzn" => "amazon",
            "tsla" => "tesla", _ => base_symbol.as_str(),
        };

        for headline in finnhub_headlines {
            if headline.contains(search_term) {
                if headline.contains("surge") || headline.contains("bullish") || headline.contains("record high") || headline.contains("beat") { pos += 1; }
                else if headline.contains("drop") || headline.contains("bearish") || headline.contains("crash") || headline.contains("earnings miss") { neg += 1; }
            }
        }
        (pos, neg)
    };

    let (long_score, short_score, pattern, _divergence, breakout, latest_rsi, candle_change, vol_ratio, atr) = 
        get_scores(&candles_1h, &candles_5m, pos_news, neg_news)?;
    
    let current_price = candles_5m.last().unwrap().close;
    let asset_name = symbol.replace("USDT", "").replace(".NS", "");

    let long_score_out_of_100 = (long_score * 10).min(100);
    let short_score_out_of_100 = (short_score * 10).min(100);

    // Currency Fix
    let (margin, currency_symbol) = if symbol.ends_with(".NS") {
        (5000.0, "₹")
    } else {
        (50.0, "$")
    };
    
    let leverage = 10.0; 
    let qty = (margin * leverage) / current_price; 
    let qty_str = if is_crypto { format!("{:.5}", qty) } else { format!("{:.2}", qty) };

    // Cooldown Check (1 hour)
    let now_ts = Utc::now().timestamp() as u64;
    let cooldown = 3600;

    if short_score >= 7 && short_score > long_score {
        let cache_key = format!("{}_SHORT", asset_name);
        if let Some(&last_ts) = cache.signals.get(&cache_key) {
            if now_ts - last_ts < cooldown { return None; }
        }

        if (latest_rsi < 25.0 || candle_change < -1.2) && breakout != "Bearish Breakdown" { return None; }
        
        // ATR Relaxed to 2.0x and 4.0x
        let sl = current_price + (atr * 2.0); 
        let tp = current_price - (atr * 4.0);  
        
        return Some(format!(
            "🔴 <b>SELL SIGNAL (SHORT)</b> 🔴\n\
            ━━━━━━━━━━━━━━━━━━━━\n\
            🌍 <b>Asset:</b> {}\n\
            ⏰ <b>Type:</b> SCALPING (5m)\n\
            ━━━━━━━━━━━━━━━━━━━━\n\
            💵 <b>Price:</b> <code>{:.4}</code>\n\
            🛑 <b>SL:</b> <code>{:.4}</code>\n\
            🎯 <b>TP:</b> <code>{:.4}</code>\n\
            📊 <b>Score:</b> {}/100 🏆\n\
            ━━━━━━━━━━━━━━━━━━━━\n\
            🚀 <b>Qty:</b> {} (Margin: {}{:.0}, 10x)\n\
            📈 <b>RSI:</b> {:.1} | <b>Vol:</b> {:.1}x | {}\n\
            📰 <b>News:</b> {}",
            asset_name, current_price, sl, tp, short_score_out_of_100, qty_str, currency_symbol, margin, latest_rsi, vol_ratio, pattern, if neg_news > 0 { "Negative" } else { "Neutral" }
        ));
    } else if long_score >= 7 && long_score > short_score {
        let cache_key = format!("{}_LONG", asset_name);
        if let Some(&last_ts) = cache.signals.get(&cache_key) {
            if now_ts - last_ts < cooldown { return None; }
        }

        if (latest_rsi > 75.0 || candle_change > 1.2) && breakout != "Bullish Breakout" { return None; }

        let sl = current_price - (atr * 2.0); 
        let tp = current_price + (atr * 4.0);  

        return Some(format!(
            "🟢 <b>BUY SIGNAL (LONG)</b> 🟢\n\
            ━━━━━━━━━━━━━━━━━━━━\n\
            🌍 <b>Asset:</b> {}\n\
            ⏰ <b>Type:</b> SCALPING (5m)\n\
            ━━━━━━━━━━━━━━━━━━━━\n\
            💵 <b>Price:</b> <code>{:.4}</code>\n\
            🛑 <b>SL:</b> <code>{:.4}</code>\n\
            🎯 <b>TP:</b> <code>{:.4}</code>\n\
            📊 <b>Score:</b> {}/100 🏆\n\
            ━━━━━━━━━━━━━━━━━━━━\n\
            🚀 <b>Qty:</b> {} (Margin: {}{:.0}, 10x)\n\
            📈 <b>RSI:</b> {:.1} | <b>Vol:</b> {:.1}x | {}\n\
            📰 <b>News:</b> {}",
            asset_name, current_price, sl, tp, long_score_out_of_100, qty_str, currency_symbol, margin, latest_rsi, vol_ratio, pattern, if pos_news > 0 { "Positive" } else { "Neutral" }
        ));
    }
    None
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let bot = Bot::from_env();
    let chat_id_str = std::env::var("TELEGRAM_CHAT_ID").expect("TELEGRAM_CHAT_ID must be set");
    let chat_id: i64 = chat_id_str.parse().expect("TELEGRAM_CHAT_ID must be a valid integer");
    let chat_id = teloxide::types::ChatId(chat_id);
    
    let finnhub_key = std::env::var("FINNHUB_API_KEY").unwrap_or_default();
    let client = Client::new();
    
    let crypto_symbols = vec!["BTCUSDT", "ETHUSDT", "SOLUSDT"];
    let stock_symbols = vec!["RELIANCE.NS", "TCS.NS", "AAPL", "TSLA", "MSFT", "GC=F", "SI=F"];

    // Load Cache
    let cache_path = "signal_cache.json";
    let cache_data = fs::read_to_string(cache_path).unwrap_or_else(|_| "{}".to_string());
    let mut cache: SignalCache = serde_json::from_str(&cache_data).unwrap_or_default();

    // Fetch Finnhub news once
    let mut finnhub_headlines: Vec<String> = vec![];
    if !finnhub_key.is_empty() {
        let url = format!("https://finnhub.io/api/v1/news?category=general&token={}", finnhub_key);
        if let Ok(resp) = client.get(&url).send().await {
            if let Ok(json) = resp.json::<Value>().await {
                if let Some(articles) = json.as_array() {
                    for article in articles {
                        if let Some(headline) = article.get("headline").and_then(|v| v.as_str()) {
                            finnhub_headlines.push(headline.to_lowercase());
                        }
                    }
                }
            }
        }
    }

    println!("🔍 Pro-Scanner V5 Running (Deadlock Fixed, Cooldown Active)...");

    let mut tasks = vec![];
    for symbol in &crypto_symbols {
        tasks.push(analyze_asset(&client, symbol, true, &finnhub_headlines, &cache));
    }
    for symbol in &stock_symbols {
        tasks.push(analyze_asset(&client, symbol, false, &finnhub_headlines, &cache));
    }

    let results = join_all(tasks).await;
    let now_ts = Utc::now().timestamp() as u64;

    for result in results {
        if let Some(msg) = result {
            if let Err(e) = bot.send_message(chat_id.clone(), msg).parse_mode(ParseMode::Html).await {
                eprintln!("❌ Failed to send message: {}", e);
            } else {
                println!("✅ Signal Sent!");
                // Parse asset name from msg to update cache
                if let Some(asset_start) = msg.find("<b>Asset:</b> ") {
                    let asset_part = &msg[asset_start + 14..];
                    if let Some(end_idx) = asset_part.find('\n') {
                        let asset_name = asset_part[..end_idx].trim();
                        let dir = if msg.contains("SELL SIGNAL") { "SHORT" } else { "LONG" };
                        cache.signals.insert(format!("{}_{}", asset_name, dir), now_ts);
                    }
                }
            }
        }
    }

    // Save Cache
    if let Ok(json_str) = serde_json::to_string_pretty(&cache) {
        let _ = fs::write(cache_path, json_str);
    }
    
    println!("✅ Scan complete.");
}
