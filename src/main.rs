use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Datelike, Duration, Timelike, Utc, Weekday};
use chrono_tz::{America::New_York, Asia::Kolkata};
use dotenvy::dotenv;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, env, fs, path::Path, time::Duration as StdDuration};
use tokio::time::{interval, MissedTickBehavior};
use tracing::{error, info, warn};

const STATE_FILE: &str = "bot_state_v2.json";
const CYCLE_MINUTES: u64 = 15;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Candle {
    timestamp: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

#[derive(Debug, Clone)]
struct Asset {
    symbol: &'static str,
    kind: AssetKind,
    strategy: Strategy,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum AssetKind { Crypto, India, Us, Metal }

#[derive(Debug, Clone, Copy, PartialEq)]
enum Strategy { Scalping, Intraday, Swing }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct BotState {
    last_signal: HashMap<String, i64>,
    paper_trades: Vec<PaperTrade>,
    day_key: String,
    daily_realized_r: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PaperTrade {
    id: String,
    symbol: String,
    side: String,
    entry: f64,
    stop: f64,
    target: f64,
    quantity: f64,
    risk_amount: f64,
    opened_at: i64,
    status: String,
}

#[derive(Debug, Clone)]
struct Signal {
    symbol: String,
    side: String,
    strategy: String,
    price: f64,
    stop: f64,
    target: f64,
    quantity: f64,
    risk_amount: f64,
    score: i32,
    rsi: f64,
    atr: f64,
    volume_ratio: f64,
    reason: String,
}

fn env_f64(key: &str, default: f64) -> f64 {
    env::var(key).ok().and_then(|v| v.parse().ok()).filter(|v: &f64| v.is_finite()).unwrap_or(default)
}
fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn valid_candles(c: &[Candle]) -> bool {
    c.len() >= 60 && c.iter().all(|x|
        x.open.is_finite() && x.high.is_finite() && x.low.is_finite() &&
        x.close.is_finite() && x.volume.is_finite() &&
        x.open > 0.0 && x.close > 0.0 && x.high >= x.low &&
        x.high >= x.open.max(x.close) && x.low <= x.open.min(x.close) && x.volume >= 0.0
    )
}

async fn get_json(client: &Client, url: &str) -> Result<serde_json::Value> {
    let mut last_error = None;
    for attempt in 0..3 {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                return resp.json::<serde_json::Value>().await.context("invalid JSON response");
            }
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                last_error = Some(anyhow!("HTTP {}: {}", status, body.chars().take(200).collect::<String>()));
                if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                    tokio::time::sleep(StdDuration::from_millis(500 * (attempt + 1))).await;
                    continue;
                }
                break;
            }
            Err(e) => {
                last_error = Some(anyhow!(e));
                tokio::time::sleep(StdDuration::from_millis(500 * (attempt + 1))).await;
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("request failed")))
}

fn kraken_pair(symbol: &str) -> Option<&'static str> {
    match symbol { "BTCUSDT" => Some("XBTUSD"), "ETHUSDT" => Some("ETHUSD"), "SOLUSDT" => Some("SOLUSD"), _ => None }
}

async fn fetch_kraken(client: &Client, symbol: &str, interval_min: u32) -> Result<Vec<Candle>> {
    let pair = kraken_pair(symbol).ok_or_else(|| anyhow!("unsupported Kraken symbol"))?;
    let url = format!("https://api.kraken.com/0/public/OHLC?pair={}&interval={}", pair, interval_min);
    let json = get_json(client, &url).await?;
    if !json["error"].as_array().unwrap_or(&vec![]).is_empty() {
        return Err(anyhow!("Kraken returned API error"));
    }
    let result = json["result"].as_object().ok_or_else(|| anyhow!("missing Kraken result"))?;
    let key = result.keys().find(|k| k.as_str() != "last").ok_or_else(|| anyhow!("missing OHLC array"))?;
    let rows = result[key].as_array().ok_or_else(|| anyhow!("invalid OHLC array"))?;
    let mut out = Vec::new();
    for row in rows.iter().take(rows.len().saturating_sub(1)) {
        let a = row.as_array().ok_or_else(|| anyhow!("invalid candle row"))?;
        let num = |i: usize| a.get(i).and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(f64::NAN);
        let ts = a.get(0).and_then(|v| v.as_i64()).unwrap_or(0);
        out.push(Candle { timestamp: ts, open: num(1), high: num(2), low: num(3), close: num(4), volume: num(6) });
    }
    Ok(out)
}

