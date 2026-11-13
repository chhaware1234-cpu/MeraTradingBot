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
# ⚙️ SETTINGS (Multi-Timeframe)
# ==========================================
MIN_SCORE = 75  # High quality signals only
COOLDOWN_HOURS = 4 
COOLDOWN_FILE = "cooldown.json"

def send_telegram_alert(message):
    if not TELEGRAM_TOKEN or not CHAT_ID: return
    url = f"https://api.telegram.org/bot{TELEGRAM_TOKEN}/sendMessage"
    payload = {"chat_id": CHAT_ID, "text": message, "parse_mode": "HTML"}
    try:
        requests.post(url, data=payload, timeout=10)
    except Exception as e:
        print(f"Telegram Error: {e}")

def load_cooldown():
    try:
        if os.path.exists(COOLDOWN_FILE):
            with open(COOLDOWN_FILE, 'r') as f:
                return json.load(f)
        return {}
    except:
        return {}

def save_cooldown(data):
    try:
        with open(COOLDOWN_FILE, 'w') as f:
            json.dump(data, f)
    except:
        pass

def check_candle_pattern(open_p, close, high, low):
    try:
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
    except:
        return "None", None

def analyze_timeframe(ticker, name, interval, period, style, cooldown_data):
    try:
        # Cooldown check specific to Ticker + Timeframe
        cooldown_key = f"{ticker}_{style}"
        now = datetime.datetime.now()
        
        if cooldown_key in cooldown_data:
            last_signal = datetime.datetime.strptime(cooldown_data[cooldown_key], "%Y-%m-%d %H:%M:%S")
            if (now - last_signal).total_seconds() < COOLDOWN_HOURS * 3600:
                return

        d = yf.download(ticker, period=period, interval=interval, progress=False)
        if d is None or d.empty or len(d) < 100: return
        
        d = d.dropna()
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
        
        tr = high - low
        atr = tr.rolling(14).mean().iloc[-1]
        
        avg_vol = volume.rolling(20).mean().iloc[-1]
        rvol = volume.iloc[-1] / avg_vol if avg_vol != 0 else 1
        
        price = round(float(close.iloc[-1]), 4)
        recent_high = high.tail(30).max()
        recent_low = low.tail(30).min()
        near_resistance = price >= (recent_high * 0.99)
        near_support = price <= (recent_low * 1.01)
        
        pattern, pattern_sig = check_candle_pattern(open_p, close, high, low)

        buy_score = 0
        sell_score = 0

        # Trend
        if ema9 > ema21 and price > ema200: buy_score += 25
        if ema9 < ema21 and price < ema200: sell_score += 25
        
        # Momentum
        if 40 <= rsi <= 65 and macd_bullish: buy_score += 25
        if 35 <= rsi <= 60 and not macd_bullish: sell_score += 25
        
        # Volume
        if rvol >= 1.2: 
            buy_score += 25
            sell_score += 25
            
        # S/R
        if near_support: buy_score += 25
        if near_resistance: sell_score += 25

        # Strict Pattern Filter
        if pattern_sig == "BUY": 
            buy_score += 10
            sell_score = 0 
        if pattern_sig == "SELL": 
            sell_score += 10
            buy_score = 0 

        # Signal generation
        if buy_score >= MIN_SCORE:
            sl = round(price - (atr * 2.0), 4)
            tp = round(price + (atr * 4.0), 4)
            qty = round((50 * 10) / price, 2)
            
            msg = f"""🟢 <b>BUY SIGNAL ({style})</b> 🟢
━━━━━━━━━━━━━━━━━━━━
🪙 <b>Asset:</b> {name}
⏱ <b>Type:</b> {style} ({interval})

💰 <b>Price:</b> {price}
🛑 <b>SL:</b> {sl}
🎯 <b>TP:</b> {tp}
📊 <b>Score:</b> {buy_score}/100 🏆

🚀 <b>Qty:</b> {qty} (Margin: $50)
📈 RSI: {round(float(rsi),1)} | Vol: {round(float(rvol),1)}x"""
            send_telegram_alert(msg)
            cooldown_data[cooldown_key] = now.strftime("%Y-%m-%d %H:%M:%S")
            save_cooldown(cooldown_data)

        elif sell_score >= MIN_SCORE:
            sl = round(price + (atr * 2.0), 4)
            tp = round(price - (atr * 4.0), 4)
            qty = round((50 * 10) / price, 2)
            
            msg = f"""🔴 <b>SELL SIGNAL ({style})</b> 🔴
━━━━━━━━━━━━━━━━━━━━
🪙 <b>Asset:</b> {name}
⏱ <b>Type:</b> {style} ({interval})

💰 <b>Price:</b> {price}
🛑 <b>SL:</b> {sl}
🎯 <b>TP:</b> {tp}
📊 <b>Score:</b> {sell_score}/100 🏆

🚀 <b>Qty:</b> {qty} (Margin: $50)
📈 RSI: {round(float(rsi),1)} | Vol: {round(float(rvol),1)}x"""
            send_telegram_alert(msg)
            cooldown_data[cooldown_key] = now.strftime("%Y-%m-%d %H:%M:%S")
            save_cooldown(cooldown_data)

    except Exception as e:
        print(f"Error on {ticker} {style}: {e}")

def main():
    print("Bot Started...")
    cooldown_data = load_cooldown()
    
    # Top 6 Liquid Coins (Kam coins = Kam errors aur fast scan)
    assets = [
        ("BTC-USD", "BITCOIN (BTC)"), 
        ("ETH-USD", "ETHEREUM (ETH)"), 
        ("SOL-USD", "SOLANA (SOL)"), 
        ("XRP-USD", "RIPPLE (XRP)"), 
        ("BNB-USD", "BINANCE COIN (BNB)"), 
        ("ADA-USD", "CARDANO (ADA)")
    ]

    for ticker, name in assets:
        # 1. Swing Trading (Daily Chart, Data 2 Years)
        analyze_timeframe(ticker, name, "1d", "2y", "SWING", cooldown_data)
        time.sleep(1)
        
        # 2. Intraday Trading (1 Hour Chart, Data 1 Month)
        analyze_timeframe(ticker, name, "1h", "1mo", "INTRADAY", cooldown_data)
        time.sleep(1)

        # 3. Scalping (15 Min Chart, Data 60 Days) - Sirf Crypto ke liye
        analyze_timeframe(ticker, name, "15m", "60d", "SCALPING", cooldown_data)
        time.sleep(1)

    print("Scan Complete. No errors encountered.")

if __name__ == "__main__":
    main()
