import yfinance as yf
import requests
import os
import warnings
import datetime
import json
import time
warnings.filterwarnings('ignore')

TELEGRAM_TOKEN = os.environ.get('TELEGRAM_TOKEN')
CHAT_ID = os.environ.get('CHAT_ID')

# ==========================================
# ⚙️ SMART SETTINGS
# ==========================================
MIN_SCORE = 70  # Score wapas 70 kiya taaki fake signals kam aayein
MAX_SIGNALS_PER_RUN = 2 # Ek baar mein sirf top 2 signals bhejega
COOLDOWN_HOURS = 4 # 4 ghante tak wahi coin repeat nahi hoga
COOLDOWN_FILE = "cooldown.json"

def send_telegram_alert(message):
    if not TELEGRAM_TOKEN or not CHAT_ID: return
    url = f"https://api.telegram.org/bot{TELEGRAM_TOKEN}/sendMessage"
    payload = {"chat_id": CHAT_ID, "text": message, "parse_mode": "HTML"}
    try:
        requests.post(url, data=payload)
    except Exception: pass

def load_cooldown():
    if os.path.exists(COOLDOWN_FILE):
        with open(COOLDOWN_FILE, 'r') as f:
            return json.load(f)
    return {}

def save_cooldown(data):
    with open(COOLDOWN_FILE, 'w') as f:
        json.dump(data, f)

def check_candle_pattern(open_p, close, high, low):
    c_open, c_close = open_p.iloc[-1], close.iloc[-1]
    p_open, p_close = open_p.iloc[-2], close.iloc[-2]
    c_high, c_low = high.iloc[-1], low.iloc[-1]
    
    body = abs(c_close - c_open)
    upper_wick = c_high - max(c_close, c_open)
    lower_wick = min(c_close, c_open) - c_low
    
    if (c_close > c_open) and (p_close < p_open) and (c_close > p_open) and (c_open < p_close):
        return "Bullish Engulfing", "BUY"
    if (c_close < c_open) and (p_close > p_open) and (c_close < p_open) and (c_open > p_close):
        return "Bearish Engulfing", "SELL"
    if lower_wick > 2 * body and upper_wick < body:
        return "Hammer", "BUY"
    if upper_wick > 2 * body and lower_wick < body:
        return "Shooting Star", "SELL"
    return "None", None