async fn fetch_yahoo(client: &Client, symbol: &str, interval_str: &str, range: &str) -> Result<Vec<Candle>> {
    let encoded = symbol.replace("=", "%3D").replace("^", "%5E");
    let url = format!("https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&range={}", encoded, interval_str, range);
    let json = get_json(client, &url).await?;
    let result = json["chart"]["result"].get(0).ok_or_else(|| anyhow!("Yahoo returned no chart data"))?;
    let timestamps = result["timestamp"].as_array().ok_or_else(|| anyhow!("Yahoo timestamps missing"))?;
    let q = &result["indicators"]["quote"][0];
    let opens = q["open"].as_array().ok_or_else(|| anyhow!("Yahoo opens missing"))?;
    let highs = q["high"].as_array().ok_or_else(|| anyhow!("Yahoo highs missing"))?;
    let lows = q["low"].as_array().ok_or_else(|| anyhow!("Yahoo lows missing"))?;
    let closes = q["close"].as_array().ok_or_else(|| anyhow!("Yahoo closes missing"))?;
    let volumes = q["volume"].as_array();
    let mut out = Vec::new();
    for i in 0..timestamps.len() {
        let get = |arr: &[serde_json::Value]| arr.get(i).and_then(|v| v.as_f64()).unwrap_or(f64::NAN);
        let ts = timestamps[i].as_i64().unwrap_or(0);
        let c = Candle {
            timestamp: ts, open: get(opens), high: get(highs), low: get(lows), close: get(closes),
            volume: volumes.and_then(|v| v.get(i)).and_then(|v| v.as_f64()).unwrap_or(0.0),
        };
        if c.open.is_finite() && c.high.is_finite() && c.low.is_finite() && c.close.is_finite() {
            out.push(c);
        }
    }
    let secs = match interval_str { "15m" => 900, "1h" => 3600, "1d" => 86400, _ => 900 };
    let now = Utc::now().timestamp();
    if out.last().map(|c| now - c.timestamp < secs as i64).unwrap_or(false) { out.pop(); }
    Ok(out)
}

async fn fetch_candles(client: &Client, asset: &Asset, tf: &str) -> Result<Vec<Candle>> {
    match asset.kind {
        AssetKind::Crypto => {
            let mins = match tf { "1d" => 1440, "4h" => 240, "1h" => 60, _ => 5 };
            fetch_kraken(client, asset.symbol, mins).await
        }
        _ => {
            let (iv, range) = match tf {
                "1d" => ("1d", "1y"),
                "4h" => ("1h", "1mo"),
                "1h" => ("1h", "3mo"),
                _ => ("15m", "5d"),
            };
            fetch_yahoo(client, asset.symbol, iv, range).await
        }
    }
}

fn ema(values: &[f64], period: usize) -> Option<f64> {
    if values.len() < period || period == 0 { return None; }
    let k = 2.0 / (period as f64 + 1.0);
    let mut e = values[..period].iter().sum::<f64>() / period as f64;
    for v in &values[period..] { e = v * k + e * (1.0 - k); }
    Some(e)
}

fn rsi(values: &[f64], period: usize) -> Option<f64> {
    if values.len() <= period { return None; }
    let (mut gain, mut loss) = (0.0, 0.0);
    for i in 1..=period {
        let d = values[i] - values[i-1];
        if d > 0.0 { gain += d; } else { loss -= d; }
    }
    let (mut ag, mut al) = (gain / period as f64, loss / period as f64);
    for i in period+1..values.len() {
        let d = values[i] - values[i-1];
        ag = (ag * (period as f64 - 1.0) + d.max(0.0)) / period as f64;
        al = (al * (period as f64 - 1.0) + (-d).max(0.0)) / period as f64;
    }
    if al == 0.0 { Some(100.0) } else { Some(100.0 - 100.0 / (1.0 + ag/al)) }
}

