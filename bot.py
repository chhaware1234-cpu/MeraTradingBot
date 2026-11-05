import yfinance as yf
import requests
import os
import warnings
warnings.filterwarnings('ignore')

# GitHub Secrets se Token aur Chat ID uthayega
TELEGRAM_TOKEN = os.environ.get('TELEGRAM_TOKEN')
CHAT_ID = os.environ.get('CHAT_ID')

# =========================================================
# 💰 RISK MANAGER SETTING
# Aap ek trade mein maximum kitna loss afford kar sakte hain?
# (Jaise ₹1000. Agar SL hit ho, toh sirf ₹1000 ka loss hoga)
# =========================================================
RISK_PER_TRADE = 1000 

def send_telegram_alert(message):
    if not TELEGRAM_TOKEN or not CHAT_ID:
        print("Secrets missing!")
        return
    url = f"https://api.telegram.org/bot{TELEGRAM_TOKEN}/sendMessage"
    payload = {"chat_id": CHAT_ID, "text": message}
    try:
        requests.post(url, data=payload)
    except Exception as e:
        pass

def scan_market(ticker, name, interval, period, style):
    try:
        d = yf.download(ticker, period=period, interval=interval, progress=False).dropna()
        if len(d) < 200: return 

        close = d["Close"].squeeze()
        open_p = d["Open"].squeeze()
        high = d["High"].squeeze()
        low = d["Low"].squeeze()
        volume = d["Volume"].squeeze()
        
        # 1. Trend Indicators
        ema9 = close.ewm(span=9).mean().iloc[-1]
        ema21 = close.ewm(span=21).mean().iloc[-1]
        ema200 = close.ewm(span=200).mean().iloc[-1] 
        
        # 2. RSI (Overbought/Oversold)
        delta = close.diff()
        gain = delta.clip(lower=0).rolling(14).mean().iloc[-1]
        loss = -delta.clip(upper=0).rolling(14).mean().iloc[-1]
        rsi = 100 - (100 / (1 + gain/loss)) if loss != 0 else 100
        
        # 3. MACD (Momentum)
        macd_line = close.ewm(span=12).mean() - close.ewm(span=26).mean()
        signal_line = macd_line.ewm(span=9).mean()
        macd_bullish = macd_line.iloc[-1] > signal_line.iloc[-1]
        
        # 4. ATR (Stop Loss / Target)
        tr = high - low
        atr = tr.rolling(14).mean().iloc[-1]
        
        price = round(float(close.iloc[-1]), 2)
        rsi = round(float(rsi), 1)

        # 5. Volume (Fake Pump/Dump)
        current_vol = volume.iloc[-1]
        avg_vol = volume.rolling(20).mean().iloc[-1]
        vol_ratio = current_vol / avg_vol if avg_vol != 0 else 1
        
        # 6. Candlestick Patterns
        c_open, c_close = open_p.iloc[-1], close.iloc[-1]
        p_open, p_close = open_p.iloc[-2], close.iloc[-2]
        bull_engulf = (c_close > c_open) and (p_close < p_open) and (c_close > p_open) and (c_open < p_close)
        bear_engulf = (c_close < c_open) and (p_close > p_open) and (c_close < p_open) and (c_open > p_close)
        candle_msg = "🕯️ Pattern" if (bull_engulf or bear_engulf) else ""

        # --- BUY LOGIC ---
        if ema9 > ema21 and price > ema200 and macd_bullish and rsi < 70 and vol_ratio >= 0.5:
            strength = "STRONG 🔥" if (bull_engulf and vol_ratio > 1.5) else "GOOD 👍"
            sl = round(price - (atr * 1.5), 2)
            tp = round(price + (atr * 3.0), 2)
            
            # Risk Manager (Position Sizing)
            sl_dist = abs(price - sl)
            qty = round(RISK_PER_TRADE / sl_dist, 2) if sl_dist > 0 else 0
            
            msg = f"🟢 BUY SIGNAL 🟢\nAsset: {name} ({style})\nPrice: {price}\nQty: {qty} (Risk: ₹{RISK_PER_TRADE})\nStrength: {strength} {candle_msg}\n🛑 SL: {sl} | 🎯 TP: {tp}"
            send_telegram_alert(msg) 
            
        # --- SELL LOGIC ---
        elif ema9 < ema21 and price < ema200 and not macd_bullish and rsi > 30 and vol_ratio >= 0.5:
            strength = "STRONG 🔥" if (bear_engulf and vol_ratio > 1.5) else "GOOD 👍"
            sl = round(price + (atr * 1.5), 2)
            tp = round(price - (atr * 3.0), 2)
            
            # Risk Manager (Position Sizing)
            sl_dist = abs(sl - price)
            qty = round(RISK_PER_TRADE / sl_dist, 2) if sl_dist > 0 else 0
            
            msg = f"🔴 SELL SIGNAL 🔴\nAsset: {name} ({style})\nPrice: {price}\nQty: {qty} (Risk: ₹{RISK_PER_TRADE})\nStrength: {strength} {candle_msg}\n🛑 SL: {sl} | 🎯 TP: {tp}"
            send_telegram_alert(msg) 
                
    except Exception as e:
        pass 

def main():
    # Assets List (Crypto, Gold, Indian Stocks)
    assets = [
        ("BTC-USD", "BITCOIN"), ("ETH-USD", "ETHEREUM"), ("SOL-USD", "SOLANA"), 
        ("GC=F", "GOLD"),
        ("RELIANCE.NS", "RELIANCE"), ("TATAMOTORS.NS", "TATA MOTORS"), 
        ("HDFCBANK.NS", "HDFC BANK"), ("^NSEI", "NIFTY 50"), ("INFY.NS", "INFOSYS")
    ]

    # Scan all assets in different timeframes
    for t, n in assets: scan_market(t, n, "1d", "2y", "SWING")
    for t, n in assets: scan_market(t, n, "1h", "1y", "INTRADAY")
    for t, n in assets: scan_market(t, n, "15m", "60d", "SCALPING")

if __name__ == "__main__":
    main()
