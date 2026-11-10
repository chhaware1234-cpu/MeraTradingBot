import yfinance as yf
import requests
import os
import warnings
import datetime
import json
import pandas as pd
warnings.filterwarnings('ignore')

TELEGRAM_TOKEN = os.environ.get('TELEGRAM_TOKEN')
CHAT_ID = os.environ.get('CHAT_ID')

# ==========================================
# ⚙️ SETTINGS
# ==========================================
MIN_SCORE = 80
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
    
    # 1. Bullish Engulfing
    if (c_close > c_open) and (p_close < p_open) and (c_close > p_open) and (c_open < p_close):
        return "Bullish Engulfing", "BUY"
    # 2. Bearish Engulfing
    if (c_close < c_open) and (p_close > p_open) and (c_close < p_open) and (c_open > p_close):
        return "Bearish Engulfing", "SELL"
    # 3. Hammer (Bullish)
    if lower_wick > 2 * body and upper_wick < body:
        return "Hammer", "BUY"
    # 4. Shooting Star (Bearish)
    if upper_wick > 2 * body and lower_wick < body:
        return "Shooting Star", "SELL"
    
    return "None", None

def analyze_market():
    cooldown_data = load_cooldown()
    now = datetime.datetime.now()
    
    # 🚨 1. BTC MARKET REGIME CHECK
    btc = yf.download("BTC-USD", period="1mo", interval="1h", progress=False).dropna()
    btc_ema9 = btc["Close"].ewm(span=9).mean().iloc[-1]
    btc_ema21 = btc["Close"].ewm(span=21).mean().iloc[-1]
    btc_bearish = btc_ema9 < btc_ema21 # Agar BTC bearish hai, toh alts mein BUY block
    
    assets = [
        ("BTC-USD", "BITCOIN (BTC)"), ("ETH-USD", "ETHEREUM (ETH)"), 
        ("SOL-USD", "SOLANA (SOL)"), ("ADA-USD", "CARDANO (ADA)"), 
        ("XRP-USD", "RIPPLE (XRP)")
    ]

    for ticker, name in assets:
        # Cooldown Check (4 hours)
        if ticker in cooldown_data:
            last_signal_time = datetime.datetime.strptime(cooldown_data[ticker], "%Y-%m-%d %H:%M:%S")
            if (now - last_signal_time).total_seconds() < 4 * 3600:
                continue # Skip if within 4 hours

        try:
            # Fetch 1H Data
            d = yf.download(ticker, period="1mo", interval="1h", progress=False).dropna()
            if len(d) < 100: continue

            close = d["Close"].squeeze()
            open_p = d["Open"].squeeze()
            high = d["High"].squeeze()
            low = d["Low"].squeeze()
            volume = d["Volume"].squeeze()

            # Indicators
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
            
            # RVOL & OBV
            avg_vol = volume.rolling(20).mean().iloc[-1]
            rvol = volume.iloc[-1] / avg_vol if avg_vol != 0 else 1
            
            obv = (close.diff().apply(lambda x: 1 if x > 0 else (-1 if x < 0 else 0)) * volume).cumsum()
            obv_trend_up = obv.iloc[-1] > obv.iloc[-5]
            
            price = round(float(close.iloc[-1]), 4)
            
            # S/R (Support & Resistance)
            recent_high = high.tail(30).max()
            recent_low = low.tail(30).min()
            near_resistance = price >= (recent_high * 0.99)
            near_support = price <= (recent_low * 1.01)

            # Candle Pattern
            pattern_name, pattern_signal = check_candle_pattern(open_p, close, high, low)

            # ==========================================
            # 🧮 SCORING SYSTEM
            # ==========================================
            buy_score = 0
            sell_score = 0

            # 1. Trend (30 Points)
            if ema9 > ema21 and price > ema200: buy_score += 30
            if ema9 < ema21 and price < ema200: sell_score += 30
            
            # 2. Momentum (25 Points)
            if 40 <= rsi <= 65 and macd_bullish: buy_score += 25
            if 35 <= rsi <= 60 and not macd_bullish: sell_score += 25
            
            # 3. Volume & Pump Filter (15 Points)
            if rvol > 1.2: # Volume should be above average
                if obv_trend_up: buy_score += 15
                if not obv_trend_up: sell_score += 15
            
            # 4. S/R Confirmation (15 Points)
            if near_support and not near_resistance: buy_score += 15
            if near_resistance and not near_support: sell_score += 15

            # 5. Candle Confirmation (15 Points)
            if pattern_signal == "BUY": buy_score += 15
            if pattern_signal == "SELL": sell_score += 15

            # ==========================================
            # 🚀 SIGNAL GENERATION
            # ==========================================
            # BUY SIGNAL
            if buy_score >= MIN_SCORE:
                if btc_bearish and ticker != "BTC-USD":
                    continue # Block Alt BUY if BTC is bearish
                
                sl = round(price - (price * 0.015), 4) # 1.5% SL
                tp = round(price + (price * 0.03), 4)  # 3% TP
                qty = round((50 * 10) / price, 2)
                
                msg = f"""🟢 <b>BUY SIGNAL (LONG)</b> 🟢
━━━━━━━━━━━━━━━━━━━━
🪙 <b>Asset:</b> {name}
⏱ <b>Type:</b> INTRADAY (1h)

💰 <b>Current Price:</b> {price}
🛑 <b>Stop Loss:</b> {sl}
🎯 <b>Take Profit:</b> {tp}

📊 <b>Score:</b> {buy_score}/100 🏆
🚀 <b>FUTURES (Long):</b> Qty {qty} (Margin: $50)

📈 <b>Technicals</b>
Trend: UP | RSI: {round(float(rsi),1)} | Vol: {round(float(rvol),1)}x
Candle: {pattern_name} 🕯️"""
                send_telegram_alert(msg)
                cooldown_data[ticker] = now.strftime("%Y-%m-%d %H:%M:%S")
                save_cooldown(cooldown_data)

            # SELL SIGNAL
            elif sell_score >= MIN_SCORE:
                sl = round(price + (price * 0.015), 4) 
                tp = round(price - (price * 0.03), 4)
                qty = round((50 * 10) / price, 2)
                
                msg = f"""🔴 <b>SELL SIGNAL (SHORT)</b> 🔴
━━━━━━━━━━━━━━━━━━━━
🪙 <b>Asset:</b> {name}
⏱ <b>Type:</b> INTRADAY (1h)

💰 <b>Current Price:</b> {price}
🛑 <b>Stop Loss:</b> {sl}
🎯 <b>Take Profit:</b> {tp}

📊 <b>Score:</b> {sell_score}/100 🏆
🚀 <b>FUTURES (Short):</b> Qty {qty} (Margin: $50)

📈 <b>Technicals</b>
Trend: DOWN | RSI: {round(float(rsi),1)} | Vol: {round(float(rvol),1)}x
Candle: {pattern_name} 🕯️"""
                send_telegram_alert(msg)
                cooldown_data[ticker] = now.strftime("%Y-%m-%d %H:%M:%S")
                save_cooldown(cooldown_data)

        except Exception as e:
            pass

if __name__ == "__main__":
    analyze_market()