fn atr(c: &[Candle], period: usize) -> Option<f64> {
    if c.len() <= period { return None; }
    let mut trs = Vec::new();
    for i in 1..c.len() {
        let prev = c[i-1].close;
        trs.push((c[i].high-c[i].low).max((c[i].high-prev).abs()).max((c[i].low-prev).abs()));
    }
    let mut value = trs[..period].iter().sum::<f64>() / period as f64;
    for tr in &trs[period..] { value = (value*(period as f64-1.0)+tr)/period as f64; }
    Some(value)
}

fn volume_ratio(c: &[Candle], period: usize) -> f64 {
    if c.len() <= period { return 0.0; }
    let base = c[c.len()-period-1..c.len()-1].iter().map(|x| x.volume).sum::<f64>() / period as f64;
    if base <= 0.0 { 0.0 } else { c.last().unwrap().volume / base }
}

fn pivot_divergence(c: &[Candle], values: &[f64]) -> (bool, bool) {
    if c.len() < 20 || values.len() != c.len() { return (false, false); }
    let mut lows = Vec::new();
    let mut highs = Vec::new();
    for i in 2..c.len()-2 {
        if c[i].low < c[i-1].low && c[i].low < c[i-2].low && c[i].low <= c[i+1].low && c[i].low <= c[i+2].low { lows.push(i); }
        if c[i].high > c[i-1].high && c[i].high > c[i-2].high && c[i].high >= c[i+1].high && c[i].high >= c[i+2].high { highs.push(i); }
    }
    let bull = if lows.len() >= 2 {
        let (a,b) = (lows[lows.len()-2], lows[lows.len()-1]);
        c[b].low < c[a].low && values[b] > values[a] + 2.0
    } else { false };
    let bear = if highs.len() >= 2 {
        let (a,b) = (highs[highs.len()-2], highs[highs.len()-1]);
        c[b].high > c[a].high && values[b] < values[a] - 2.0
    } else { false };
    (bull, bear)
}

fn market_open(asset: &Asset) -> bool {
    let now = Utc::now();
    match asset.kind {
        AssetKind::Crypto => true,
        AssetKind::India => {
            let t = now.with_timezone(&Kolkata);
            !matches!(t.weekday(), Weekday::Sat | Weekday::Sun) &&
            ((t.hour() > 9 || (t.hour() == 9 && t.minute() >= 15)) &&
             (t.hour() < 15 || (t.hour() == 15 && t.minute() <= 30)))
        }
        AssetKind::Us => {
            let t = now.with_timezone(&New_York);
            !matches!(t.weekday(), Weekday::Sat | Weekday::Sun) &&
            ((t.hour() > 9 || (t.hour() == 9 && t.minute() >= 30)) && (t.hour() < 16))
        }
        AssetKind::Metal => {
            let t = now.with_timezone(&New_York);
            !matches!(t.weekday(), Weekday::Sat) &&
            !(t.weekday() == Weekday::Sun && t.hour() < 18) &&
            !(t.weekday() == Weekday::Fri && t.hour() >= 17)
        }
    }
}