def analyze_market():
    cooldown_data = load_cooldown()
    now = datetime.datetime.now()
    
    # BTC Regime
    btc = yf.download("BTC-USD", period="1mo", interval="1h", progress=False).dropna()
    btc_ema9 = btc["Close"].ewm(span=9).mean().iloc[-1]
    btc_ema21 = btc["Close"].ewm(span=21).mean().iloc[-1]
    btc_bearish = btc_ema9 < btc_ema21 
    
    assets = [
        ("BTC-USD", "BITCOIN (BTC)"), ("ETH-USD", "ETHEREUM (ETH)"), 
        ("SOL-USD", "SOLANA (SOL)"), ("ADA-USD", "CARDANO (ADA)"), 
        ("XRP-USD", "RIPPLE (XRP)"), ("BNB-USD", "BINANCE COIN (BNB)"), 
        ("DOGE-USD", "DOGECOIN (DOGE)"), ("AVAX-USD", "AVALANCHE (AVAX)"), 
        ("DOT-USD", "POLKADOT (DOT)"), ("LINK-USD", "CHAINLINK (LINK)"), 
        ("LTC-USD", "LITECOIN (LTC)")
    ]

    # List to store all valid signals before sending
    valid_signals = []

    for ticker, name in assets:
        if ticker in cooldown_data:
            last_signal = datetime.datetime.strptime(cooldown_data[ticker], "%Y-%m-%d %H:%M:%S")
            if (now - last_signal).total_seconds() < COOLDOWN_HOURS * 3600:
                continue

        try:
            d = yf.download(ticker, period="1mo", interval="1h", progress=False).dropna()
            if len(d) < 100: continue

            close = d["Close"].squeeze()
            open_p = d["Open"].squeeze()
            high = d["High"].squeeze()
            low = d["Low"].squeeze()
            volume = d["Volume"].squeeze()

            ema9 = close.ewm(span=9).mean().iloc[-1]
            ema21 = close.ewm(span=21).mean().iloc[-1]
            ema200 = close.ewm(span=200).mean().iloc[-1]
            
            delta = close.diff()
            gain = delta.clip(lower=0).rolling(14).mean().iloc[-1]
            loss = -delta.clip(upper=0).rolling(14).mean().iloc[-1]
            rsi = 100 - (100 / (1 + gain/loss)) if loss != 0 else 100
            
            macd_line = close.ewm(span=12).mean() - close.ewm(span=26).mean()
            signal_line = macd_line.ewm(span=9).mean()
            macd_bullish = macd_line.iloc[-1] > signal_line.iloc[-1]
            
            avg_vol = volume.rolling(20).mean().iloc[-1]
            rvol = volume.iloc[-1] / avg_vol if avg_vol != 0 else 1
            obv = (close.diff().apply(lambda x: 1 if x > 0 else (-1 if x < 0 else 0)) * volume).cumsum()
            obv_up = obv.iloc[-1] > obv.iloc[-5]
            
            price = round(float(close.iloc[-1]), 4)
            recent_high = high.tail(30).max()
            recent_low = low.tail(30).min()
            near_resistance = price >= (recent_high * 0.99)
            near_support = price <= (recent_low * 1.01)
            pattern, pattern_sig = check_candle_pattern(open_p, close, high, low)

            buy_score = 0
            sell_score = 0

            if ema9 > ema21 and price > ema200: buy_score += 30
            if ema9 < ema21 and price < ema200: sell_score += 30
            
            if 40 <= rsi <= 65 and macd_bullish: buy_score += 25
            if 35 <= rsi <= 60 and not macd_bullish: sell_score += 25
            
            if rvol >= 1.0: 
                if obv_up: buy_score += 15
                if not obv_up: sell_score += 15
            
            if near_support and not near_resistance: buy_score += 15
            if near_resistance and not near_support: sell_score += 15

            if pattern_sig == "BUY": buy_score += 15
            if pattern_sig == "SELL": sell_score += 15

            if buy_score >= MIN_SCORE:
                if not (btc_bearish and ticker != "BTC-USD"):
                    valid_signals.append({
                        "type": "BUY", "ticker": ticker, "name": name, "score": buy_score,
                        "price": price, "rsi": rsi, "rvol": rvol, "pattern": pattern,
                        "sl": round(price - (price * 0.015), 4), "tp": round(price + (price * 0.03), 4)
                    })

            elif sell_score >= MIN_SCORE:
                valid_signals.append({
                    "type": "SELL", "ticker": ticker, "name": name, "score": sell_score,
                    "price": price, "rsi": rsi, "rvol": rvol, "pattern": pattern,
                    "sl": round(price + (price * 0.015), 4), "tp": round(price - (price * 0.03), 4)
                })

            time.sleep(0.5) # API limit

        except Exception as e:
            pass

    # Sort signals by highest score and send TOP 2 ONLY
    valid_signals.sort(key=lambda x: x['score'], reverse=True)
    top_signals = valid_signals[:MAX_SIGNALS_PER_RUN]

    if not top_signals:
        print("No high-quality signals found in this run.")
        return

    for sig in top_signals:
        emoji = "🟢" if sig['type'] == "BUY" else "🔴"
        qty = round((50 * 10) / sig['price'], 2)
        
        msg = f"""{emoji} <b>{sig['type']} SIGNAL ({'LONG' if sig['type']=='BUY' else 'SHORT'})</b> {emoji}
━━━━━━━━━━━━━━━━━━━━
🪙 <b>Asset:</b> {sig['name']}
⏱ <b>Type:</b> INTRADAY (1h)

💰 <b>Price:</b> {sig['price']}
🛑 <b>SL:</b> {sig['sl']}
🎯 <b>TP:</b> {sig['tp']}
📊 <b>Score:</b> {sig['score']}/100 🏆

🚀 <b>Qty:</b> {qty} (Margin: $50)
📈 RSI: {round(float(sig['rsi']),1)} | Vol: {round(float(sig['rvol']),1)}x | {sig['pattern']}"""
        
        send_telegram_alert(msg)
        cooldown_data[sig['ticker']] = now.strftime("%Y-%m-%d %H:%M:%S")
        save_cooldown(cooldown_data)

if __name__ == "__main__":
    analyze_market()