fn build_signal(asset: &Asset, entry: &[Candle], trend: &[Candle], btc_bullish: Option<bool>) -> Option<Signal> {
    if !valid_candles(entry) || !valid_candles(trend) { return None; }
    let closes: Vec<f64> = entry.iter().map(|c| c.close).collect();
    let trend_closes: Vec<f64> = trend.iter().map(|c| c.close).collect();
    let price = *closes.last()?;
    let r = rsi(&closes, 14)?;
    let a = atr(entry, 14)?;
    let ema50 = ema(&trend_closes, 50)?;
    
    if !price.is_finite() || !a.is_finite() || a <= 0.0 || price <= 0.0 { return None; }

    let last = entry.last()?;
    let prev = &entry[entry.len()-2];
    let uptrend = *trend_closes.last()? > ema50;
    let downtrend = *trend_closes.last()? < ema50;
    let vr = volume_ratio(entry, 20);
    
    let rsi_values: Vec<f64> = (0..closes.len())
        .map(|i| if i >= 14 { rsi(&closes[..=i], 14).unwrap_or(50.0) } else { 50.0 })
        .collect();
    let (bull_div, bear_div) = pivot_divergence(entry, &rsi_values);

    let bull_engulf = last.close > last.open && prev.close < prev.open && last.close >= prev.open && last.open <= prev.close;
    let bear_engulf = last.close < last.open && prev.close > prev.open && last.open >= prev.close && last.close <= prev.open;

    let range_high = entry[entry.len()-21..entry.len()-1].iter().map(|c| c.high).fold(f64::MIN, f64::max);
    let range_low = entry[entry.len()-21..entry.len()-1].iter().map(|c| c.low).fold(f64::MAX, f64::min);
    let bull_break = last.close > range_high;
    let bear_break = last.close < range_low;

    let mut long = 0;
    let mut short = 0;

    if r < 25.0 {
        short -= 5; long += 2;
    } else if r > 75.0 {
        long -= 5; short += 2;
    } else {
        if uptrend && r >= 52.0 && r <= 68.0 { long += 2; }
        if downtrend && r <= 48.0 && r >= 32.0 { short += 2; }
    }

    if bull_engulf { long += 1; }
    if bear_engulf { short += 1; }
    if bull_div { long += 2; }
    if bear_div { short += 2; }
    if bull_break { long += 2; }
    if bear_break { short += 2; }

    if vr >= 1.5 && last.close > last.open { long += 1; }
    if vr >= 1.5 && last.close < last.open { short += 1; }

    // BTC Macro Filter
    if asset.kind == AssetKind::Crypto && asset.symbol != "BTCUSDT" {
        if let Some(is_btc_bull) = btc_bullish {
            if long >= 6 && !is_btc_bull { long -= 5; }
            if short >= 6 && is_btc_bull { short -= 5; }
        }
    }

    let (side, score) = if long >= 4 && long > short {
        ("LONG", long)
    } else if short >= 4 && short > long {
        ("SHORT", short)
    } else { return None; };

    let sl_mult = match asset.strategy { Strategy::Swing => 3.0, Strategy::Intraday => 2.5, _ => 2.0 };
    let tp_mult = match asset.strategy { Strategy::Swing => 6.0, Strategy::Intraday => 5.0, _ => 4.0 };

    let stop_distance = (a * sl_mult).max(price * 0.002);
    let (stop, target) = if side == "LONG" {
        (price - stop_distance, price + (stop_distance * (tp_mult / sl_mult)))
    } else {
        (price + stop_distance, price - (stop_distance * (tp_mult / sl_mult)))
    };
    
    if stop <= 0.0 || target <= 0.0 || !stop.is_finite() || !target.is_finite() { return None; }

    let equity = env_f64("PAPER_EQUITY", 100000.0);
    let risk_pct = env_f64("RISK_PER_TRADE_PCT", 0.5).clamp(0.05, 2.0);
    let risk_amount = equity * risk_pct / 100.0;
    let quantity = risk_amount / stop_distance;
    
    if !quantity.is_finite() || quantity <= 0.0 { return None; }

    let strat_str = format!("{:?}", asset.strategy).to_uppercase();
    
    Some(Signal { 
        symbol: asset.symbol.into(), side: side.into(), strategy: strat_str,
        price, stop, target, quantity, risk_amount, score, rsi: r, atr: a, volume_ratio: vr, 
        reason: format!("RSI {:.1}; Vol {:.1}x", r, vr)
    })
}

async fn send_telegram(client: &Client, message: &str) -> Result<()> {
    let token = env::var("TELEGRAM_BOT_TOKEN").context("TELEGRAM_BOT_TOKEN missing")?;
    let chat = env::var("TELEGRAM_CHAT_ID").context("TELEGRAM_CHAT_ID missing")?;
    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
    let body = serde_json::json!({ "chat_id": chat, "text": message, "parse_mode": "HTML", "disable_web_page_preview": true });
    let resp = client.post(url).json(&body).send().await?;
    if !resp.status().is_success() { return Err(anyhow!("Telegram API returned {}", resp.status())); }
    Ok(())
}

fn format_signal(s: &Signal) -> String {
    let emoji = if s.side == "LONG" { "🟢" } else { "🔴" };
    format!(
        "{} <b>PAPER SIGNAL — {} {}</b>\n\
        ━━━━━━━━━━━━━━━━━━━━\n\
        🌍 <b>Asset:</b> {}\n\
        ⏰ <b>Type:</b> {}\n\
        ━━━━━━━━━━━━━━━━━━━━\n\
        💵 <b>Entry:</b> {:.6}\n\
        🛑 <b>Stop:</b> {:.6}\n\
        🎯 <b>Target:</b> {:.6}\n\
        📊 <b>Score:</b> {}/10 🏆\n\
        ━━━━━━━━━━━━━━━━━━━━\n\
        🚀 <b>Qty:</b> {:.6}\n\
        💰 <b>Risk:</b> {:.2}\n\
        📈 <b>RSI:</b> {:.1} | <b>Vol:</b> {:.2}x\n\
        📝 <b>Reason:</b> {}\n\n\
        <i>Signal only. No real order was placed.</i>",
        emoji, s.symbol, s.side, s.symbol, s.strategy, s.price, s.stop, s.target, s.score.min(10), s.quantity, s.risk_amount, s.rsi, s.volume_ratio, s.reason
    )
}

fn load_state() -> BotState {
    fs::read_to_string(STATE_FILE).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}
fn save_state(state: &BotState) -> Result<()> {
    let tmp = format!("{}.tmp", STATE_FILE);
    fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
    fs::rename(tmp, STATE_FILE)?;
    Ok(())
}

fn manage_paper_trades(state: &mut BotState, latest: &HashMap<String, f64>) {
    for trade in state.paper_trades.iter_mut().filter(|t| t.status == "OPEN") {
        if let Some(price) = latest.get(&trade.symbol) {
            let hit_stop = if trade.side == "LONG" { *price <= trade.stop } else { *price >= trade.stop };
            let hit_target = if trade.side == "LONG" { *price >= trade.target } else { *price <= trade.target };
            if hit_stop || hit_target {
                trade.status = if hit_stop { "STOPPED" } else { "TARGET" }.into();
                let r = if hit_stop { -1.0 } else { 2.0 };
                state.daily_realized_r += r;
                info!("Paper trade {} closed: {}", trade.id, trade.status);
            }
        }
    }
}

async fn scan_once(client: &Client, assets: &[Asset], state: &mut BotState) -> Result<()> {
    let today = Utc::now().format("%Y-%m-%d").to_string();
    if state.day_key != today { state.day_key = today; state.daily_realized_r = 0.0; }

    let max_daily_loss_r = env_f64("MAX_DAILY_LOSS_R", 3.0);
    let mut latest = HashMap::new();
    
    // Fetch BTC trend for macro filter
    let mut btc_bullish = None;
    if let Ok(btc_c) = fetch_candles(client, &Asset { symbol: "BTCUSDT", kind: AssetKind::Crypto, strategy: Strategy::Intraday }, "1h").await {
        if valid_candles(&btc_c) {
            let cc: Vec<f64> = btc_c.iter().map(|c| c.close).collect();
            if let (Some(bema), Some(blast)) = (ema(&cc, 50), cc.last()) {
                btc_bullish = Some(*blast > bema);
            }
        }
    }

    for asset in assets {
        if !market_open(asset) { continue; }
        let fetched = tokio::try_join!(
            fetch_candles(client, asset, "15m"),
            fetch_candles(client, asset, "1h")
        );
        let (entry, trend) = match fetched {
            Ok(v) => v,
            Err(e) => { warn!("{} data fetch failed: {:#}", asset.symbol, e); continue; }
        };
        if !valid_candles(&entry) || !valid_candles(&trend) { continue; }
        let price = entry.last().unwrap().close;
        latest.insert(asset.symbol.to_string(), price);

        if state.daily_realized_r <= -max_daily_loss_r {
            warn!("Daily paper risk stop reached; no new signals today");
            continue;
        }
        if state.paper_trades.iter().filter(|t| t.status == "OPEN").count() >= env_u64("MAX_OPEN_PAPER_TRADES", 5) as usize {
            continue;
        }

        let Some(signal) = build_signal(asset, &entry, &trend, btc_bullish) else { continue; };
        let key = format!("{}:{}:{}", signal.symbol, signal.side, signal.strategy);
        let now = Utc::now().timestamp();
        let cooldown = env_u64("SIGNAL_COOLDOWN_MINUTES", 60) as i64 * 60;
        if now.saturating_sub(*state.last_signal.get(&key).unwrap_or(&0)) < cooldown { continue; }

        let id = format!("{}-{}", signal.symbol, now);
        let message = format_signal(&signal);
        match send_telegram(client, &message).await {
            Ok(()) => {
                state.last_signal.insert(key, now);
                state.paper_trades.push(PaperTrade {
                    id, symbol: signal.symbol.clone(), side: signal.side.clone(), entry: signal.price,
                    stop: signal.stop, target: signal.target, quantity: signal.quantity,
                    risk_amount: signal.risk_amount, opened_at: now, status: "OPEN".into()
                });
                info!("Sent {} {} paper signal ({})", signal.side, signal.symbol, signal.strategy);
            }
            Err(e) => error!("Telegram 
             send failed for {}: {:#}", signal.symbol, e),
        }
    }
    manage_paper_trades(state, &latest);
    save_state(state)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    let client = Client::builder()
        .timeout(StdDuration::from_secs(15))
        .user_agent("Mozilla/5.0 RustSignalBotV2")
        .build()?;

    let assets = vec![
        Asset { symbol: "BTCUSDT", kind: AssetKind::Crypto, strategy: Strategy::Scalping },
        Asset { symbol: "BTCUSDT", kind: AssetKind::Crypto, strategy: Strategy::Intraday },
        Asset { symbol: "BTCUSDT", kind: AssetKind::Crypto, strategy: Strategy::Swing },
        Asset { symbol: "ETHUSDT", kind: AssetKind::Crypto, strategy: Strategy::Scalping },
        Asset { symbol: "ETHUSDT", kind: AssetKind::Crypto, strategy: Strategy::Intraday },
        Asset { symbol: "ETHUSDT", kind: AssetKind::Crypto, strategy: Strategy::Swing },
        Asset { symbol: "SOLUSDT", kind: AssetKind::Crypto, strategy: Strategy::Scalping },
        Asset { symbol: "SOLUSDT", kind: AssetKind::Crypto, strategy: Strategy::Intraday },
        Asset { symbol: "SOLUSDT", kind: AssetKind::Crypto, strategy: Strategy::Swing },
        Asset { symbol: "RELIANCE.NS", kind: AssetKind::India, strategy: Strategy::Intraday },
        Asset { symbol: "TCS.NS", kind: AssetKind::India, strategy: Strategy::Intraday },
        Asset { symbol: "AAPL", kind: AssetKind::Us, strategy: Strategy::Intraday },
        Asset { symbol: "TSLA", kind: AssetKind::Us, strategy: Strategy::Intraday },
        Asset { symbol: "MSFT", kind: AssetKind::Us, strategy: Strategy::Intraday },
        Asset { symbol: "GC=F", kind: AssetKind::Metal, strategy: Strategy::Intraday },
        Asset { symbol: "SI=F", kind: AssetKind::Metal, strategy: Strategy::Intraday },
    ];

    let mut state = load_state();
    info!("Rust Signal Bot V2 started. Scan interval: {} minutes. LIVE ORDERS: DISABLED.", CYCLE_MINUTES);

    let mut ticker = interval(StdDuration::from_secs(CYCLE_MINUTES * 60));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        if let Err(e) = scan_once(&client, &assets, &mut state).await {
            error!("Scan cycle failed: {:#}", e);
        }
    }
               }
